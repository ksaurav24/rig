use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};
#[cfg(not(test))]
use std::sync::LazyLock;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use rig_api::{
    Access, DashboardAccessRequest, DashboardDomainRequest, ErrorBody, NetworkInfo, ServerMeta,
    SystemInfo, TailscaleInfo, TlsMode,
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    access,
    auth::{self, Auth, Owner, Viewer, real_ip},
    error::{ApiError, ApiResult},
    metrics::cached_system_snapshot,
    proxy,
    state::{AppState, NetworkSnapshot},
};

const NETWORK_CACHE_TTL: Duration = Duration::from_secs(30);
pub const API_VERSION: u32 = 1;

#[cfg(not(test))]
static PUBLIC_IP_HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("rig/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static public IP client settings are valid")
});

const CAPABILITIES: &[&str] = &[
    "events_sse",
    "device_pairing",
    "local_deploy",
    "terminal_ws",
    "env_bulk",
];

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(server_meta))
        .routes(routes!(system_info))
        .routes(routes!(network_info))
        .routes(routes!(set_dashboard_access))
        .routes(routes!(set_dashboard_domain))
}

/// Server version and capability negotiation. Returns minimal info without auth;
/// includes `mode` and `access` when authenticated.
#[utoipa::path(get, path = "/api/v1/meta", tag = "system",
    responses((status = 200, body = ServerMeta)))]
async fn server_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<ServerMeta> {
    let authed = auth::is_authenticated_request(&state, &headers).await;
    Json(ServerMeta {
        server_version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION,
        mode: authed.then_some(state.config.mode),
        access: authed.then(|| state.access.get()),
        capabilities: CAPABILITIES.iter().map(|s| (*s).to_owned()).collect(),
    })
}

/// Health of the server Rig runs on: engine, proxy and resource usage.
#[utoipa::path(get, path = "/api/v1/system", tag = "system",
    responses((status = 200, body = SystemInfo)))]
async fn system_info(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<SystemInfo>> {
    let counts = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE(SUM(status = 'running'), 0) FROM apps WHERE org_id = ?",
    )
    .bind(&auth.org_id)
    .fetch_one(&state.db);
    let (snap, counts) = tokio::join!(cached_system_snapshot(&state), counts);
    let snap = snap?;
    let (apps_total, apps_running) = counts?;

    Ok(Json(SystemInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        engine: snap.engine.as_ref().map(|e| e.name.clone()),
        engine_version: snap.engine.map(|e| e.version),
        proxy_ready: snap.proxy_ready,
        cpu_percent: snap.cpu_percent,
        memory_used: snap.memory_used,
        memory_total: snap.memory_total,
        disk_used: snap.disk_used,
        disk_total: snap.disk_total,
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

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    let is_cgnat = a == 100 && (64..=127).contains(&b);
    !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_broadcast()
        && !ip.is_documentation()
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !is_cgnat
}

pub(crate) fn local_public_ipv4() -> Option<String> {
    let Ok(nets) = sysinfo::Networks::new_with_refreshed_list() else {
        return None;
    };
    nets.values()
        .flat_map(|n| n.ip_networks().iter().map(|i| i.addr))
        .find_map(|ip| match ip {
            IpAddr::V4(v4) if is_public_ipv4(v4) => Some(v4.to_string()),
            _ => None,
        })
}

async fn detect_public_ip() -> Option<String> {
    if let Ok(Some(local)) = tokio::task::spawn_blocking(local_public_ipv4).await {
        return Some(local);
    }
    #[cfg(test)]
    {
        return None;
    }
    #[cfg(not(test))]
    {
        for url in ["https://api.ipify.org", "https://ifconfig.me/ip"] {
            if let Ok(resp) = PUBLIC_IP_HTTP.get(url).send().await
                && resp.status().is_success()
                && let Ok(text) = resp.text().await
                && let Ok(ip) = text.trim().parse::<IpAddr>()
            {
                return Some(ip.to_string());
            }
        }
        None
    }
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

/// Where this machine can be reached (public IP, LAN addresses, Tailscale name)
/// and who may open the dashboard.
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

async fn cached_network_probe(
    state: &AppState,
) -> (Option<String>, Vec<String>, Option<TailscaleInfo>) {
    if let Some(snap) = state
        .caches
        .network
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        && snap.sampled_at.elapsed() < NETWORK_CACHE_TTL
    {
        return (
            snap.public_ip.clone(),
            snap.lan_addresses.clone(),
            snap.tailscale.clone(),
        );
    }
    let _guard = state.caches.network_refresh.lock().await;
    if let Some(snap) = state
        .caches
        .network
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        && snap.sampled_at.elapsed() < NETWORK_CACHE_TTL
    {
        return (
            snap.public_ip.clone(),
            snap.lan_addresses.clone(),
            snap.tailscale.clone(),
        );
    }
    let lan = tokio::task::spawn_blocking(lan_addresses);
    let (public_ip, lan, tailscale) = tokio::join!(detect_public_ip(), lan, tailscale_info());
    let lan_addresses = lan.unwrap_or_default();
    let snap = NetworkSnapshot {
        public_ip: public_ip.clone(),
        lan_addresses: lan_addresses.clone(),
        tailscale: tailscale.clone(),
        sampled_at: Instant::now(),
    };
    *state
        .caches
        .network
        .write()
        .unwrap_or_else(|p| p.into_inner()) = Some(snap);
    (public_ip, lan_addresses, tailscale)
}

async fn network(state: &AppState, you: IpAddr) -> NetworkInfo {
    let ((public_ip, lan_addresses, tailscale), dashboard_domain) =
        tokio::join!(cached_network_probe(state), proxy::dashboard_domain(&state.db));
    NetworkInfo {
        public_ip,
        lan_addresses,
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
    auth.require_session("sign in to the dashboard to change network access settings")?;
    let you = if req.access == Access::Local {
        addr.ip()
    } else {
        real_ip(&headers, addr.ip())
    };
    if !access::allows(req.access, you) {
        return Err(ApiError::validation(match req.access {
            Access::Tailscale => {
                "you are not connected over Tailscale, so this would lock you out; open the dashboard on its tailnet address first"
            }
            Access::Local => {
                "you are not connected over loopback (127.0.0.1 / ::1), so local mode would lock you out"
            }
            _ => "you are not on the local network, so this would lock you out",
        }));
    }
    let prev = state.access.get();
    access::store(&state.db, req.access).await?;
    state.access.set(req.access);
    if prev == Access::Local || req.access == Access::Local {
        if let Err(e) = proxy::sync(&state).await {
            tracing::warn!("proxy not updated after switching local access mode: {e:#}");
        }
    }
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
    auth.require_session("sign in to the dashboard to change the dashboard domain")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ipv4_filter_excludes_private_loopback_and_cgnat() {
        assert!(is_public_ipv4(Ipv4Addr::new(203, 0, 113, 10)) == false); // TEST-NET-3 documentation
        assert!(is_public_ipv4(Ipv4Addr::new(1, 1, 1, 1)));
        assert!(is_public_ipv4(Ipv4Addr::new(45, 79, 12, 34)));
        assert!(!is_public_ipv4(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(!is_public_ipv4(Ipv4Addr::new(172, 17, 0, 1)));
        assert!(!is_public_ipv4(Ipv4Addr::new(192, 168, 1, 10)));
        assert!(!is_public_ipv4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!is_public_ipv4(Ipv4Addr::new(100, 64, 0, 5)));
        assert!(!is_public_ipv4(Ipv4Addr::new(169, 254, 1, 1)));
    }
}
