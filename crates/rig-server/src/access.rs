//! Network access modes: who may reach a domain or the dashboard, judged by
//! the address a connection comes from.
//!
//! The same ranges drive Caddy's `remote_ip` matcher (for app domains) and
//! the dashboard's own check, so the two can never disagree. Both look at the
//! direct peer only. A connection that reaches Rig through another proxy on a
//! different machine shows that proxy's address, so access modes are only
//! meaningful when clients connect to this machine directly.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc, RwLock,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rig_api::Access;

use crate::state::AppState;

/// Private-network ranges, as CIDR strings for Caddy.
pub const LAN_RANGES: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "127.0.0.0/8",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
];

/// Tailscale's address ranges, plus this machine.
pub const TAILSCALE_RANGES: &[&str] = &[
    "100.64.0.0/10",
    "fd7a:115c:a1e0::/48",
    "127.0.0.0/8",
    "::1/128",
];

/// Loopback-only ranges for `local` mode.
pub const LOCAL_RANGES: &[&str] = &["127.0.0.0/8", "::1/128"];

pub fn ranges(access: Access) -> &'static [&'static str] {
    match access {
        Access::Public => &[],
        Access::Lan => LAN_RANGES,
        Access::Tailscale => TAILSCALE_RANGES,
        Access::Local => LOCAL_RANGES,
    }
}

/// Whether `ip` is a globally routable public address (rejects loopback,
/// RFC1918/ULA private, link-local / cloud metadata, CGNAT, multicast,
/// unspecified, broadcast, benchmark, documentation, and reserved ranges).
pub fn is_global_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_global_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_global_v4(v4);
            }
            is_global_v6(v6)
        }
    }
}

fn is_global_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        // 0.0.0.0/8 ("this network")
        || a == 0
        // 100.64.0.0/10 (CGNAT / Tailscale)
        || (a == 100 && (b & 0xc0) == 64)
        // 192.0.0.0/24 (IETF protocol assignments)
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15 (benchmarking)
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4 (reserved)
        || a >= 240)
}

fn is_global_v6(ip: Ipv6Addr) -> bool {
    let segs = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local (fc00::/7)
        || (segs[0] & 0xfe00) == 0xfc00
        // Link-local unicast (fe80::/10)
        || (segs[0] & 0xffc0) == 0xfe80
        // Site-local deprecated (fec0::/10)
        || (segs[0] & 0xffc0) == 0xfec0
        // Documentation (2001:db8::/32)
        || (segs[0] == 0x2001 && segs[1] == 0x0db8)
        // Discard-only (100::/64)
        || (segs[0] == 0x0100 && segs[1] == 0 && segs[2] == 0 && segs[3] == 0))
}

/// Rejects obvious local/internal hostnames and non-global IP literals up front.
pub fn is_blocked_hostname(host: &str) -> bool {
    let h = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if h.is_empty() {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return !is_global_ip(ip);
    }
    h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h == "metadata.google.internal"
}

/// Resolve `host:port` and verify that every returned IP is globally routable
/// unless `allow_private` is true.
pub async fn resolve_and_validate_host(
    host: &str,
    port: u16,
    allow_private: bool,
) -> Result<Vec<SocketAddr>> {
    let clean = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if !allow_private && is_blocked_hostname(clean) {
        bail!("connecting to loopback, private, or local addresses is not allowed");
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((clean, port))
        .await
        .context("resolving host")?
        .collect();
    if addrs.is_empty() {
        bail!("host did not resolve to any address");
    }
    if !allow_private && addrs.iter().any(|a| !is_global_ip(a.ip())) {
        bail!("host resolves to a loopback, private, or link-local address");
    }
    Ok(addrs)
}

/// Whether a host network interface is a container engine bridge whose subnets
/// must not be treated as trusted LAN peers by Caddy or Rig.
fn is_container_bridge_interface(name: &str) -> bool {
    ["docker", "br-", "podman", "cni"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// Computes canonical CIDR strings (e.g. `172.18.0.0/16`, `10.89.0.0/24`) for
/// every active Docker/Podman bridge interface on this host.
pub fn container_bridge_cidrs() -> Vec<String> {
    let Ok(nets) = sysinfo::Networks::new_with_refreshed_list() else {
        return Vec::new();
    };
    let mut cidrs = Vec::new();
    for (name, net) in &nets {
        if !is_container_bridge_interface(&name.to_string_lossy()) {
            continue;
        }
        for ipnet in net.ip_networks() {
            if let Some(cidr) = canonical_cidr(ipnet.addr, u32::from(ipnet.prefix)) {
                cidrs.push(cidr);
            }
        }
    }
    cidrs.sort();
    cidrs.dedup();
    cidrs
}

fn canonical_cidr(addr: IpAddr, prefix: u32) -> Option<String> {
    match addr {
        IpAddr::V4(v4) if prefix <= 32 => {
            if v4.is_loopback() || v4.is_link_local() {
                return None;
            }
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX.checked_shl(32 - prefix).unwrap_or(0)
            };
            let net = Ipv4Addr::from(u32::from(v4) & mask);
            Some(format!("{net}/{prefix}"))
        }
        IpAddr::V6(v6) if prefix <= 128 => {
            if v6.is_loopback() || (v6.segments()[0] & 0xffc0) == 0xfe80 {
                return None;
            }
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX.checked_shl(128 - prefix).unwrap_or(0)
            };
            let net = Ipv6Addr::from(u128::from(v6) & mask);
            Some(format!("{net}/{prefix}"))
        }
        _ => None,
    }
}

/// Whether `ip` falls in one of the CIDR `ranges`.
pub fn in_ranges<S: AsRef<str>>(ip: IpAddr, ranges: &[S]) -> bool {
    // An IPv4 client on a dual-stack socket shows up as ::ffff:a.b.c.d.
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    ranges.iter().any(|r| {
        let Some((net, bits)) = r.as_ref().split_once('/') else {
            return false;
        };
        let (Ok(net), Ok(bits)) = (net.parse::<IpAddr>(), bits.parse::<u32>()) else {
            return false;
        };
        match (ip, net) {
            (IpAddr::V4(a), IpAddr::V4(n)) => {
                let mask = if bits == 0 {
                    0
                } else {
                    u32::MAX.checked_shl(32 - bits).unwrap_or(0)
                };
                u32::from(a) & mask == u32::from(n) & mask
            }
            (IpAddr::V6(a), IpAddr::V6(n)) => {
                let mask = if bits == 0 {
                    0
                } else {
                    u128::MAX.checked_shl(128 - bits).unwrap_or(0)
                };
                u128::from(a) & mask == u128::from(n) & mask
            }
            _ => false,
        }
    })
}

pub fn allows(access: Access, ip: IpAddr) -> bool {
    access == Access::Public || in_ranges(ip, ranges(access))
}

/// Like [`allows`], but also refuses traffic originating from container engine
/// bridge subnets (`docker0`, `br-*`, `podman*`) so compromised containers
/// cannot impersonate trusted LAN peers.
pub fn allows_with_blocked<S: AsRef<str>>(access: Access, ip: IpAddr, blocked: &[S]) -> bool {
    if access == Access::Public {
        return true;
    }
    if !blocked.is_empty() && in_ranges(ip, blocked) {
        return false;
    }
    in_ranges(ip, ranges(access))
}

/// The dashboard's current mode and active container bridge CIDRs, kept in
/// memory so per-request checks cost nothing.
#[derive(Debug, Clone, Default)]
pub struct DashboardAccess {
    mode: Arc<AtomicU8>,
    blocked_bridges: Arc<RwLock<(Vec<String>, Option<Instant>)>>,
}

impl DashboardAccess {
    pub fn get(&self) -> Access {
        match self.mode.load(Ordering::Relaxed) {
            1 => Access::Lan,
            2 => Access::Tailscale,
            3 => Access::Local,
            _ => Access::Public,
        }
    }

    pub fn set(&self, access: Access) {
        let v = match access {
            Access::Public => 0,
            Access::Lan => 1,
            Access::Tailscale => 2,
            Access::Local => 3,
        };
        self.mode.store(v, Ordering::Relaxed);
    }

    pub fn blocked_bridges(&self) -> Vec<String> {
        self.blocked_bridges
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .0
            .clone()
    }

    /// Returns cached bridge CIDRs when refreshed within `max_age`.
    pub fn fresh_blocked_bridges(&self, max_age: Duration) -> Option<Vec<String>> {
        let guard = self
            .blocked_bridges
            .read()
            .unwrap_or_else(|p| p.into_inner());
        let updated = guard.1?;
        (updated.elapsed() < max_age).then(|| guard.0.clone())
    }

    pub fn set_blocked_bridges(&self, cidrs: Vec<String>) {
        *self
            .blocked_bridges
            .write()
            .unwrap_or_else(|p| p.into_inner()) = (cidrs, Some(Instant::now()));
    }
}

pub const SETTING: &str = "dashboard_access";

pub fn parse(s: &str) -> Access {
    match s {
        "lan" => Access::Lan,
        "tailscale" => Access::Tailscale,
        "local" => Access::Local,
        _ => Access::Public,
    }
}

pub async fn load(db: &sqlx::SqlitePool) -> anyhow::Result<Access> {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(SETTING)
        .fetch_optional(db)
        .await?;
    Ok(v.as_deref().map_or(Access::Public, parse))
}

pub async fn store(db: &sqlx::SqlitePool, access: Access) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value,
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')",
    )
    .bind(SETTING)
    .bind(access.as_str())
    .execute(db)
    .await?;
    Ok(())
}

/// Turns away dashboard and API requests from outside the allowed networks.
/// Deploy webhooks stay reachable: they come from Git hosts and carry their
/// own signature.
pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mode = state.access.get();
    if mode != Access::Public && !req.uri().path().starts_with("/api/v1/hooks/") {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| {
                // Local mode ignores forwarded headers completely: only direct loopback peers pass.
                if mode == Access::Local {
                    c.0.ip()
                } else {
                    crate::auth::real_ip(req.headers(), c.0.ip())
                }
            });
        let blocked = state.access.blocked_bridges();
        if !peer.is_some_and(|ip| allows_with_blocked(mode, ip, &blocked)) {
            let msg = match mode {
                Access::Lan => "This Rig server only answers on the local network.",
                Access::Local => "This Rig server only answers on loopback (local mode).",
                _ => "This Rig server only answers on its tailnet.",
            };
            return (axum::http::StatusCode::FORBIDDEN, msg).into_response();
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn lan_and_tailnet_ranges() {
        for lan in [
            "10.1.2.3",
            "172.31.0.1",
            "192.168.1.9",
            "127.0.0.1",
            "::1",
            "fd00::1",
            "fe80::2",
        ] {
            assert!(allows(Access::Lan, ip(lan)), "{lan}");
        }
        for public in ["8.8.8.8", "172.32.0.1", "192.169.0.1", "2001:db8::1"] {
            assert!(!allows(Access::Lan, ip(public)), "{public}");
            assert!(!allows(Access::Tailscale, ip(public)), "{public}");
            assert!(allows(Access::Public, ip(public)), "{public}");
        }
        assert!(allows(Access::Tailscale, ip("100.126.82.121")));
        assert!(allows(Access::Tailscale, ip("fd7a:115c:a1e0::5")));
        assert!(!allows(Access::Tailscale, ip("192.168.1.9")));
        // Tailnet addresses are not "the LAN"; the two modes stay distinct.
        assert!(!allows(Access::Lan, ip("100.126.82.121")));
        assert!(!allows(Access::Tailscale, ip("100.128.0.1")));
        // IPv4-mapped IPv6 is judged as IPv4.
        assert!(allows(Access::Lan, ip("::ffff:192.168.1.9")));
        assert!(!allows(Access::Lan, ip("::ffff:8.8.8.8")));
    }

    #[test]
    fn mode_round_trips() {
        let d = DashboardAccess::default();
        assert_eq!(d.get(), Access::Public);
        for a in [Access::Lan, Access::Tailscale, Access::Local, Access::Public] {
            d.set(a);
            assert_eq!(d.get(), a);
            assert_eq!(parse(a.as_str()), a);
        }
        assert!(allows(Access::Local, ip("127.0.0.1")));
        assert!(allows(Access::Local, ip("::1")));
        assert!(!allows(Access::Local, ip("192.168.1.9")));
        assert!(!allows(Access::Local, ip("100.126.82.121")));
        assert!(!allows(Access::Local, ip("8.8.8.8")));
    }

    #[test]
    fn global_ip_and_blocked_hostnames() {
        for bad in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.5",
        ] {
            assert!(!is_global_ip(ip(bad)), "should reject {bad}");
            assert!(is_blocked_hostname(bad), "hostname should reject {bad}");
        }
        for good in ["1.1.1.1", "8.8.8.8", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(is_global_ip(ip(good)), "should allow {good}");
            assert!(!is_blocked_hostname(good), "hostname should allow {good}");
        }
        for bad_host in ["localhost", "foo.localhost", "printer.local", "metadata.google.internal"] {
            assert!(is_blocked_hostname(bad_host), "{bad_host}");
        }
        assert!(!is_blocked_hostname("hooks.slack.com"));
    }

    #[test]
    fn bridge_cidrs_are_excluded_from_lan() {
        let bridges = vec!["172.18.0.0/16".to_string(), "10.89.0.0/24".to_string()];
        assert!(!allows_with_blocked(Access::Lan, ip("172.18.0.5"), &bridges));
        assert!(!allows_with_blocked(Access::Lan, ip("10.89.0.12"), &bridges));
        assert!(allows_with_blocked(Access::Lan, ip("192.168.1.10"), &bridges));
        assert!(allows_with_blocked(Access::Lan, ip("10.0.1.5"), &bridges));
        // Public routes remain public.
        assert!(allows_with_blocked(Access::Public, ip("172.18.0.5"), &bridges));
    }
}
