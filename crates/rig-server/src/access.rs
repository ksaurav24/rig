//! Network access modes: who may reach a domain or the dashboard, judged by
//! the address a connection comes from.
//!
//! The same ranges drive Caddy's `remote_ip` matcher (for app domains) and
//! the dashboard's own check, so the two can never disagree. Both look at the
//! direct peer only. A connection that reaches Rig through another proxy on a
//! different machine shows that proxy's address, so access modes are only
//! meaningful when clients connect to this machine directly.

use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

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

pub fn ranges(access: Access) -> &'static [&'static str] {
    match access {
        Access::Public => &[],
        Access::Lan => LAN_RANGES,
        Access::Tailscale => TAILSCALE_RANGES,
    }
}

/// Whether `ip` falls in one of the CIDR `ranges`.
fn in_ranges(ip: IpAddr, ranges: &[&str]) -> bool {
    // An IPv4 client on a dual-stack socket shows up as ::ffff:a.b.c.d.
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    ranges.iter().any(|r| {
        let Some((net, bits)) = r.split_once('/') else {
            return false;
        };
        let (Ok(net), Ok(bits)) = (net.parse::<IpAddr>(), bits.parse::<u32>()) else {
            return false;
        };
        match (ip, net) {
            (IpAddr::V4(a), IpAddr::V4(n)) => {
                let mask = u32::MAX.checked_shl(32 - bits).unwrap_or(0);
                u32::from(a) & mask == u32::from(n) & mask
            }
            (IpAddr::V6(a), IpAddr::V6(n)) => {
                let mask = u128::MAX.checked_shl(128 - bits).unwrap_or(0);
                u128::from(a) & mask == u128::from(n) & mask
            }
            _ => false,
        }
    })
}

pub fn allows(access: Access, ip: IpAddr) -> bool {
    access == Access::Public || in_ranges(ip, ranges(access))
}

/// The dashboard's current mode, kept in memory so the check costs nothing.
#[derive(Clone, Default)]
pub struct DashboardAccess(Arc<AtomicU8>);

impl DashboardAccess {
    pub fn get(&self) -> Access {
        match self.0.load(Ordering::Relaxed) {
            1 => Access::Lan,
            2 => Access::Tailscale,
            _ => Access::Public,
        }
    }

    pub fn set(&self, access: Access) {
        let v = match access {
            Access::Public => 0,
            Access::Lan => 1,
            Access::Tailscale => 2,
        };
        self.0.store(v, Ordering::Relaxed);
    }
}

pub const SETTING: &str = "dashboard_access";

pub fn parse(s: &str) -> Access {
    match s {
        "lan" => Access::Lan,
        "tailscale" => Access::Tailscale,
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
            .map(|c| crate::auth::real_ip(req.headers(), c.0.ip()));
        if !peer.is_some_and(|ip| allows(mode, ip)) {
            let msg = match mode {
                Access::Lan => "This Rig dashboard only answers on the local network.",
                _ => "This Rig dashboard only answers on its tailnet.",
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
        for a in [Access::Lan, Access::Tailscale, Access::Public] {
            d.set(a);
            assert_eq!(d.get(), a);
            assert_eq!(parse(a.as_str()), a);
        }
    }
}
