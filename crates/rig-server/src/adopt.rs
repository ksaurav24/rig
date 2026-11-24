//! One-click adoption of existing Docker/Podman containers, Compose stacks,
//! Coolify, and Dokploy workloads into Rig (`rig adopt`).
//!
//! Safety guarantees:
//! - Discovery never leaks suspected secret values in plaintext.
//! - Dry-run previews touch nothing on disk, in SQLite, or in the container engine.
//! - Host bind mounts and existing named volumes keep their exact paths/names
//!   (`owned = 0`), so no data is ever moved, copied, or deleted by Rig.
//! - Cutover performs automatic rollback if the newly adopted container fails
//!   its health check (restarting the old container if it had to be paused for
//!   writable mounts, and cleaning up all created DB rows).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Result, anyhow};
use rig_api::{
    Access, AddDomainRequest, AdoptEnvPreview, AdoptExecuteRequest, AdoptExecuteResult,
    AdoptPreview, AdoptPreviewRequest, DiscoveredContainer, DiscoveredDomain, DiscoveredEnvVar,
    DiscoveredMount, DiscoveredPort, TlsMode,
};
use tokio::sync::mpsc;

use crate::{
    auth::{Admin, Auth},
    crypto::random_token,
    deploy::{self, DEFAULT_APP_MEMORY_MB, LABEL_APP, LABEL_DEPLOYMENT, LABEL_MANAGED},
    error::{ApiError, ApiResult},
    proxy,
    routes::apps::{load_app, validate_host},
    routes::auth::slugify,
    runtime::{ContainerSpec, InspectedContainer, LogSink, VolumeMount},
    state::{AppState, new_id, now},
};

const STOP_GRACE_SECS: u32 = 10;

/// Inspect all unmanaged containers on the server and enrich them with origin,
/// proxy domain rules, port detection, and vault secret heuristics.
pub async fn discover_containers(state: &AppState) -> Result<Vec<DiscoveredContainer>> {
    let raw = state.runtime.list_external_containers().await?;
    let mut out: Vec<DiscoveredContainer> = raw
        .into_iter()
        .filter(|c| !is_rig_internal(c))
        .map(enrich_container)
        .collect();
    // Running containers first, then sorted by stack & name.
    out.sort_by(|a, b| {
        let a_run = a.state != "running";
        let b_run = b.state != "running";
        (a_run, a.stack.as_deref().unwrap_or(""), a.name.as_str()).cmp(&(
            b_run,
            b.stack.as_deref().unwrap_or(""),
            b.name.as_str(),
        ))
    });
    Ok(out)
}

fn is_rig_internal(c: &InspectedContainer) -> bool {
    c.labels.get(LABEL_MANAGED).map(String::as_str) == Some("true")
        || c.name.starts_with("rig-buildkit")
}

/// Convert a raw engine inspection into a `DiscoveredContainer` with secrets masked.
pub fn enrich_container(c: InspectedContainer) -> DiscoveredContainer {
    let (origin, stack, service) = detect_origin(&c.labels);
    let domains = extract_domains(&c.labels, &c.env);
    let ports: Vec<DiscoveredPort> = c
        .ports
        .iter()
        .map(|p| DiscoveredPort {
            container_port: p.container_port,
            protocol: p.protocol.clone(),
            host_ip: p.host_ip.clone(),
            host_port: p.host_port,
        })
        .collect();
    let detected_port = detect_primary_port(&ports, &domains, &c.env);
    let mounts: Vec<DiscoveredMount> = c
        .mounts
        .iter()
        .map(|m| DiscoveredMount {
            kind: m.kind.clone(),
            source: m.source.clone(),
            destination: m.destination.clone(),
            read_only: m.read_only,
        })
        .collect();
    let env: Vec<DiscoveredEnvVar> = c
        .env
        .iter()
        .filter(|(k, _)| !is_standard_runtime_env(k))
        .map(|(k, v)| {
            let reason = classify_secret_env(k, v);
            DiscoveredEnvVar {
                key: k.clone(),
                preview_value: if reason.is_some() {
                    None
                } else {
                    Some(v.clone())
                },
                is_secret_guess: reason.is_some(),
                secret_reason: reason.map(str::to_owned),
            }
        })
        .collect();
    let health_path = extract_health_path(&c.labels);

    DiscoveredContainer {
        id: c.id,
        name: c.name,
        state: c.state,
        image: c.image,
        image_digest: c.image_digest,
        image_id: c.image_id,
        origin,
        stack,
        service,
        ports,
        detected_port,
        mounts,
        networks: c.networks,
        env,
        domains,
        restart_policy: c.restart_policy,
        memory_mb: c.memory_mb,
        cpu_millis: c.cpu_millis,
        command: c.command,
        entrypoint: c.entrypoint,
        health_path,
        created_at: c.created_at,
    }
}

/// Detect whether a container belongs to Coolify, Dokploy, Docker Compose, or standalone.
pub fn detect_origin(labels: &HashMap<String, String>) -> (String, Option<String>, Option<String>) {
    let service = labels.get("com.docker.compose.service").cloned();
    let compose_project = labels.get("com.docker.compose.project").cloned();

    let is_coolify = labels.get("coolify.managed").map(String::as_str) == Some("true")
        || labels.keys().any(|k| k.starts_with("coolify."));
    if is_coolify {
        let stack = labels
            .get("coolify.projectName")
            .or_else(|| labels.get("coolify.serviceName"))
            .or_else(|| labels.get("coolify.applicationId"))
            .cloned()
            .or(compose_project);
        return ("coolify".into(), stack, service);
    }

    let is_dokploy = labels.keys().any(|k| k.starts_with("dokploy.") || k.contains("dokploy"));
    if is_dokploy {
        let stack = labels
            .get("dokploy.projectName")
            .or_else(|| labels.get("dokploy.appName"))
            .cloned()
            .or(compose_project);
        return ("dokploy".into(), stack, service);
    }

    if compose_project.is_some() || service.is_some() {
        return ("compose".into(), compose_project, service);
    }

    ("standalone".into(), None, None)
}

/// Extract reverse-proxy domain rules from Traefik labels, Caddy docker-proxy
/// labels, and nginx-proxy `VIRTUAL_HOST` env vars.
pub fn extract_domains(
    labels: &HashMap<String, String>,
    env: &[(String, String)],
) -> Vec<DiscoveredDomain> {
    let mut domains: Vec<DiscoveredDomain> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1. Traefik v2/v3 labels (used by standalone Traefik, Coolify, and Dokploy).
    // Collect service ports first (`traefik.http.services.<svc>.loadbalancer.server.port`).
    let mut traefik_service_ports: HashMap<String, u16> = HashMap::new();
    let mut default_traefik_port: Option<u16> = None;
    for (k, v) in labels {
        if let Some(rest) = k.strip_prefix("traefik.http.services.")
            && let Some((svc, suffix)) = rest.split_once('.')
            && suffix == "loadbalancer.server.port"
            && let Ok(port) = v.trim().parse::<u16>()
        {
            traefik_service_ports.insert(svc.to_string(), port);
            if default_traefik_port.is_none() {
                default_traefik_port = Some(port);
            }
        }
    }

    // Group router rules (`traefik.http.routers.<router>.rule`).
    let mut router_rules: BTreeMap<String, String> = BTreeMap::new();
    let mut router_tls: HashMap<String, bool> = HashMap::new();
    let mut router_service: HashMap<String, String> = HashMap::new();
    for (k, v) in labels {
        if let Some(rest) = k.strip_prefix("traefik.http.routers.")
            && let Some((router, suffix)) = rest.split_once('.')
        {
            match suffix {
                "rule" => {
                    router_rules.insert(router.to_string(), v.clone());
                }
                "tls" => {
                    router_tls.insert(router.to_string(), !v.eq_ignore_ascii_case("false"));
                }
                s if s.starts_with("tls.") => {
                    router_tls.insert(router.to_string(), true);
                }
                "service" => {
                    router_service.insert(router.to_string(), v.clone());
                }
                _ => {}
            }
        }
    }

    for (router, rule) in router_rules {
        let tls = router_tls.get(&router).copied().unwrap_or(false);
        let svc_port = router_service
            .get(&router)
            .and_then(|s| traefik_service_ports.get(s).copied())
            .or_else(|| traefik_service_ports.get(&router).copied())
            .or(default_traefik_port);
        for host in parse_traefik_hosts(&rule) {
            let lower = host.to_ascii_lowercase();
            if seen.insert(lower.clone()) {
                domains.push(DiscoveredDomain {
                    host: lower,
                    tls_mode: if tls { TlsMode::Auto } else { TlsMode::Off },
                    source: "traefik".into(),
                    service_port: svc_port,
                });
            }
        }
    }

    // 2. Caddy docker-proxy labels (`caddy=example.com`, `caddy_0=https://a.com`, `caddy.reverse_proxy={{upstreams 8080}}`).
    let caddy_port = labels.iter().find_map(|(k, v)| {
        if k.ends_with(".reverse_proxy") && k.starts_with("caddy") {
            parse_caddy_upstream_port(v)
        } else {
            None
        }
    });
    for (k, v) in labels {
        if k == "caddy" || (k.starts_with("caddy_") && !k.contains('.')) {
            for raw_host in v.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()) {
                let (tls_mode, stripped) = if let Some(h) = raw_host.strip_prefix("http://") {
                    (TlsMode::Off, h)
                } else if let Some(h) = raw_host.strip_prefix("https://") {
                    (TlsMode::Auto, h)
                } else {
                    (TlsMode::Auto, raw_host)
                };
                let host = stripped.split(':').next().unwrap_or(stripped).trim();
                if is_valid_discovered_host(host) {
                    let lower = host.to_ascii_lowercase();
                    if seen.insert(lower.clone()) {
                        domains.push(DiscoveredDomain {
                            host: lower,
                            tls_mode,
                            source: "caddy".into(),
                            service_port: caddy_port,
                        });
                    }
                }
            }
        }
    }

    // 3. nginx-proxy `VIRTUAL_HOST` / `LETSENCRYPT_HOST` env vars.
    let virtual_port = env
        .iter()
        .find(|(k, _)| k == "VIRTUAL_PORT")
        .and_then(|(_, v)| v.trim().parse::<u16>().ok());
    let has_le = env.iter().any(|(k, v)| k == "LETSENCRYPT_HOST" && !v.trim().is_empty());
    if let Some((_, vh)) = env.iter().find(|(k, _)| k == "VIRTUAL_HOST") {
        for raw_host in vh.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if is_valid_discovered_host(raw_host) {
                let lower = raw_host.to_ascii_lowercase();
                if seen.insert(lower.clone()) {
                    domains.push(DiscoveredDomain {
                        host: lower,
                        tls_mode: if has_le { TlsMode::Auto } else { TlsMode::Off },
                        source: "virtual_host".into(),
                        service_port: virtual_port,
                    });
                }
            }
        }
    }

    domains
}

/// Parses `Host(\`a.example.com\`)` and `Host(\`a.com\`, \`b.com\`)` expressions from Traefik rules.
pub fn parse_traefik_hosts(rule: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    let bytes = rule.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if rule[i..].starts_with("Host(") {
            let start = i + 5;
            if let Some(rel_end) = rule[start..].find(')') {
                let inner = &rule[start..start + rel_end];
                for part in inner.split(',') {
                    let host = part.trim().trim_matches(['`', '"', '\'']).trim();
                    if is_valid_discovered_host(host) {
                        hosts.push(host.to_string());
                    }
                }
                i = start + rel_end + 1;
                continue;
            }
        }
        i += 1;
    }
    hosts
}

fn parse_caddy_upstream_port(val: &str) -> Option<u16> {
    // Handles `{{upstreams 8080}}` or `{{upstreams http 8080}}`.
    let trimmed = val.trim().trim_matches(['{', '}']).trim();
    trimmed
        .split_whitespace()
        .rev()
        .find_map(|tok| tok.parse::<u16>().ok())
}

fn is_valid_discovered_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.contains('*')
        && !host.contains('/')
        && host
            .split('.')
            .all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

fn extract_health_path(labels: &HashMap<String, String>) -> Option<String> {
    for (k, v) in labels {
        if k.starts_with("traefik.http.services.")
            && k.ends_with(".loadbalancer.healthcheck.path")
            && v.starts_with('/')
        {
            return Some(v.clone());
        }
    }
    None
}

fn detect_primary_port(
    ports: &[DiscoveredPort],
    domains: &[DiscoveredDomain],
    env: &[(String, String)],
) -> Option<u16> {
    if let Some(p) = domains.iter().find_map(|d| d.service_port) {
        return Some(p);
    }
    if let Some(p) = ports
        .iter()
        .find(|p| p.protocol == "tcp" && p.host_port.is_some())
        .map(|p| p.container_port)
    {
        return Some(p);
    }
    if let Some(p) = ports
        .iter()
        .find(|p| p.protocol == "tcp")
        .map(|p| p.container_port)
    {
        return Some(p);
    }
    env.iter()
        .find(|(k, _)| k == "PORT" || k == "APP_PORT" || k == "HTTP_PORT")
        .and_then(|(_, v)| v.trim().parse::<u16>().ok())
        .filter(|&p| p > 0)
}

fn is_standard_runtime_env(key: &str) -> bool {
    matches!(
        key,
        "PATH" | "HOSTNAME" | "HOME" | "TERM" | "SHLVL" | "PWD" | "GPG_KEY" | "PYTHON_VERSION"
            | "NODE_VERSION" | "YARN_VERSION" | "GOLANG_VERSION" | "RUST_VERSION"
    )
}

/// Heuristic classifier identifying env vars that should be encrypted as secrets in Rig's vault.
pub fn classify_secret_env(key: &str, value: &str) -> Option<&'static str> {
    let upper = key.to_ascii_uppercase();
    const SENSITIVE_PARTS: &[(&str, &str)] = &[
        ("PASSWORD", "key contains PASSWORD"),
        ("PASSWD", "key contains PASSWD"),
        ("SECRET", "key contains SECRET"),
        ("TOKEN", "key contains TOKEN"),
        ("API_KEY", "key contains API_KEY"),
        ("APIKEY", "key contains APIKEY"),
        ("PRIVATE_KEY", "key contains PRIVATE_KEY"),
        ("PRIV_KEY", "key contains PRIV_KEY"),
        ("ACCESS_KEY", "key contains ACCESS_KEY"),
        ("AUTH", "key contains AUTH"),
        ("CREDENTIAL", "key contains CREDENTIAL"),
        ("COOKIE_KEY", "key contains COOKIE_KEY"),
        ("SIGNING_KEY", "key contains SIGNING_KEY"),
        ("ENCRYPTION_KEY", "key contains ENCRYPTION_KEY"),
        ("MASTER_KEY", "key contains MASTER_KEY"),
        ("JWT", "key contains JWT"),
        ("WEBHOOK", "key contains WEBHOOK"),
        ("DATABASE_URL", "database connection string"),
        ("REDIS_URL", "redis connection string"),
        ("MONGO_URL", "database connection string"),
        ("DSN", "key contains DSN"),
        ("CONNECTION_STRING", "connection string"),
        ("CLIENT_SECRET", "OAuth client secret"),
        ("STRIPE", "payment key"),
        ("OPENAI", "AI provider key"),
        ("ANTHROPIC", "AI provider key"),
        ("AWS_", "AWS credential"),
    ];
    for (needle, reason) in SENSITIVE_PARTS {
        if upper.contains(needle) {
            return Some(reason);
        }
    }
    if upper.ends_with("_KEY") || upper.ends_with("_PASS") || upper.ends_with("_PWD") {
        return Some("key name suggests a secret");
    }
    // Detect URLs containing embedded credentials (`scheme://user:pass@host`).
    if let Some((_, rest)) = value.split_once("://")
        && let Some((userinfo, _)) = rest.split_once('@')
        && userinfo.contains(':')
        && !userinfo.contains('/')
    {
        return Some("value contains URL credentials");
    }
    // Detect PEM private keys or common token prefixes.
    if value.contains("-----BEGIN ")
        || value.starts_with("sk-")
        || value.starts_with("ghp_")
        || value.starts_with("gho_")
        || value.starts_with("glpat-")
        || value.starts_with("xoxb-")
        || value.starts_with("rig_")
    {
        return Some("value matches known secret format");
    }
    None
}

fn resolve_secret_set(
    env: &[DiscoveredEnvVar],
    secret_keys: Option<&[String]>,
    plain_keys: Option<&[String]>,
) -> BTreeSet<String> {
    let mut secrets: BTreeSet<String> = env
        .iter()
        .filter(|e| e.is_secret_guess)
        .map(|e| e.key.clone())
        .collect();
    if let Some(extra) = secret_keys {
        for k in extra {
            secrets.insert(k.clone());
        }
    }
    if let Some(plain) = plain_keys {
        for k in plain {
            secrets.remove(k);
        }
    }
    secrets
}

fn suggest_app_name(c: &DiscoveredContainer) -> String {
    let raw = c
        .service
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&c.name);
    let trimmed = raw.trim_start_matches('/').trim();
    if trimmed.is_empty() {
        "adopted-app".into()
    } else {
        trimmed.chars().take(64).collect()
    }
}

/// Generate a preview of `rig.toml` for an adopted service.
#[allow(clippy::too_many_arguments)]
pub fn render_rig_toml(
    app_name: &str,
    project_name: Option<&str>,
    image: &str,
    port: u16,
    health_path: Option<&str>,
    memory_mb: Option<u32>,
    cpu_millis: Option<u32>,
    mounts: &[DiscoveredMount],
    domains: &[AddDomainRequest],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("app = {:?}\n", app_name));
    if let Some(proj) = project_name {
        out.push_str(&format!("project = {:?}\n", proj));
    }
    out.push_str(&format!("image = {:?}\n", image));
    out.push_str(&format!("port = {port}\n"));
    if let Some(hp) = health_path {
        out.push_str(&format!("health_path = {:?}\n", hp));
    }
    if let Some(mem) = memory_mb {
        out.push_str(&format!("memory_mb = {mem}\n"));
    }
    if let Some(cpu) = cpu_millis {
        out.push_str(&format!("cpu_millis = {cpu}\n"));
    }
    if !mounts.is_empty() {
        out.push_str("volumes = [\n");
        for m in mounts {
            let suffix = if m.read_only { ":ro" } else { "" };
            out.push_str(&format!("  {:?},\n", format!("{}:{}{suffix}", m.source, m.destination)));
        }
        out.push_str("]\n");
    }
    if !domains.is_empty() {
        out.push_str("domains = [\n");
        for d in domains {
            out.push_str(&format!("  {:?},\n", d.host));
        }
        out.push_str("]\n");
    }
    out
}

/// Generate Podman Quadlet `.container` and `.network` unit file previews.
#[allow(clippy::too_many_arguments)]
pub fn render_quadlet_units(
    app_name: &str,
    network_name: &str,
    image: &str,
    port: u16,
    health_path: Option<&str>,
    memory_mb: Option<u32>,
    cpu_millis: Option<u32>,
    mounts: &[DiscoveredMount],
    env_keys: &[AdoptEnvPreview],
    command: Option<&[String]>,
    restart_policy: Option<&str>,
) -> (String, String) {
    let slug = slugify(app_name);
    let mut container = String::new();
    container.push_str(&format!("# rig-{slug}.container (Podman Quadlet)\n"));
    container.push_str("[Unit]\n");
    container.push_str(&format!("Description=Rig managed service ({app_name})\n"));
    container.push_str("After=network-online.target\n\n");
    container.push_str("[Container]\n");
    container.push_str(&format!("ContainerName=rig-{slug}\n"));
    container.push_str(&format!("Image={image}\n"));
    container.push_str(&format!("Network={network_name}.network\n"));
    container.push_str(&format!("PublishPort=127.0.0.1::{port}\n"));
    container.push_str("Label=rig.managed=true\n");
    container.push_str(&format!("Label=rig.adopted={slug}\n"));
    container.push_str("NoNewPrivileges=true\n");
    container.push_str("DropCapability=NET_RAW\n");
    container.push_str("PidsLimit=512\n");
    if let Some(hp) = health_path {
        container.push_str(&format!(
            "HealthCmd=curl -fsS http://127.0.0.1:{port}{hp} || exit 1\n"
        ));
        container.push_str("HealthInterval=15s\n");
    }
    for m in mounts {
        let opt = if m.read_only { ":ro" } else { ":rw" };
        container.push_str(&format!("Volume={}:{}{opt}\n", m.source, m.destination));
    }
    for e in env_keys {
        if e.is_secret {
            container.push_str(&format!(
                "# Vault secret injected at runtime (never stored plaintext)\nSecret=rig-{slug}-{},type=env,target={}\n",
                slugify(&e.key),
                e.key
            ));
        } else {
            container.push_str(&format!("Environment={}=<from-rig-state>\n", e.key));
        }
    }
    if let Some(cmd) = command
        && !cmd.is_empty()
    {
        container.push_str(&format!("Exec={}\n", cmd.join(" ")));
    }
    container.push_str("\n[Service]\n");
    let restart = match restart_policy {
        Some("no") => "no",
        Some("on-failure") => "on-failure",
        _ => "always",
    };
    container.push_str(&format!("Restart={restart}\n"));
    container.push_str("TimeoutStartSec=90\n");
    if let Some(mem) = memory_mb {
        container.push_str(&format!("MemoryMax={mem}M\n"));
    }
    if let Some(cpu) = cpu_millis {
        // systemd CPUQuota is percentage of one core (1000m = 100%).
        let pct = (f64::from(cpu) / 10.0).round() as u32;
        container.push_str(&format!("CPUQuota={pct}%\n"));
    }
    container.push_str("\n[Install]\n");
    container.push_str("WantedBy=default.target\n");

    let network = format!(
        "# {network_name}.network (Podman Quadlet)\n[Unit]\nDescription=Rig project network ({network_name})\n\n[Network]\nNetworkName={network_name}\nLabel=rig.managed=true\n"
    );

    (container, network)
}

/// Build a dry-run preview without modifying anything on disk, in SQLite, or in the engine.
pub async fn preview_adopt(
    state: &AppState,
    org_id: &str,
    req: &AdoptPreviewRequest,
) -> ApiResult<AdoptPreview> {
    let raw = state
        .runtime
        .inspect_container_full(&req.container_id)
        .await
        .map_err(|e| ApiError::Unavailable(format!("inspecting container: {e:#}")))?
        .ok_or(ApiError::NotFound("container"))?;
    if is_rig_internal(&raw) {
        return Err(ApiError::validation(
            "this container is already managed by Rig",
        ));
    }

    let discovered = enrich_container(raw);
    let project_name = match &req.project_id {
        Some(pid) => sqlx::query_scalar::<_, String>(
            "SELECT name FROM projects WHERE id = ? AND org_id = ?",
        )
        .bind(pid)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?,
        None => None,
    };

    let app_name = req
        .app_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| suggest_app_name(&discovered));

    let port = req
        .port
        .or(discovered.detected_port)
        .unwrap_or(80);
    if port == 0 {
        return Err(ApiError::validation("port must be between 1 and 65535"));
    }

    let health_path = req
        .health_path
        .clone()
        .or_else(|| discovered.health_path.clone());

    let image = discovered
        .image_digest
        .clone()
        .unwrap_or_else(|| discovered.image.clone());

    let domains: Vec<AddDomainRequest> = match &req.domains {
        Some(explicit) => explicit.clone(),
        None => discovered
            .domains
            .iter()
            .map(|d| AddDomainRequest {
                host: d.host.clone(),
                tls_mode: d.tls_mode,
                access: Access::Public,
                port: None,
            })
            .collect(),
    };

    let secrets = resolve_secret_set(
        &discovered.env,
        req.secret_keys.as_deref(),
        req.plain_keys.as_deref(),
    );
    let env_keys: Vec<AdoptEnvPreview> = discovered
        .env
        .iter()
        .map(|e| AdoptEnvPreview {
            key: e.key.clone(),
            is_secret: secrets.contains(&e.key),
            secret_reason: e.secret_reason.clone(),
        })
        .collect();

    let has_writable_mounts = discovered.mounts.iter().any(|m| !m.read_only);
    let cutover_strategy = if has_writable_mounts {
        "stop-start-with-rollback".to_string()
    } else {
        "zero-downtime".to_string()
    };

    let mut warnings = Vec::new();
    if discovered.state != "running" {
        warnings.push(format!(
            "Source container is currently `{}`; adopting will start a fresh Rig-managed container.",
            discovered.state
        ));
    }
    if has_writable_mounts {
        warnings.push(
            "Container has writable mounts: Rig will briefly stop the old container before starting the adopted one to prevent concurrent writes, and automatically restart the old container if health checks fail."
                .into(),
        );
    }
    let bind_count = discovered.mounts.iter().filter(|m| m.kind == "bind").count();
    if bind_count > 0 {
        warnings.push(format!(
            "{bind_count} host bind mount(s) will be preserved in-place at their exact host paths (Rig will never move or delete host bind data)."
        ));
    }
    let secret_count = env_keys.iter().filter(|e| e.is_secret).count();
    if secret_count > 0 {
        warnings.push(format!(
            "{secret_count} environment variable(s) flagged as secrets will be encrypted into Rig's XChaCha20-Poly1305 vault."
        ));
    }
    if discovered.detected_port.is_none() && req.port.is_none() {
        warnings.push(
            "Could not auto-detect an exposed container port; defaulting to port 80 (override with --port)."
                .into(),
        );
    }
    // Check if any discovered domain already exists in Rig.
    for d in &domains {
        if let Ok(h) = validate_host(&d.host) {
            let taken: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM domains WHERE host = ? AND ifnull(port, 0) = ifnull(?, 0)",
            )
            .bind(&h)
            .bind(d.port.map(i64::from))
            .fetch_optional(&state.db)
            .await?;
            if taken.is_some() {
                warnings.push(format!(
                    "Domain `{h}` is already registered in Rig; it will be skipped unless changed."
                ));
            }
        }
    }

    let network_name = req
        .project_id
        .as_deref()
        .map(deploy::project_network)
        .unwrap_or_else(|| "rig-project".into());

    let rig_toml = render_rig_toml(
        &app_name,
        project_name.as_deref(),
        &image,
        port,
        health_path.as_deref(),
        discovered.memory_mb,
        discovered.cpu_millis,
        &discovered.mounts,
        &domains,
    );
    let (quadlet_container, quadlet_network) = render_quadlet_units(
        &app_name,
        &network_name,
        &image,
        port,
        health_path.as_deref(),
        discovered.memory_mb,
        discovered.cpu_millis,
        &discovered.mounts,
        &env_keys,
        discovered.command.as_deref(),
        discovered.restart_policy.as_deref(),
    );

    Ok(AdoptPreview {
        container: discovered.clone(),
        app_name,
        project_id: req.project_id.clone(),
        project_name,
        image,
        port,
        health_path,
        memory_mb: discovered.memory_mb,
        cpu_millis: discovered.cpu_millis,
        mounts: discovered.mounts,
        domains,
        env_keys,
        rig_toml,
        quadlet_container,
        quadlet_network,
        cutover_strategy,
        warnings,
    })
}

/// Execute safe adoption of an existing container into Rig with health check
/// verification, atomic Caddy cutover, and automatic rollback on failure.
pub async fn execute_adopt(
    state: &AppState,
    auth: &Auth<Admin>,
    req: &AdoptExecuteRequest,
) -> ApiResult<AdoptExecuteResult> {
    let raw = state
        .runtime
        .inspect_container_full(&req.container_id)
        .await
        .map_err(|e| ApiError::Unavailable(format!("inspecting container: {e:#}")))?
        .ok_or(ApiError::NotFound("container"))?;
    if is_rig_internal(&raw) {
        return Err(ApiError::validation(
            "this container is already managed by Rig",
        ));
    }

    // Verify project exists in caller's org.
    let project_name: String = sqlx::query_scalar(
        "SELECT name FROM projects WHERE id = ? AND org_id = ?",
    )
    .bind(&req.project_id)
    .bind(&auth.org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("project"))?;

    let discovered = enrich_container(raw.clone());
    let app_name = req
        .app_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| suggest_app_name(&discovered));
    if app_name.len() > 64 {
        return Err(ApiError::validation("app name must be 64 characters or fewer"));
    }
    let slug = slugify(&app_name);
    let taken: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM apps WHERE project_id = ?1 AND (slug = ?2 OR name = ?3))
              + (SELECT COUNT(*) FROM databases WHERE project_id = ?1 AND slug = ?2)",
    )
    .bind(&req.project_id)
    .bind(&slug)
    .bind(&app_name)
    .fetch_one(&state.db)
    .await?;
    if taken > 0 {
        return Err(ApiError::Conflict(format!(
            "project {project_name} already has an app or database called {app_name}. Pass --app to pick another name"
        )));
    }

    let port = req.port.or(discovered.detected_port).unwrap_or(80);
    if port == 0 {
        return Err(ApiError::validation("port must be between 1 and 65535"));
    }
    let health_path = req
        .health_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| discovered.health_path.clone());
    if let Some(hp) = &health_path
        && (!hp.starts_with('/') || hp.len() > 200 || !hp.chars().all(|c| c.is_ascii_graphic()))
    {
        return Err(ApiError::validation(
            "health_path must start with / and contain no spaces",
        ));
    }

    let image_ref = discovered
        .image_digest
        .clone()
        .unwrap_or_else(|| discovered.image.clone());
    let image_to_run = raw
        .image_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| image_ref.clone());

    // Validate host bind mount paths exist BEFORE touching the old container.
    for m in &raw.mounts {
        if m.kind == "bind" && !std::path::Path::new(&m.source).exists() {
            return Err(ApiError::validation(format!(
                "bind mount host path `{}` does not exist on this server",
                m.source
            )));
        }
    }

    // Prepare env vars (using live plaintext values read directly from the engine socket).
    let secret_set = resolve_secret_set(
        &discovered.env,
        req.secret_keys.as_deref(),
        req.plain_keys.as_deref(),
    );
    let live_env: Vec<(String, String, bool)> = raw
        .env
        .iter()
        .filter(|(k, _)| !is_standard_runtime_env(k))
        .map(|(k, v)| (k.clone(), v.clone(), secret_set.contains(k)))
        .collect();
    let secrets_encrypted = live_env.iter().filter(|(_, _, s)| *s).count();

    // Prepare domains to register (skip any conflicting ones rather than failing cutover,
    // unless caller explicitly supplied them).
    let explicit_domains = req.domains.is_some();
    let candidate_domains: Vec<AddDomainRequest> = match &req.domains {
        Some(d) => d.clone(),
        None => discovered
            .domains
            .iter()
            .map(|d| AddDomainRequest {
                host: d.host.clone(),
                tls_mode: d.tls_mode,
                access: Access::Public,
                port: None,
            })
            .collect(),
    };
    let mut verified_domains: Vec<(String, TlsMode, Access, Option<u16>)> = Vec::new();
    for d in &candidate_domains {
        let host = match validate_host(&d.host) {
            Ok(h) => h,
            Err(e) if explicit_domains => return Err(e),
            Err(_) => continue,
        };
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM domains WHERE host = ? AND ifnull(port, 0) = ifnull(?, 0)",
        )
        .bind(&host)
        .bind(d.port.map(i64::from))
        .fetch_optional(&state.db)
        .await?;
        if exists.is_some() {
            if explicit_domains {
                return Err(ApiError::Conflict(format!(
                    "domain {host} is already in use by another app"
                )));
            }
            continue;
        }
        verified_domains.push((host, d.tls_mode, d.access, d.port));
    }

    // Extra non-default networks from the old container so Compose sibling DNS keeps working.
    let extra_networks: Vec<String> = raw
        .networks
        .iter()
        .filter(|n| !matches!(n.as_str(), "bridge" | "host" | "none" | "default" | "podman"))
        .cloned()
        .collect();

    let app_id = new_id();
    let dep_id = new_id();
    let short = &dep_id[dep_id.len().saturating_sub(8)..];
    let at = now();

    // Build ContainerSpec preserving exact image, mounts, env, command, entrypoint, and resources.
    let mut container_env: Vec<(String, String)> = live_env
        .iter()
        .map(|(k, v, _)| (k.clone(), v.clone()))
        .collect();
    if !container_env.iter().any(|(k, _)| k == "PORT") {
        container_env.push(("PORT".into(), port.to_string()));
    }
    let volume_mounts: Vec<VolumeMount> = raw
        .mounts
        .iter()
        .map(|m| VolumeMount {
            source: m.source.clone(),
            target: m.destination.clone(),
            read_only: m.read_only,
        })
        .collect();
    let project_net = deploy::project_network(&req.project_id);
    let spec = ContainerSpec {
        name: format!("rig-{slug}-{short}"),
        image: image_to_run.clone(),
        port,
        env: container_env,
        labels: HashMap::from([
            (LABEL_MANAGED.to_string(), "true".to_string()),
            (LABEL_APP.to_string(), app_id.clone()),
            (LABEL_DEPLOYMENT.to_string(), dep_id.clone()),
            ("rig.adopted_from".to_string(), raw.name.clone()),
        ]),
        memory_mb: Some(raw.memory_mb.unwrap_or(DEFAULT_APP_MEMORY_MB)),
        cpu_millis: raw.cpu_millis,
        command: raw.command.clone(),
        entrypoint: raw.entrypoint.clone(),
        network: Some(project_net.clone()),
        aliases: vec![slug.clone()],
        extra_networks: extra_networks.clone(),
        volumes: volume_mounts,
    };

    // ---- Cutover with automatic rollback
    let old_was_running = raw.state == "running";
    let has_writable_mounts = raw.mounts.iter().any(|m| !m.read_only);
    let mut old_paused = false;

    if has_writable_mounts && old_was_running {
        state
            .runtime
            .stop(&raw.id, STOP_GRACE_SECS)
            .await
            .map_err(|e| {
                ApiError::Unavailable(format!(
                    "could not pause old container {} before mount handoff: {e:#}",
                    raw.name
                ))
            })?;
        old_paused = true;
    }

    let (log_tx, mut log_rx) = mpsc::channel::<crate::runtime::LogMsg>(128);
    let log_drain = tokio::spawn(async move {
        let mut lines = Vec::new();
        while let Some(msg) = log_rx.recv().await {
            lines.push(msg.line);
        }
        lines
    });
    let log = LogSink::new(log_tx);

    let started = match state.runtime.run(&spec).await {
        Ok(s) => s,
        Err(e) => {
            if old_paused {
                let _ = state.runtime.start(&raw.id).await;
            }
            drop(log);
            let _ = log_drain.await;
            return Err(ApiError::Unavailable(format!(
                "starting adopted container failed (old container restored): {e:#}"
            )));
        }
    };

    if let Err(e) =
        deploy::wait_container_healthy(state, &started, port, health_path.as_deref(), &log).await
    {
        let _ = state.runtime.remove(&started.id).await;
        if old_paused {
            let _ = state.runtime.start(&raw.id).await;
        }
        drop(log);
        let _ = log_drain.await;
        return Err(ApiError::Unavailable(format!(
            "adopted container failed health check and was rolled back (old container {} remains running): {e:#}",
            raw.name
        )));
    }
    drop(log);
    let captured_logs = log_drain.await.unwrap_or_default();

    // ---- Register app, vault secrets, preserved mounts, domains, and running deployment atomically
    let server_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM servers WHERE org_id = ? AND kind = 'local' LIMIT 1")
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;
    let webhook_secret_enc = state.vault.encrypt(&random_token(24))?;
    let cmd_json = raw
        .command
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| ApiError::Internal(anyhow!(e)))?;
    let ep_json = raw
        .entrypoint
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| ApiError::Internal(anyhow!(e)))?;
    let nets_json = (!extra_networks.is_empty())
        .then(|| serde_json::to_string(&extra_networks))
        .transpose()
        .map_err(|e| ApiError::Internal(anyhow!(e)))?;

    let persist_res: ApiResult<()> = async {
        let mut tx = state.db.begin().await?;
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, server_id, name, slug, source_kind, image,
                               dockerfile_path, port, memory_mb, cpu_millis, health_path,
                               restart_policy, status, current_deployment_id, webhook_secret,
                               command, entrypoint, extra_networks, adopted_from, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'image', ?, 'Dockerfile', ?, ?, ?, ?, ?, 'running', ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&app_id)
        .bind(&auth.org_id)
        .bind(&req.project_id)
        .bind(server_id)
        .bind(&app_name)
        .bind(&slug)
        .bind(&image_ref)
        .bind(i64::from(port))
        .bind(raw.memory_mb.map(i64::from))
        .bind(raw.cpu_millis.map(i64::from))
        .bind(&health_path)
        .bind(raw.restart_policy.as_deref().unwrap_or("unless-stopped"))
        .bind(&dep_id)
        .bind(&webhook_secret_enc)
        .bind(&cmd_json)
        .bind(&ep_json)
        .bind(&nets_json)
        .bind(&raw.name)
        .bind(&at)
        .bind(&at)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO deployments (id, org_id, app_id, status, trigger, image_ref, image_id,
                                      commit_msg, container_id, host_port, created_by, created_at,
                                      started_at, finished_at)
             VALUES (?, ?, ?, 'running', 'adopt', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&dep_id)
        .bind(&auth.org_id)
        .bind(&app_id)
        .bind(&image_ref)
        .bind(&raw.image_id)
        .bind(format!("Adopted from container {}", raw.name))
        .bind(&started.id)
        .bind(i64::from(started.host_port))
        .bind(&auth.user_id)
        .bind(&at)
        .bind(&at)
        .bind(&at)
        .execute(&mut *tx)
        .await?;

        for (seq, line) in captured_logs.iter().enumerate() {
            sqlx::query(
                "INSERT INTO deployment_logs (deployment_id, seq, ts, stream, line) VALUES (?, ?, ?, 'deploy', ?)",
            )
            .bind(&dep_id)
            .bind((seq + 1) as i64)
            .bind(&at)
            .bind(line)
            .execute(&mut *tx)
            .await?;
        }

        for m in &raw.mounts {
            sqlx::query(
                "INSERT INTO app_volumes (id, app_id, name, mount_path, kind, read_only, owned, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, 0, ?)",
            )
            .bind(new_id())
            .bind(&app_id)
            .bind(&m.source)
            .bind(&m.destination)
            .bind(&m.kind)
            .bind(i64::from(m.read_only))
            .bind(&at)
            .execute(&mut *tx)
            .await?;
        }

        for (key, value, is_secret) in &live_env {
            let value_enc = state.vault.encrypt(value)?;
            sqlx::query(
                "INSERT INTO env_vars (id, org_id, app_id, key, value_enc, is_secret, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(&auth.org_id)
            .bind(&app_id)
            .bind(key)
            .bind(value_enc)
            .bind(is_secret)
            .bind(&at)
            .execute(&mut *tx)
            .await?;
        }

        for (host, tls_mode, access, dport) in &verified_domains {
            sqlx::query(
                "INSERT INTO domains (id, org_id, app_id, host, tls_mode, access, port, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(&auth.org_id)
            .bind(&app_id)
            .bind(host)
            .bind(tls_mode.as_str())
            .bind(access.as_str())
            .bind(dport.map(i64::from))
            .bind(&at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
    .await;

    if let Err(e) = persist_res {
        let _ = state.runtime.remove(&started.id).await;
        if old_paused {
            let _ = state.runtime.start(&raw.id).await;
        }
        return Err(e);
    }

    // ---- Atomic Caddy proxy cutover
    if let Err(e) = proxy::sync(state).await {
        tracing::warn!(error = %format!("{e:#}"), "proxy sync warning after adopt");
    }

    // ---- Gracefully stop (and optionally remove) the old container
    let mut old_stopped = old_paused || !old_was_running;
    let mut old_removed = false;
    if req.stop_old && !old_paused && old_was_running {
        if let Err(e) = state.runtime.stop(&raw.id, STOP_GRACE_SECS).await {
            tracing::warn!(container = %raw.id, error = %e, "failed to stop old container after adopt");
        } else {
            old_stopped = true;
        }
    }
    if req.remove_old {
        if let Err(e) = state.runtime.remove(&raw.id).await {
            tracing::warn!(container = %raw.id, error = %e, "failed to remove old container after adopt");
        } else {
            old_removed = true;
        }
    }

    auth.audit(
        state,
        "app.adopt",
        Some(&app_id),
        Some(&format!("{} <- {}", app_name, raw.name)),
    )
    .await?;

    let app = load_app(state, &auth.org_id, &app_id).await?;
    let deployment = rig_api::Deployment {
        id: dep_id,
        app_id,
        status: rig_api::DeploymentStatus::Running,
        trigger: "adopt".into(),
        image_ref: Some(image_ref.clone()),
        commit_sha: None,
        commit_msg: Some(format!("Adopted from container {}", raw.name)),
        error: None,
        created_at: at.clone(),
        started_at: Some(at.clone()),
        finished_at: Some(at),
    };

    let domains_req: Vec<AddDomainRequest> = verified_domains
        .iter()
        .map(|(h, t, a, p)| AddDomainRequest {
            host: h.clone(),
            tls_mode: *t,
            access: *a,
            port: *p,
        })
        .collect();
    let env_preview: Vec<AdoptEnvPreview> = discovered
        .env
        .iter()
        .map(|e| AdoptEnvPreview {
            key: e.key.clone(),
            is_secret: secret_set.contains(&e.key),
            secret_reason: e.secret_reason.clone(),
        })
        .collect();
    let rig_toml = render_rig_toml(
        &app_name,
        Some(&project_name),
        &image_ref,
        port,
        health_path.as_deref(),
        raw.memory_mb,
        raw.cpu_millis,
        &discovered.mounts,
        &domains_req,
    );
    let (quadlet_container, quadlet_network) = render_quadlet_units(
        &app_name,
        &project_net,
        &image_ref,
        port,
        health_path.as_deref(),
        raw.memory_mb,
        raw.cpu_millis,
        &discovered.mounts,
        &env_preview,
        raw.command.as_deref(),
        raw.restart_policy.as_deref(),
    );

    Ok(AdoptExecuteResult {
        app,
        deployment,
        old_container_id: raw.id,
        old_container_stopped: old_stopped,
        old_container_removed: old_removed,
        domains_registered: verified_domains.into_iter().map(|(h, _, _, _)| h).collect(),
        volumes_preserved: discovered.mounts,
        secrets_encrypted,
        rig_toml,
        quadlet_container,
        quadlet_network,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_traefik_caddy_and_virtual_host_domains() {
        let mut labels = HashMap::new();
        labels.insert(
            "traefik.http.routers.web.rule".into(),
            "Host(`app.example.com`) || Host(`www.example.com`)".into(),
        );
        labels.insert("traefik.http.routers.web.tls".into(), "true".into());
        labels.insert(
            "traefik.http.services.web.loadbalancer.server.port".into(),
            "3000".into(),
        );
        labels.insert("caddy".into(), "https://caddy.example.com".into());
        labels.insert("caddy.reverse_proxy".into(), "{{upstreams 8080}}".into());

        let env = vec![
            ("VIRTUAL_HOST".into(), "legacy.example.com".into()),
            ("LETSENCRYPT_HOST".into(), "legacy.example.com".into()),
        ];

        let domains = extract_domains(&labels, &env);
        let hosts: Vec<&str> = domains.iter().map(|d| d.host.as_str()).collect();
        assert!(hosts.contains(&"app.example.com"));
        assert!(hosts.contains(&"www.example.com"));
        assert!(hosts.contains(&"caddy.example.com"));
        assert!(hosts.contains(&"legacy.example.com"));
        assert_eq!(
            domains.iter().find(|d| d.host == "app.example.com").unwrap().service_port,
            Some(3000)
        );
    }

    #[test]
    fn secret_classifier_flags_sensitive_keys_and_urls() {
        assert!(classify_secret_env("DATABASE_URL", "postgres://u:p@db/app").is_some());
        assert!(classify_secret_env("STRIPE_API_KEY", "sk_live_123").is_some());
        assert!(classify_secret_env("MY_CONN", "redis://:secret@redis:6379/0").is_some());
        assert!(classify_secret_env("APP_ENV", "production").is_none());
        assert!(classify_secret_env("PORT", "8080").is_none());
    }

    #[test]
    fn detects_coolify_dokploy_compose_and_standalone_origins() {
        let coolify = HashMap::from([
            ("coolify.managed".into(), "true".into()),
            ("coolify.projectName".into(), "acme-prod".into()),
            ("com.docker.compose.service".into(), "api".into()),
        ]);
        assert_eq!(
            detect_origin(&coolify),
            ("coolify".into(), Some("acme-prod".into()), Some("api".into()))
        );

        let dokploy = HashMap::from([
            ("dokploy.appName".into(), "billing".into()),
        ]);
        assert_eq!(
            detect_origin(&dokploy),
            ("dokploy".into(), Some("billing".into()), None)
        );

        let compose = HashMap::from([
            ("com.docker.compose.project".into(), "homelab".into()),
            ("com.docker.compose.service".into(), "wiki".into()),
        ]);
        assert_eq!(
            detect_origin(&compose),
            ("compose".into(), Some("homelab".into()), Some("wiki".into()))
        );

        assert_eq!(detect_origin(&HashMap::new()), ("standalone".into(), None, None));
    }

    #[test]
    fn quadlet_and_rig_toml_generation_is_well_formed() {
        let mounts = vec![DiscoveredMount {
            kind: "bind".into(),
            source: "/srv/data".into(),
            destination: "/data".into(),
            read_only: false,
        }];
        let envs = vec![
            AdoptEnvPreview {
                key: "DB_PASSWORD".into(),
                is_secret: true,
                secret_reason: Some("key contains PASSWORD".into()),
            },
            AdoptEnvPreview {
                key: "RUST_LOG".into(),
                is_secret: false,
                secret_reason: None,
            },
        ];
        let (container, network) = render_quadlet_units(
            "Billing API",
            "rig-proj1",
            "ghcr.io/acme/billing@sha256:abc",
            8080,
            Some("/health"),
            Some(512),
            Some(1000),
            &mounts,
            &envs,
            Some(&["./server".into(), "--prod".into()]),
            Some("unless-stopped"),
        );
        assert!(container.contains("[Container]"));
        assert!(container.contains("Image=ghcr.io/acme/billing@sha256:abc"));
        assert!(container.contains("Volume=/srv/data:/data:rw"));
        assert!(container.contains("Secret=rig-billing-api-db-password,type=env,target=DB_PASSWORD"));
        assert!(container.contains("MemoryMax=512M"));
        assert!(container.contains("CPUQuota=100%"));
        assert!(network.contains("[Network]"));
        assert!(network.contains("NetworkName=rig-proj1"));
    }
}
