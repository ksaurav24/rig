//! The reverse proxy boundary.
//!
//! Rig owns the proxy's whole configuration: every change rebuilds it from
//! the database and loads it in one request, so the proxy can never drift
//! from what the dashboard shows. Caddy applies a load without dropping
//! connections.

use std::time::Duration;

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rig_api::{Access, DashboardDomain, TlsMode};
use serde_json::{Value, json};
use sqlx::Row;

use crate::{access, state::AppState};

/// One hostname routed to one app container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub host: String,
    pub tls: TlsMode,
    /// Loopback port of the app's live container.
    pub upstream_port: u16,
    pub access: Access,
    /// A listener port of its own; None uses the standard ports.
    pub port: Option<u16>,
}

#[async_trait]
pub trait Proxy: Send + Sync {
    /// Whether the proxy answers on its admin endpoint.
    async fn ready(&self) -> bool;

    /// Replace the proxy's routes with exactly `routes`, plus the
    /// dashboard on `dashboard` (or on every address over plain HTTP).
    async fn apply(&self, routes: &[Route], dashboard: Option<&DashboardDomain>) -> Result<()>;
    /// The proxy's own (HTTP, HTTPS) ports, which domains may not claim.
    fn ports(&self) -> (u16, u16);
}

#[derive(Debug, Clone)]
pub struct CaddyOptions {
    /// Admin endpoint, e.g. `http://127.0.0.1:2019`.
    pub admin: url::Url,
    pub http_port: u16,
    pub https_port: u16,
    /// Loopback port Rig itself listens on, so the proxy can serve the
    /// dashboard. None leaves the dashboard off the proxy.
    pub dashboard_port: Option<u16>,
}

pub struct CaddyProxy {
    opts: CaddyOptions,
    http: reqwest::Client,
}

impl CaddyProxy {
    pub fn new(opts: CaddyOptions) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .context("building proxy client")?;
        Ok(Self { opts, http })
    }

    fn endpoint(&self, path: &str) -> Result<url::Url> {
        self.opts
            .admin
            .join(path)
            .context("building proxy admin URL")
    }
}

#[async_trait]
impl Proxy for CaddyProxy {
    fn ports(&self) -> (u16, u16) {
        (self.opts.http_port, self.opts.https_port)
    }

    async fn ready(&self) -> bool {
        let Ok(url) = self.endpoint("/config/") else {
            return false;
        };
        matches!(self.http.get(url).send().await, Ok(r) if r.status().is_success())
    }

    async fn apply(&self, routes: &[Route], dashboard: Option<&DashboardDomain>) -> Result<()> {
        let config = caddy_config(&self.opts, routes, dashboard);
        let res = self
            .http
            .post(self.endpoint("/load")?)
            .json(&config)
            .send()
            .await
            .context("the proxy admin endpoint is not reachable")?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            bail!(
                "the proxy rejected the configuration ({status}): {}",
                body.trim()
            );
        }
        Ok(())
    }
}

/// The complete Caddy configuration for `routes`.
///
/// Each listener port gets one server. HTTPS hosts (`auto`, `internal`,
/// `tailscale`) on the standard ports share `rig_https`; plain hosts (`off`)
/// share `rig_http`; a domain with a port of its own gets `rig_port_<n>`.
/// Caddy adds its own HTTP-to-HTTPS redirects for the standard HTTPS hosts.
///
/// A LAN or tailnet domain matches on the connecting address as well, and
/// anyone else gets a 403 for that name instead of falling through.
///
/// The dashboard is served on its own domain when one is set. Otherwise it
/// answers on the HTTP port for any name no app has claimed, so a fresh
/// server is reachable at `http://<its address>`. Rig enforces its own
/// dashboard access rules, so its route never filters by address.
pub fn caddy_config(
    opts: &CaddyOptions,
    routes: &[Route],
    dashboard: Option<&DashboardDomain>,
) -> Value {
    let mut all: Vec<Route> = routes.to_vec();
    let mut fallback = None;
    if let Some(port) = opts.dashboard_port {
        match dashboard {
            Some(d) => all.push(Route {
                host: d.host.clone(),
                tls: d.tls_mode,
                upstream_port: port,
                access: Access::Public,
                port: None,
            }),
            None => fallback = Some(port),
        }
    }
    let routes = all.as_slice();
    let mut servers: BTreeMap<String, (u16, bool, Vec<Value>)> = BTreeMap::new();
    for r in routes {
        let secure = r.tls != TlsMode::Off;
        let (name, port) = match r.port {
            Some(p) => (format!("rig_port_{p}"), p),
            None if secure => ("rig_https".to_owned(), opts.https_port),
            None => ("rig_http".to_owned(), opts.http_port),
        };
        let entry = servers.entry(name).or_insert((port, secure, Vec::new()));
        let proxy = json!([{
            "handler": "reverse_proxy",
            "upstreams": [{ "dial": format!("127.0.0.1:{}", r.upstream_port) }],
        }]);
        let ranges = access::ranges(r.access);
        if ranges.is_empty() {
            entry.2.push(json!({
                "match": [{ "host": [r.host] }],
                "handle": proxy,
                "terminal": true,
            }));
        } else {
            entry.2.push(json!({
                "match": [{ "host": [r.host], "remote_ip": { "ranges": ranges } }],
                "handle": proxy,
                "terminal": true,
            }));
            entry.2.push(json!({
                "match": [{ "host": [r.host] }],
                "handle": [{
                    "handler": "static_response",
                    "status_code": 403,
                    "body": "This address is not allowed to reach this app.\n",
                    "close": true,
                }],
                "terminal": true,
            }));
        }
    }
    if let Some(port) = fallback {
        // Names served over HTTPS keep Caddy's redirect to HTTPS, so the
        // catch-all leaves them alone.
        let mut secure: Vec<&str> = routes
            .iter()
            .filter(|r| r.port.is_none() && r.tls != TlsMode::Off)
            .map(|r| r.host.as_str())
            .collect();
        secure.sort_unstable();
        secure.dedup();
        let mut route = json!({
            "handle": [{
                "handler": "reverse_proxy",
                "upstreams": [{ "dial": format!("127.0.0.1:{port}") }],
            }],
            "terminal": true,
        });
        if !secure.is_empty() {
            route["match"] = json!([{ "not": [{ "host": secure }] }]);
        }
        servers
            .entry("rig_http".to_owned())
            .or_insert((opts.http_port, false, Vec::new()))
            .2
            .push(route);
    }
    let servers: serde_json::Map<String, Value> = servers
        .into_iter()
        .map(|(name, (port, secure, routes))| {
            let mut server = json!({ "listen": [format!(":{port}")], "routes": routes });
            // A plain server on a port other than the HTTP port would
            // otherwise be given certificates.
            if !secure && port != opts.http_port {
                server["automatic_https"] = json!({ "disable": true });
            }
            (name, server)
        })
        .collect();

    let hosts = |mode: TlsMode| -> Vec<&str> {
        let mut h: Vec<&str> = routes
            .iter()
            .filter(|r| r.tls == mode)
            .map(|r| r.host.as_str())
            .collect();
        h.sort_unstable();
        h.dedup();
        h
    };
    let mut policies = Vec::new();
    let internal = hosts(TlsMode::Internal);
    if !internal.is_empty() {
        policies.push(json!({ "subjects": internal, "issuers": [{ "module": "internal" }] }));
    }
    let tailscale = hosts(TlsMode::Tailscale);
    if !tailscale.is_empty() {
        policies.push(json!({
            "subjects": tailscale,
            "get_certificate": [{ "via": "tailscale" }],
        }));
    }

    let mut apps = serde_json::Map::new();
    apps.insert(
        "http".into(),
        json!({ "http_port": opts.http_port, "https_port": opts.https_port, "servers": servers }),
    );
    if !policies.is_empty() {
        apps.insert(
            "tls".into(),
            json!({ "automation": { "policies": policies } }),
        );
    }

    // Loading a config replaces the admin settings too, so restate the
    // address Rig reaches the proxy on or the next load would lose it.
    let admin_listen = format!(
        "{}:{}",
        opts.admin.host_str().unwrap_or("127.0.0.1"),
        opts.admin.port_or_known_default().unwrap_or(2019)
    );
    json!({ "admin": { "listen": admin_listen }, "apps": apps })
}

/// The dashboard's own hostname, if the owner has set one.
pub async fn dashboard_domain(db: &sqlx::SqlitePool) -> sqlx::Result<Option<DashboardDomain>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM settings WHERE key IN ('dashboard_domain', 'dashboard_tls')",
    )
    .fetch_all(db)
    .await?;
    let get = |k: &str| rows.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    Ok(get("dashboard_domain").map(|host| DashboardDomain {
        host: host.to_owned(),
        tls_mode: parse_tls(get("dashboard_tls").unwrap_or("auto")),
    }))
}

/// Sets (or with None, clears) the dashboard's own hostname.
pub async fn store_dashboard_domain(
    db: &sqlx::SqlitePool,
    host: Option<&str>,
    tls: TlsMode,
) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM settings WHERE key IN ('dashboard_domain', 'dashboard_tls')")
        .execute(&mut *tx)
        .await?;
    if let Some(host) = host {
        let tls = match tls {
            TlsMode::Auto => "auto",
            TlsMode::Internal => "internal",
            TlsMode::Tailscale => "tailscale",
            TlsMode::Off => "off",
        };
        sqlx::query("INSERT INTO settings (key, value) VALUES ('dashboard_domain', ?), ('dashboard_tls', ?)")
            .bind(host)
            .bind(tls)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

pub fn parse_tls(s: &str) -> TlsMode {
    match s {
        "auto" => TlsMode::Auto,
        "internal" => TlsMode::Internal,
        "tailscale" => TlsMode::Tailscale,
        _ => TlsMode::Off,
    }
}

/// Rebuild the proxy's routes from the database and load them.
pub async fn sync(state: &AppState) -> Result<()> {
    let rows = sqlx::query(
        "SELECT d.host, d.tls_mode, d.access, d.port, dep.host_port
           FROM domains d
           JOIN apps a ON a.id = d.app_id
           JOIN deployments dep ON dep.id = a.current_deployment_id
          WHERE a.status = 'running' AND dep.host_port IS NOT NULL
          ORDER BY d.host",
    )
    .fetch_all(&state.db)
    .await
    .context("reading routes")?;
    let routes: Vec<Route> = rows
        .iter()
        .filter_map(|r| {
            let tls = parse_tls(r.get::<String, _>("tls_mode").as_str());
            let upstream_port = u16::try_from(r.get::<i64, _>("host_port")).ok()?;
            let port = match r.get::<Option<i64>, _>("port") {
                Some(p) => Some(u16::try_from(p).ok()?),
                None => None,
            };
            Some(Route {
                host: r.get("host"),
                tls,
                upstream_port,
                access: access::parse(r.get("access")),
                port,
            })
        })
        .collect();
    let dashboard = dashboard_domain(&state.db)
        .await
        .context("reading the dashboard domain")?;
    state.proxy.apply(&routes, dashboard.as_ref()).await
}

/// Keeps the proxy in step with the database: once at startup (Caddy may
/// have restarted with an empty config) and then every minute, which also
/// repairs a proxy restarted while Rig kept running. Caddy skips a load
/// identical to its current config.
pub async fn supervisor(state: AppState) {
    let mut last_ok = None;
    loop {
        let ok = match sync(&state).await {
            Ok(()) => true,
            Err(e) => {
                if last_ok != Some(false) {
                    tracing::warn!("proxy not updated: {e:#}");
                }
                false
            }
        };
        if ok && last_ok == Some(false) {
            tracing::info!("proxy configuration restored");
        }
        last_ok = Some(ok);
        tokio::time::sleep(Duration::from_secs(if ok { 60 } else { 5 })).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> CaddyOptions {
        CaddyOptions {
            admin: "http://127.0.0.1:2019".parse().unwrap(),
            http_port: 80,
            https_port: 443,
            dashboard_port: None,
        }
    }

    #[test]
    fn dashboard_falls_back_to_http_without_a_domain() {
        let o = CaddyOptions { dashboard_port: Some(7460), ..opts() };
        let c = caddy_config(
            &o,
            &[r("a.example.com", TlsMode::Auto, 32001), r("p.lan", TlsMode::Off, 32003)],
            None,
        );
        let http = &c["apps"]["http"]["servers"]["rig_http"]["routes"];
        let last = http.as_array().unwrap().last().unwrap();
        assert_eq!(last["handle"][0]["upstreams"][0]["dial"], "127.0.0.1:7460");
        // The HTTPS name keeps Caddy's redirect; the plain app matched first.
        assert_eq!(last["match"][0]["not"][0]["host"], json!(["a.example.com"]));
        assert_eq!(http[0]["match"][0]["host"], json!(["p.lan"]));
    }

    #[test]
    fn dashboard_domain_gets_its_own_https_route() {
        let o = CaddyOptions { dashboard_port: Some(7460), ..opts() };
        let d = DashboardDomain { host: "rig.example.com".into(), tls_mode: TlsMode::Auto };
        let c = caddy_config(&o, &[], Some(&d));
        let https = &c["apps"]["http"]["servers"]["rig_https"]["routes"];
        assert_eq!(https[0]["match"][0]["host"], json!(["rig.example.com"]));
        assert_eq!(https[0]["handle"][0]["upstreams"][0]["dial"], "127.0.0.1:7460");
        assert!(c["apps"]["http"]["servers"].get("rig_http").is_none());
    }

    fn r(host: &str, tls: TlsMode, port: u16) -> Route {
        Route {
            host: host.into(),
            tls,
            upstream_port: port,
            access: Access::Public,
            port: None,
        }
    }

    #[test]
    fn empty_config_keeps_admin_and_has_no_servers() {
        let c = caddy_config(&opts(), &[], None);
        assert_eq!(c["admin"]["listen"], "127.0.0.1:2019");
        assert_eq!(c["apps"]["http"]["servers"], json!({}));
        assert!(c["apps"].get("tls").is_none());
    }

    #[test]
    fn modes_land_on_the_right_server() {
        let c = caddy_config(
            &opts(),
            &[
                r("a.example.com", TlsMode::Auto, 32001),
                r("b.lan", TlsMode::Internal, 32002),
                r("c.internal", TlsMode::Off, 32003),
            ],
            None,
        );
        let https = &c["apps"]["http"]["servers"]["rig_https"];
        assert_eq!(https["listen"], json!([":443"]));
        assert_eq!(https["routes"].as_array().unwrap().len(), 2);
        assert_eq!(
            https["routes"][0]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:32001"
        );
        let http = &c["apps"]["http"]["servers"]["rig_http"];
        assert_eq!(http["listen"], json!([":80"]));
        assert_eq!(http["routes"][0]["match"][0]["host"], json!(["c.internal"]));
        // Only the internal host is pinned to the local authority.
        assert_eq!(
            c["apps"]["tls"]["automation"]["policies"][0]["subjects"],
            json!(["b.lan"])
        );
    }

    #[test]
    fn restricted_and_tailscale_routes() {
        let lan = Route {
            access: Access::Lan,
            ..r("nas.lan", TlsMode::Internal, 32010)
        };
        let ts_a = Route {
            access: Access::Tailscale,
            port: Some(8443),
            ..r("box.tail1.ts.net", TlsMode::Tailscale, 32011)
        };
        let ts_b = Route {
            access: Access::Tailscale,
            port: Some(8444),
            ..r("box.tail1.ts.net", TlsMode::Tailscale, 32012)
        };
        let plain = Route {
            port: Some(9000),
            ..r("x.internal", TlsMode::Off, 32013)
        };
        let c = caddy_config(&opts(), &[lan, ts_a, ts_b, plain], None);
        let servers = &c["apps"]["http"]["servers"];
        let https = &servers["rig_https"]["routes"];
        // Allow from LAN ranges, then refuse everyone else for that name.
        assert_eq!(https[0]["match"][0]["remote_ip"]["ranges"][0], "10.0.0.0/8");
        assert_eq!(https[1]["handle"][0]["status_code"], 403);
        assert_eq!(servers["rig_port_8443"]["listen"], json!([":8443"]));
        assert_eq!(
            servers["rig_port_8444"]["routes"][0]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:32012"
        );
        assert_eq!(
            servers["rig_port_8443"]["routes"][0]["match"][0]["remote_ip"]["ranges"][0],
            "100.64.0.0/10"
        );
        assert_eq!(servers["rig_port_9000"]["automatic_https"]["disable"], true);
        assert!(servers["rig_https"].get("automatic_https").is_none());
        let policies = &c["apps"]["tls"]["automation"]["policies"];
        assert_eq!(policies[1]["subjects"], json!(["box.tail1.ts.net"]));
        assert_eq!(policies[1]["get_certificate"][0]["via"], "tailscale");
    }
}
