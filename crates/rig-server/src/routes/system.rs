use std::time::Duration;

use std::net::{IpAddr, SocketAddr};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use rig_api::{
    Access, DashboardAccessRequest, DashboardDomainRequest, ErrorBody, NetworkInfo, SystemInfo,
    TailscaleInfo, TlsMode,
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    access, proxy,
    auth::{Auth, Owner, Viewer, real_ip},
    error::{ApiError, ApiResult},
    metrics::sample_host,
    state::AppState,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(system_info))
        .routes(routes!(network_info))
        .routes(routes!(set_dashboard_access))
        .routes(routes!(set_dashboard_domain))
}

/// Health of the server Rig runs on: engine, proxy and resource usage.
#[utoipa::path(get, path = "/api/v1/system", tag = "system",
    responses((status = 200, body = SystemInfo)))]
async fn system_info(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<SystemInfo>> {
    let data_dir = state.config.data_dir.clone();
    let usage = tokio::task::spawn_blocking(move || sample_host(&data_dir));
    let engine = tokio::time::timeout(Duration::from_secs(3), state.runtime.info());
    let counts = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE(SUM(status = 'running'), 0) FROM apps WHERE org_id = ?",
    )
    .bind(&auth.org_id)
    .fetch_one(&state.db);

    let (usage, engine, proxy_ready, counts) =
        tokio::join!(usage, engine, state.proxy.ready(), counts);
    let usage = usage.map_err(|e| anyhow::anyhow!("sampling host usage: {e}"))??;
    let engine = engine.ok().and_then(Result::ok);
    let (apps_total, apps_running) = counts?;

    Ok(Json(SystemInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        engine: engine.as_ref().map(|e| e.name.clone()),
        engine_version: engine.map(|e| e.version),
        proxy_ready,
        cpu_percent: usage.cpu_percent,
        memory_used: usage.memory_used,
        memory_total: usage.memory_total,
        disk_used: usage.disk_used,
        disk_total: usage.disk_total,
        apps_running: u32::try_from(apps_running).unwrap_or(0),
        apps_total: u32::try_from(apps_total).unwrap_or(0),
    }))
}

/// The machine's MagicDNS name and addresses, if Tailscale runs here.
pub(crate) async fn tailscale_info() -> Option<TailscaleInfo> {
    let out = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new("tailscale")
            .args(["status", "--json", "--peers=false"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    if v["BackendState"] != "Running" {
        return None;
    }
    let me = &v["Self"];
    Some(TailscaleInfo {
        dns_name: me["DNSName"]
            .as_str()
            .map(|s| s.trim_end_matches('.').to_owned())
            .filter(|s| !s.is_empty()),
        addresses: me["TailscaleIPs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

pub(crate) fn lan_addresses() -> Vec<String> {
    let Ok(nets) = sysinfo::Networks::new_with_refreshed_list() else {
        return Vec::new();
    };
    let mut out: Vec<String> = nets
        .values()
        .flat_map(|n| n.ip_networks().iter().map(|i| i.addr))
        .filter(|ip| match ip {
            IpAddr::V4(v4) => v4.is_private(),
            IpAddr::V6(_) => false,
        })
        .map(|ip| ip.to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Where this machine can be reached (LAN addresses, Tailscale name) and who
/// may open the dashboard.
#[utoipa::path(get, path = "/api/v1/system/network", tag = "system",
    responses((status = 200, body = NetworkInfo)))]
async fn network_info(
    State(state): State<AppState>,
    _auth: Auth<Viewer>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> ApiResult<Json<NetworkInfo>> {
    Ok(Json(network(&state, real_ip(&headers, addr.ip())).await))
}

async fn network(state: &AppState, you: IpAddr) -> NetworkInfo {
    let lan = tokio::task::spawn_blocking(lan_addresses);
    let (lan, tailscale, dashboard_domain) =
        tokio::join!(lan, tailscale_info(), proxy::dashboard_domain(&state.db));
    NetworkInfo {
        lan_addresses: lan.unwrap_or_default(),
        tailscale,
        dashboard_access: state.access.get(),
        your_address: Some(you.to_string()),
        dashboard_domain: dashboard_domain.ok().flatten(),
    }
}

/// Limit who may open the dashboard and API to the local network or the
/// tailnet. Deploy webhooks stay reachable. Refused if it would lock out the
/// address you are using now. On the server,
/// `rig-server dashboard-access public` undoes it.
#[utoipa::path(put, path = "/api/v1/system/dashboard-access", tag = "system",
    request_body = DashboardAccessRequest,
    responses((status = 200, body = NetworkInfo), (status = 422, body = ErrorBody)))]
async fn set_dashboard_access(
    State(state): State<AppState>,
    auth: Auth<Owner>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<DashboardAccessRequest>,
) -> ApiResult<Json<NetworkInfo>> {
    let you = real_ip(&headers, addr.ip());
    if !access::allows(req.access, you) {
        return Err(ApiError::validation(match req.access {
            Access::Tailscale => {
                "you are not connected over Tailscale, so this would lock you out; open the dashboard on its tailnet address first"
            }
            _ => "you are not on the local network, so this would lock you out",
        }));
    }
    access::store(&state.db, req.access).await?;
    state.access.set(req.access);
    auth.audit(
        &state,
        "system.dashboard_access",
        None,
        Some(req.access.as_str()),
    )
    .await?;
    Ok(Json(network(&state, you).await))
}

/// Serve the dashboard on a domain of its own (with a certificate unless
/// `tls_mode` is `off`), or send `host: null` to serve it on the machine's
/// addresses over plain HTTP again. Point the name's DNS at this server
/// first: the old address stops serving the dashboard. On the server,
/// `rig-server dashboard-domain --clear` undoes it.
#[utoipa::path(put, path = "/api/v1/system/dashboard-domain", tag = "system",
    request_body = DashboardDomainRequest,
    responses((status = 200, body = NetworkInfo), (status = 422, body = ErrorBody)))]
async fn set_dashboard_domain(
    State(state): State<AppState>,
    auth: Auth<Owner>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<DashboardDomainRequest>,
) -> ApiResult<Json<NetworkInfo>> {
    let tls = req.tls_mode.unwrap_or(TlsMode::Auto);
    let host = match req.host.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => {
            let h = super::apps::validate_host(h)?;
            if tls == TlsMode::Auto && h.parse::<IpAddr>().is_ok() {
                return Err(ApiError::validation(
                    "automatic HTTPS requires a domain name, not an IP address; choose Private CA for an IP",
                ));
            }
            let taken: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM domains WHERE host = ? AND port IS NULL")
                    .bind(&h)
                    .fetch_optional(&state.db)
                    .await?;
            if taken.is_some() {
                return Err(ApiError::validation("an app already uses this name"));
            }
            Some(h)
        }
        None => None,
    };
    proxy::store_dashboard_domain(&state.db, host.as_deref(), tls).await?;
    if let Err(e) = proxy::sync(&state).await {
        tracing::warn!("proxy not updated after a dashboard domain change: {e:#}");
    }
    auth.audit(&state, "system.dashboard_domain", None, host.as_deref())
        .await?;
    Ok(Json(network(&state, real_ip(&headers, addr.ip())).await))
}
