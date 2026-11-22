//! Interactive installer prompt helper invoked by `packaging/install.sh`.
//!
//! Renders an arrow-key TUI via `cliclack` on `/dev/tty`, probes network and
//! Tailscale state, validates domain/TLS/access choices (including live DNS
//! verification), and writes a shell-safe plan file for `install.sh` to
//! execute. Never touches `/var/lib/rig`, SQLite, or the system itself.

use std::{
    io,
    net::{IpAddr, UdpSocket},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::routes::{apps::validate_host, system};

/// Arguments passed by `install.sh` after lightweight host/package probes.
#[derive(Args, Debug, Clone)]
pub struct WizardArgs {
    /// Path to write the selected installation plan as shell assignments.
    #[arg(long)]
    pub output: PathBuf,

    /// Distribution pretty name from /etc/os-release (e.g. "Ubuntu 24.04 LTS").
    #[arg(long, default_value = "Linux")]
    pub distro_name: String,

    /// Total host memory in MiB.
    #[arg(long)]
    pub ram_mb: Option<u64>,

    /// Free disk space for /var/lib/rig in GiB.
    #[arg(long)]
    pub disk_gb: Option<u64>,

    /// Installed Docker version, if Docker is already present.
    #[arg(long, default_value = "")]
    pub docker_installed: String,

    /// Whether the distro's package manager can install Docker automatically.
    #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
    pub docker_installable: bool,

    /// Installed Podman version, if Podman is already present.
    #[arg(long, default_value = "")]
    pub podman_installed: String,

    /// Podman version available in the distro's repositories, if any.
    #[arg(long, default_value = "")]
    pub podman_candidate: String,

    /// Pre-selected container engine from CLI flags (`podman` or `docker`).
    #[arg(long)]
    pub engine: Option<String>,

    /// Pre-selected dashboard domain/IP from CLI flags.
    #[arg(long)]
    pub domain: Option<String>,

    /// Pre-selected TLS mode from CLI flags (`auto`, `internal`, `tailscale`, `off`).
    #[arg(long)]
    pub tls: Option<String>,

    /// Pre-selected dashboard access mode from CLI flags (`public`, `lan`, `tailscale`).
    #[arg(long)]
    pub access: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailscaleState {
    Connected {
        dns_name: Option<String>,
        ips: Vec<String>,
    },
    InstalledNotConnected,
    NotInstalled,
}

impl TailscaleState {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    pub fn dns_name(&self) -> Option<&str> {
        match self {
            Self::Connected { dns_name, .. } => dns_name.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ServerIps {
    pub public_ip: Option<String>,
    pub primary_ip: Option<String>,
    pub lan_ips: Vec<String>,
}

impl ServerIps {
    /// Best address to show as this server's reachable address.
    pub fn best(&self) -> Option<&str> {
        self.public_ip
            .as_deref()
            .or(self.primary_ip.as_deref())
            .or_else(|| self.lan_ips.first().map(String::as_str))
    }

    /// All known IPv4/IPv6 addresses belonging to this server.
    pub fn known_ips(&self) -> Vec<IpAddr> {
        let mut out = Vec::new();
        for s in self
            .public_ip
            .iter()
            .chain(self.primary_ip.iter())
            .chain(self.lan_ips.iter())
        {
            if let Ok(ip) = s.parse::<IpAddr>()
                && !out.contains(&ip)
            {
                out.push(ip);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineChoice {
    Podman,
    Docker,
}

impl EngineChoice {
    fn as_str(self) -> &'static str {
        match self {
            Self::Podman => "podman",
            Self::Docker => "docker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeChoice {
    Full,
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpsChoice {
    CustomDomain,
    Tailscale,
    SelfSigned,
    PlainHttp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsNextAction {
    Recheck,
    ContinueAnyway,
    ChangeOption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessChoice {
    Public,
    Lan,
    Tailscale,
}

impl AccessChoice {
    fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Lan => "lan",
            Self::Tailscale => "tailscale",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Public => "Public (anyone with the URL)",
            Self::Lan => "LAN only (private networks)",
            Self::Tailscale => "Tailscale only (tailnet devices)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOption {
    pub id: &'static str,
    pub label: String,
    pub hint: String,
    pub available: bool,
    pub already_installed: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub engine: String,
    pub mode: String,
    pub domain: String,
    pub tls: String,
    pub access: String,
    pub public_url: String,
    pub secure_cookies: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DnsCheck {
    Matches(IpAddr),
    Mismatch(Vec<IpAddr>),
    Unresolved(String),
}

enum WizardOutcome {
    Plan(InstallPlan),
    Cancelled,
}

/// Runs the interactive wizard. Returns `Ok(true)` when a plan was confirmed
/// and written, or `Ok(false)` when the user cancelled (caller should exit 130).
pub async fn run(args: WizardArgs) -> Result<bool> {
    match run_inner(&args).await {
        Ok(WizardOutcome::Plan(plan)) => {
            let content = format_plan_file(&plan);
            std::fs::write(&args.output, content)
                .with_context(|| format!("writing install plan to {}", args.output.display()))?;
            let _ = cliclack::outro("Configuration confirmed. Starting installation...");
            Ok(true)
        }
        Ok(WizardOutcome::Cancelled) => {
            let _ = cliclack::outro_cancel("Installation cancelled.");
            Ok(false)
        }
        Err(e) => {
            if let Some(io_err) = e.downcast_ref::<io::Error>() {
                if io_err.kind() == io::ErrorKind::Interrupted {
                    let _ = cliclack::outro_cancel("Installation cancelled.");
                    return Ok(false);
                }
                if io_err.kind() == io::ErrorKind::NotConnected {
                    bail!(
                        "interactive wizard requires a terminal (/dev/tty); pass --yes with flags for non-interactive installs"
                    );
                }
            }
            Err(e)
        }
    }
}

async fn run_inner(args: &WizardArgs) -> Result<WizardOutcome> {
    let _ = cliclack::clear_screen();
    cliclack::intro(format!("Rig Installer v{}", env!("CARGO_PKG_VERSION")))?;

    // System summary line
    let mut sys_parts = vec![format!("{} ({})", args.distro_name, std::env::consts::ARCH)];
    if let Some(ram) = args.ram_mb {
        sys_parts.push(format!("{ram} MB RAM"));
    }
    if let Some(disk) = args.disk_gb {
        sys_parts.push(format!("{disk} GB free"));
    }
    cliclack::log::info(sys_parts.join(" · "))?;
    if let Some(ram) = args.ram_mb
        && ram < 900
    {
        cliclack::log::warning(format!(
            "Only {ram} MB of RAM detected; Rig and a couple of apps prefer 1 GB or more."
        ))?;
    }
    if let Some(disk) = args.disk_gb
        && disk < 5
    {
        cliclack::log::warning(format!(
            "Only {disk} GB free for /var/lib/rig; images and builds need room."
        ))?;
    }

    // Concurrent network & Tailscale probe
    let spinner = cliclack::spinner();
    spinner.start("Detecting network and Tailscale...");
    let (ts_state, server_ips) = tokio::join!(probe_tailscale(), probe_server_ips());
    let mut net_notes = Vec::new();
    if let Some(ip) = server_ips.best() {
        net_notes.push(format!("Address: {ip}"));
    }
    match &ts_state {
        TailscaleState::Connected {
            dns_name: Some(name),
            ..
        } => net_notes.push(format!("Tailscale: {name}")),
        TailscaleState::Connected { dns_name: None, .. } => {
            net_notes.push("Tailscale: connected (MagicDNS off)".into())
        }
        TailscaleState::InstalledNotConnected => {
            net_notes.push("Tailscale: installed, not connected".into())
        }
        TailscaleState::NotInstalled => {}
    }
    if net_notes.is_empty() {
        spinner.stop("Network probe complete");
    } else {
        spinner.stop(net_notes.join(" · "));
    }

    // Step 1: Container engine
    let (podman_opt, docker_opt, default_engine) = evaluate_engines(args);
    let engine = if let Some(pre) = args.engine.as_deref() {
        let choice = match pre {
            "podman" => {
                if !podman_opt.available {
                    bail!(
                        "Podman cannot be used here: {}",
                        podman_opt
                            .unavailable_reason
                            .as_deref()
                            .unwrap_or("unavailable")
                    );
                }
                EngineChoice::Podman
            }
            "docker" => {
                if !docker_opt.available {
                    bail!(
                        "Docker cannot be used here: {}",
                        docker_opt
                            .unavailable_reason
                            .as_deref()
                            .unwrap_or("unavailable")
                    );
                }
                EngineChoice::Docker
            }
            other => bail!("invalid engine '{other}' (expected podman or docker)"),
        };
        cliclack::log::step(format!("Container engine: {} (from CLI flag)", choice.as_str()))?;
        choice
    } else {
        let Some(default_engine) = default_engine else {
            let p_reason = podman_opt
                .unavailable_reason
                .as_deref()
                .unwrap_or("unavailable");
            let d_reason = docker_opt
                .unavailable_reason
                .as_deref()
                .unwrap_or("unavailable");
            bail!(
                "no supported container engine is available on this host\n  - Podman: {p_reason}\n  - Docker: {d_reason}"
            );
        };
        loop {
            let podman_hint = if podman_opt.available {
                podman_opt.hint.clone()
            } else {
                format!(
                    "unavailable — {}",
                    podman_opt
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("upgrade Podman to 4.9+ first to select this")
                )
            };
            let docker_hint = if docker_opt.available {
                docker_opt.hint.clone()
            } else {
                format!(
                    "unavailable — {}",
                    docker_opt
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("install Docker Engine first to select this")
                )
            };
            let podman_label = if podman_opt.available {
                podman_opt.label.clone()
            } else {
                format!("{} [upgrade needed]", podman_opt.label)
            };
            let docker_label = if docker_opt.available {
                docker_opt.label.clone()
            } else {
                format!("{} [not installed]", docker_opt.label)
            };
            let picked = cliclack::select("Container engine")
                .item(EngineChoice::Podman, podman_label, podman_hint)
                .item(EngineChoice::Docker, docker_label, docker_hint)
                .initial_value(default_engine)
                .interact()?;
            match picked {
                EngineChoice::Podman if !podman_opt.available => {
                    cliclack::log::warning(format!(
                        "Podman cannot be selected yet: {}.\nUpgrade Podman to 4.9+ (https://podman.io/docs/installation) and re-run install.sh, or pick Docker.",
                        podman_opt
                            .unavailable_reason
                            .as_deref()
                            .unwrap_or("Podman 4.9+ is required")
                    ))?;
                }
                EngineChoice::Docker if !docker_opt.available => {
                    cliclack::log::warning(format!(
                        "Docker cannot be selected yet: {}.\nInstall Docker Engine (https://docs.docker.com/engine/install/) and re-run install.sh, or pick Podman.",
                        docker_opt
                            .unavailable_reason
                            .as_deref()
                            .unwrap_or("Docker Engine is not installed")
                    ))?;
                }
                other => break other,
            }
        }
    };

    // Step 2: Install mode
    let mode = loop {
        let picked = cliclack::select("Install mode")
            .item(
                ModeChoice::Full,
                "Full (API + web dashboard)",
                "recommended",
            )
            .item(
                ModeChoice::Headless,
                "Headless (API & CLI only) [coming soon]",
                "not available yet",
            )
            .initial_value(ModeChoice::Full)
            .interact()?;
        match picked {
            ModeChoice::Full => break "full".to_string(),
            ModeChoice::Headless => {
                cliclack::log::warning(
                    "Headless mode is not available yet — choose Full to continue, or press Esc/Ctrl+C to cancel.",
                )?;
            }
        }
    };

    // Step 3: Dashboard address & HTTPS
    let (domain, tls) = prompt_domain_and_tls(args, &ts_state, &server_ips).await?;

    // Step 4: Dashboard access scope
    let access = prompt_access(args, &ts_state, &domain, &tls)?;

    // Build public URL & secure cookies flag
    let (public_url, secure_cookies) = compute_public_url(&domain, &tls, &server_ips);

    let engine_note = match engine {
        EngineChoice::Podman => {
            if podman_opt.already_installed && podman_version_ok(&args.podman_installed) {
                format!("Podman (rootless, installed {})", args.podman_installed)
            } else if podman_opt.already_installed {
                format!(
                    "Podman (rootless, will upgrade {} -> {})",
                    args.podman_installed.trim(),
                    args.podman_candidate.trim()
                )
            } else {
                "Podman (rootless, will install)".into()
            }
        }
        EngineChoice::Docker => {
            if docker_opt.already_installed {
                format!("Docker (installed {})", args.docker_installed)
            } else {
                "Docker (will install)".into()
            }
        }
    };
    let tls_label = match tls.as_str() {
        "auto" => "Automatic HTTPS (Let's Encrypt)",
        "tailscale" => "Tailscale HTTPS",
        "internal" => "Self-signed HTTPS (Private CA)",
        _ => "Off (plain HTTP)",
    };

    let summary = format!(
        "Engine:   {engine_note}\n\
         Mode:     Full (API + web dashboard)\n\
         Address:  {public_url}\n\
         HTTPS:    {tls_label}\n\
         Access:   {}",
        access.label()
    );
    cliclack::note("Installation summary", summary)?;

    let confirmed = cliclack::confirm("Proceed with installation?")
        .initial_value(true)
        .interact()?;
    if !confirmed {
        return Ok(WizardOutcome::Cancelled);
    }

    Ok(WizardOutcome::Plan(InstallPlan {
        engine: engine.as_str().into(),
        mode,
        domain,
        tls,
        access: access.as_str().into(),
        public_url,
        secure_cookies,
    }))
}

async fn prompt_domain_and_tls(
    args: &WizardArgs,
    ts_state: &TailscaleState,
    server_ips: &ServerIps,
) -> Result<(String, String)> {
    // Handle pre-supplied CLI flags gracefully.
    if args.domain.is_some() || args.tls.is_some() {
        let tls = match args.tls.as_deref() {
            Some(t) => t.to_string(),
            None => "auto".to_string(),
        };
        let domain = match (args.domain.as_deref(), tls.as_str()) {
            (Some(d), _) => {
                let valid = validate_wizard_host(d, &tls)?;
                if tls == "auto" {
                    run_dns_check_notice(&valid, server_ips).await?;
                }
                valid
            }
            (None, "tailscale") => {
                let Some(name) = ts_state.dns_name() else {
                    bail!(
                        "--tls tailscale requires Tailscale to be running with MagicDNS enabled (or pass --domain <name>.ts.net)"
                    );
                };
                name.to_string()
            }
            (None, "internal") => prompt_self_signed_host(server_ips)?,
            (None, "auto") => {
                // User asked for --tls auto without --domain: prompt for the domain.
                match prompt_custom_domain_with_dns(server_ips).await? {
                    Some(h) => h,
                    None => bail!("a domain name is required for --tls auto"),
                }
            }
            (None, "off") => String::new(),
            (None, other) => bail!("invalid TLS mode '{other}'"),
        };
        let display_addr = if domain.is_empty() {
            server_ips
                .best()
                .map(|ip| format!("http://{ip}"))
                .unwrap_or_else(|| "plain HTTP".into())
        } else if tls == "off" {
            format!("http://{domain}")
        } else {
            format!("https://{domain} ({tls})")
        };
        cliclack::log::step(format!("Dashboard address: {display_addr} (from CLI flag)"))?;
        return Ok((domain, tls));
    }

    loop {
        let (ts_label, ts_hint) = match ts_state {
            TailscaleState::Connected {
                dns_name: Some(name),
                ..
            } => (
                format!("Tailscale HTTPS ({name})"),
                "trusted certificate on your tailnet".to_string(),
            ),
            TailscaleState::Connected { dns_name: None, .. } => (
                "Tailscale HTTPS [MagicDNS off]".to_string(),
                "enable MagicDNS in Tailscale first to select this".to_string(),
            ),
            TailscaleState::InstalledNotConnected => (
                "Tailscale HTTPS [not connected]".to_string(),
                "run 'tailscale up' first to select this".to_string(),
            ),
            TailscaleState::NotInstalled => (
                "Tailscale HTTPS [not installed]".to_string(),
                "install Tailscale first to select this".to_string(),
            ),
        };

        let ip_hint = server_ips
            .best()
            .map(|ip| format!("https://{ip} — encrypted setup, one-time browser warning"))
            .unwrap_or_else(|| "encrypted setup without a domain, one-time browser warning".into());

        let initial = if ts_state.dns_name().is_some() {
            HttpsChoice::Tailscale
        } else {
            HttpsChoice::CustomDomain
        };
        let picked = cliclack::select("Dashboard address & HTTPS")
            .item(
                HttpsChoice::CustomDomain,
                "Custom domain with automatic HTTPS",
                "Let's Encrypt — recommended for public servers",
            )
            .item(HttpsChoice::Tailscale, ts_label, ts_hint)
            .item(
                HttpsChoice::SelfSigned,
                "Self-signed HTTPS (Private CA)",
                ip_hint,
            )
            .item(
                HttpsChoice::PlainHttp,
                "Plain HTTP",
                "not recommended — first-setup password sent unencrypted",
            )
            .initial_value(initial)
            .interact()?;

        match picked {
            HttpsChoice::CustomDomain => {
                if let Some(host) = prompt_custom_domain_with_dns(server_ips).await? {
                    return Ok((host, "auto".into()));
                }
                // User asked to pick a different address/HTTPS option; loop back.
            }
            HttpsChoice::Tailscale => match ts_state {
                TailscaleState::Connected {
                    dns_name: Some(ts_name),
                    ..
                } => return Ok((ts_name.to_string(), "tailscale".into())),
                TailscaleState::Connected { dns_name: None, .. } => {
                    cliclack::log::warning(
                        "Tailscale is connected, but MagicDNS is disabled. Enable MagicDNS in your Tailscale admin console first, or pick another option.",
                    )?;
                }
                TailscaleState::InstalledNotConnected => {
                    cliclack::log::warning(
                        "Tailscale is installed but not connected. Run 'tailscale up' first to select Tailscale HTTPS, or pick another option.",
                    )?;
                }
                TailscaleState::NotInstalled => {
                    cliclack::log::warning(
                        "Tailscale is not installed. Install Tailscale first (https://tailscale.com/download) to select this, or pick another option.",
                    )?;
                }
            },
            HttpsChoice::SelfSigned => {
                let host = prompt_self_signed_host(server_ips)?;
                cliclack::log::info(
                    "Your browser will show a one-time certificate warning; traffic and passwords stay encrypted.",
                )?;
                return Ok((host, "internal".into()));
            }
            HttpsChoice::PlainHttp => {
                let addr = server_ips.best().unwrap_or("<server-ip>");
                cliclack::log::warning(format!(
                    "Plain HTTP sends your setup password unencrypted. Alternatively, you can tunnel via:\n  ssh -L 7460:127.0.0.1:7460 <user>@{addr}  (then open http://localhost:7460)"
                ))?;
                return Ok((String::new(), "off".into()));
            }
        }
    }
}

async fn prompt_custom_domain_with_dns(server_ips: &ServerIps) -> Result<Option<String>> {
    let host: String = cliclack::input("Dashboard domain")
        .placeholder("rig.example.com")
        .validate(|input: &String| match validate_wizard_host(input, "auto") {
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        })
        .interact()?;
    let host = validate_wizard_host(&host, "auto")?;

    loop {
        let sp = cliclack::spinner();
        sp.start(format!("Checking DNS for {host}..."));
        let check = check_dns(&host, server_ips).await;
        match check {
            DnsCheck::Matches(ip) => {
                sp.stop(format!("{host} resolves to {ip} (matches this server)"));
                return Ok(Some(host));
            }
            DnsCheck::Mismatch(ips) => {
                let found = ips
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let expected = server_ips.best().unwrap_or("this server's IP");
                sp.error(format!(
                    "{host} resolves to {found}, expected {expected}"
                ));
            }
            DnsCheck::Unresolved(reason) => {
                let expected = server_ips.best().unwrap_or("this server's IP");
                sp.error(format!(
                    "{host} does not resolve yet ({reason}); add an A record pointing to {expected}"
                ));
            }
        }

        let action = cliclack::select("DNS does not point to this server yet. What next?")
            .item(
                DnsNextAction::Recheck,
                "Re-check DNS",
                "after adding or updating your A record",
            )
            .item(
                DnsNextAction::ContinueAnyway,
                "Continue anyway",
                "Caddy will retry once DNS propagates (undo later with dashboard-domain --clear)",
            )
            .item(
                DnsNextAction::ChangeOption,
                "Choose a different HTTPS option",
                "go back to the address menu",
            )
            .initial_value(DnsNextAction::Recheck)
            .interact()?;

        match action {
            DnsNextAction::Recheck => continue,
            DnsNextAction::ContinueAnyway => return Ok(Some(host)),
            DnsNextAction::ChangeOption => return Ok(None),
        }
    }
}

async fn run_dns_check_notice(host: &str, server_ips: &ServerIps) -> Result<()> {
    let sp = cliclack::spinner();
    sp.start(format!("Checking DNS for {host}..."));
    match check_dns(host, server_ips).await {
        DnsCheck::Matches(ip) => {
            sp.stop(format!("{host} resolves to {ip} (matches this server)"));
        }
        DnsCheck::Mismatch(ips) => {
            let found = ips
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            sp.error(format!(
                "{host} resolves to {found} (may not match this server; continuing as requested)"
            ));
        }
        DnsCheck::Unresolved(reason) => {
            sp.error(format!(
                "{host} did not resolve ({reason}; continuing as requested)"
            ));
        }
    }
    Ok(())
}

fn prompt_self_signed_host(server_ips: &ServerIps) -> Result<String> {
    let default_host = server_ips.best().unwrap_or("rig.local");
    let raw: String = cliclack::input("IP address or local hostname for the certificate")
        .default_input(default_host)
        .validate(|input: &String| match validate_wizard_host(input, "internal") {
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        })
        .interact()?;
    validate_wizard_host(&raw, "internal")
}

fn prompt_access(
    args: &WizardArgs,
    ts_state: &TailscaleState,
    domain: &str,
    tls: &str,
) -> Result<AccessChoice> {
    if let Some(pre) = args.access.as_deref() {
        let choice = match pre {
            "public" => AccessChoice::Public,
            "lan" => AccessChoice::Lan,
            "tailscale" => {
                if !ts_state.is_connected() {
                    bail!(
                        "--access tailscale cannot be set because Tailscale is not running on this server"
                    );
                }
                AccessChoice::Tailscale
            }
            other => bail!("invalid access mode '{other}' (expected public, lan or tailscale)"),
        };
        cliclack::log::step(format!("Dashboard access: {} (from CLI flag)", choice.as_str()))?;
        return Ok(choice);
    }

    let default_access = default_access_mode(tls, domain, ts_state.is_connected());
    let (ts_access_label, ts_access_hint) = match ts_state {
        TailscaleState::Connected { .. } => ("Tailscale only", "devices on your tailnet only"),
        TailscaleState::InstalledNotConnected => (
            "Tailscale only [not connected]",
            "run 'tailscale up' first to select this",
        ),
        TailscaleState::NotInstalled => (
            "Tailscale only [not installed]",
            "install Tailscale first to select this",
        ),
    };
    loop {
        let picked = cliclack::select("Who can reach the dashboard?")
            .item(AccessChoice::Public, "Public", "anyone with the URL")
            .item(AccessChoice::Lan, "LAN only", "private network ranges only")
            .item(AccessChoice::Tailscale, ts_access_label, ts_access_hint)
            .initial_value(default_access)
            .interact()?;
        if picked == AccessChoice::Tailscale && !ts_state.is_connected() {
            match ts_state {
                TailscaleState::InstalledNotConnected => {
                    cliclack::log::warning(
                        "Tailscale is installed but not connected. Run 'tailscale up' first to restrict access to Tailscale.",
                    )?;
                }
                TailscaleState::NotInstalled => {
                    cliclack::log::warning(
                        "Tailscale is not installed. Install Tailscale first (https://tailscale.com/download) to select this.",
                    )?;
                }
                TailscaleState::Connected { .. } => unreachable!(),
            }
            continue;
        }
        return Ok(picked);
    }
}

// ----------------------------------------------------------------- pure logic

/// True when a Debian/RPM/semver Podman version string is 4.9 or newer.
pub fn podman_version_ok(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() {
        return false;
    }
    // Strip optional epoch ("2:4.9.3-1") and distro suffixes ("4.9.3+ds1-1").
    let without_epoch = s.rsplit_once(':').map_or(s, |(_, rest)| rest);
    let base = without_epoch
        .split(['-', '+', '~'])
        .next()
        .unwrap_or_default();
    let mut parts = base.split('.');
    let Some(major) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    (major, minor) >= (4, 9)
}

/// True when `host` parses as an IPv4 or IPv6 literal.
pub fn is_ip_literal(host: &str) -> bool {
    host.trim().parse::<IpAddr>().is_ok()
}

/// True when `host` is an RFC1918 / loopback / link-local / ULA IP literal.
pub fn is_private_ip(host: &str) -> bool {
    match host.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        Ok(IpAddr::V6(v6)) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// Validates a dashboard host according to the chosen TLS mode.
pub fn validate_wizard_host(host: &str, tls: &str) -> Result<String> {
    let h = validate_host(host).map_err(|e| anyhow::anyhow!("{e}"))?;
    if tls == "auto" && is_ip_literal(&h) {
        bail!(
            "automatic HTTPS (Let's Encrypt) requires a domain name, not an IP address (choose Self-signed HTTPS for an IP)"
        );
    }
    Ok(h)
}

fn evaluate_engines(args: &WizardArgs) -> (EngineOption, EngineOption, Option<EngineChoice>) {
    let podman_installed = !args.podman_installed.trim().is_empty();
    let docker_installed = !args.docker_installed.trim().is_empty();

    let podman = if podman_installed {
        if podman_version_ok(&args.podman_installed) {
            EngineOption {
                id: "podman",
                label: "Podman (rootless)".into(),
                hint: format!("installed v{}", args.podman_installed.trim()),
                available: true,
                already_installed: true,
                unavailable_reason: None,
            }
        } else if podman_version_ok(&args.podman_candidate) {
            EngineOption {
                id: "podman",
                label: "Podman (rootless)".into(),
                hint: format!(
                    "installed v{} — will upgrade to v{} from distro packages",
                    args.podman_installed.trim(),
                    args.podman_candidate.trim()
                ),
                available: true,
                already_installed: true,
                unavailable_reason: None,
            }
        } else {
            EngineOption {
                id: "podman",
                label: "Podman (rootless)".into(),
                hint: String::new(),
                available: false,
                already_installed: true,
                unavailable_reason: Some(format!(
                    "installed v{}, but Rig requires 4.9+ — upgrade Podman first to select this",
                    args.podman_installed.trim()
                )),
            }
        }
    } else if podman_version_ok(&args.podman_candidate) {
        EngineOption {
            id: "podman",
            label: "Podman (rootless)".into(),
            hint: format!(
                "will install v{} — recommended",
                args.podman_candidate.trim()
            ),
            available: true,
            already_installed: false,
            unavailable_reason: None,
        }
    } else {
        let reason = if args.podman_candidate.trim().is_empty() {
            format!(
                "no Podman 4.9+ package on {} — install/upgrade Podman 4.9+ first to select this",
                args.distro_name
            )
        } else {
            format!(
                "{} ships v{}, Rig needs 4.9+ — upgrade Podman first to select this",
                args.distro_name,
                args.podman_candidate.trim()
            )
        };
        EngineOption {
            id: "podman",
            label: "Podman (rootless)".into(),
            hint: String::new(),
            available: false,
            already_installed: false,
            unavailable_reason: Some(reason),
        }
    };

    let docker = if docker_installed {
        EngineOption {
            id: "docker",
            label: "Docker".into(),
            hint: format!("installed v{} — recommended", args.docker_installed.trim()),
            available: true,
            already_installed: true,
            unavailable_reason: None,
        }
    } else if args.docker_installable {
        let hint = if !podman.available {
            "will install from distro packages — recommended here".into()
        } else {
            "will install from distro packages".into()
        };
        EngineOption {
            id: "docker",
            label: "Docker".into(),
            hint,
            available: true,
            already_installed: false,
            unavailable_reason: None,
        }
    } else {
        EngineOption {
            id: "docker",
            label: "Docker".into(),
            hint: String::new(),
            available: false,
            already_installed: false,
            unavailable_reason: Some(format!(
                "not installed and not in {}'s default repos (install Docker Engine first to use Docker)",
                args.distro_name
            )),
        }
    };

    let default = if docker.available && docker.already_installed {
        Some(EngineChoice::Docker)
    } else if podman.available && podman.already_installed {
        Some(EngineChoice::Podman)
    } else if podman.available {
        Some(EngineChoice::Podman)
    } else if docker.available {
        Some(EngineChoice::Docker)
    } else {
        None
    };

    (podman, docker, default)
}

fn default_access_mode(tls: &str, domain: &str, tailscale_connected: bool) -> AccessChoice {
    if tls == "tailscale" && tailscale_connected {
        AccessChoice::Tailscale
    } else if tls == "internal" && is_private_ip(domain) {
        AccessChoice::Lan
    } else {
        AccessChoice::Public
    }
}

fn compute_public_url(domain: &str, tls: &str, server_ips: &ServerIps) -> (String, bool) {
    let secure = tls != "off";
    if !domain.is_empty() {
        let scheme = if secure { "https" } else { "http" };
        (format!("{scheme}://{domain}"), secure)
    } else {
        let addr = server_ips.best().unwrap_or("<this-server>");
        (format!("http://{addr}"), false)
    }
}

/// Serializes an [`InstallPlan`] as safely single-quoted shell assignments.
pub fn format_plan_file(plan: &InstallPlan) -> String {
    fn q(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
    format!(
        "PLAN_ENGINE={}\n\
         PLAN_MODE={}\n\
         PLAN_DOMAIN={}\n\
         PLAN_TLS={}\n\
         PLAN_ACCESS={}\n\
         PLAN_PUBLIC_URL={}\n\
         PLAN_SECURE_COOKIES={}\n",
        q(&plan.engine),
        q(&plan.mode),
        q(&plan.domain),
        q(&plan.tls),
        q(&plan.access),
        q(&plan.public_url),
        q(if plan.secure_cookies { "true" } else { "false" }),
    )
}

// -------------------------------------------------------------------- probes

async fn probe_tailscale() -> TailscaleState {
    if let Some(info) = system::tailscale_info().await {
        return TailscaleState::Connected {
            dns_name: info.dns_name,
            ips: info.addresses,
        };
    }
    let installed = tokio::task::spawn_blocking(tailscale_binary_exists)
        .await
        .unwrap_or(false);
    if installed {
        TailscaleState::InstalledNotConnected
    } else {
        TailscaleState::NotInstalled
    }
}

fn tailscale_binary_exists() -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join("tailscale").is_file() {
                return true;
            }
        }
    }
    std::path::Path::new("/usr/bin/tailscale").is_file()
        || std::path::Path::new("/usr/local/bin/tailscale").is_file()
}

async fn probe_server_ips() -> ServerIps {
    let local = tokio::task::spawn_blocking(|| {
        let primary = primary_ipv4();
        let lan = system::lan_addresses();
        (primary, lan)
    });
    let external = detect_public_ipv4();
    let (local, public_ip) = tokio::join!(local, external);
    let (primary_ip, lan_ips) = local.unwrap_or_default();
    ServerIps {
        public_ip,
        primary_ip,
        lan_ips,
    }
}

fn primary_ipv4() -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("1.1.1.1:80").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4.to_string()),
        _ => None,
    }
}

async fn detect_public_ipv4() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .connect_timeout(Duration::from_secs(1))
        .build()
        .ok()?;
    for url in ["https://api.ipify.org", "https://ifconfig.me/ip"] {
        if let Ok(resp) = client.get(url).send().await
            && resp.status().is_success()
            && let Ok(text) = resp.text().await
        {
            let trimmed = text.trim();
            if let Ok(IpAddr::V4(v4)) = trimmed.parse::<IpAddr>() {
                return Some(v4.to_string());
            }
        }
    }
    None
}

async fn check_dns(host: &str, server_ips: &ServerIps) -> DnsCheck {
    let res = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, 80)),
    )
    .await;
    let addrs = match res {
        Ok(Ok(iter)) => {
            let mut ips: Vec<IpAddr> = iter.map(|sa| sa.ip()).collect();
            ips.sort();
            ips.dedup();
            ips
        }
        Ok(Err(e)) => return DnsCheck::Unresolved(e.to_string()),
        Err(_) => return DnsCheck::Unresolved("DNS lookup timed out".into()),
    };
    if addrs.is_empty() {
        return DnsCheck::Unresolved("no A/AAAA records returned".into());
    }
    let known = server_ips.known_ips();
    if let Some(matched) = addrs.iter().find(|ip| known.contains(ip)) {
        DnsCheck::Matches(*matched)
    } else {
        DnsCheck::Mismatch(addrs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> WizardArgs {
        WizardArgs {
            output: PathBuf::from("/tmp/plan"),
            distro_name: "Ubuntu 24.04 LTS".into(),
            ram_mb: Some(2048),
            disk_gb: Some(20),
            docker_installed: String::new(),
            docker_installable: true,
            podman_installed: String::new(),
            podman_candidate: "4.9.3+ds1-1ubuntu0.2".into(),
            engine: None,
            domain: None,
            tls: None,
            access: None,
        }
    }

    #[test]
    fn podman_version_checks_debian_rpm_and_semver() {
        assert!(podman_version_ok("4.9"));
        assert!(podman_version_ok("4.9.3+ds1-1ubuntu0.2"));
        assert!(podman_version_ok("2:4.9.4-1.el9"));
        assert!(podman_version_ok("5.0.3"));
        assert!(podman_version_ok("5.4.1+ds1-1"));
        assert!(!podman_version_ok(""));
        assert!(!podman_version_ok("4.3.1+ds1-8+deb12u1"));
        assert!(!podman_version_ok("4.8.9"));
        assert!(!podman_version_ok("3.4.4"));
        assert!(!podman_version_ok("garbage"));
    }

    #[test]
    fn engine_evaluation_picks_best_default() {
        // Fresh Ubuntu 24.04 -> Podman recommended
        let (p, d, def) = evaluate_engines(&base_args());
        assert!(p.available && d.available);
        assert_eq!(def, Some(EngineChoice::Podman));

        // Fresh Debian 12 (Podman 4.3) -> Podman unavailable, Docker default
        let mut deb12 = base_args();
        deb12.distro_name = "Debian GNU/Linux 12".into();
        deb12.podman_candidate = "4.3.1+ds1-8".into();
        let (p, d, def) = evaluate_engines(&deb12);
        assert!(!p.available);
        assert!(p.unavailable_reason.unwrap().contains("4.3.1"));
        assert!(d.available);
        assert_eq!(def, Some(EngineChoice::Docker));

        // Existing Docker install -> Docker default even if Podman 4.9+ is available
        let mut has_docker = base_args();
        has_docker.docker_installed = "27.3.1".into();
        let (_, d, def) = evaluate_engines(&has_docker);
        assert!(d.already_installed);
        assert_eq!(def, Some(EngineChoice::Docker));

        // RHEL 9 without Docker repo -> Podman only
        let mut rhel = base_args();
        rhel.distro_name = "Rocky Linux 9".into();
        rhel.docker_installable = false;
        rhel.podman_candidate = "5.2.2".into();
        let (p, d, def) = evaluate_engines(&rhel);
        assert!(p.available && !d.available);
        assert_eq!(def, Some(EngineChoice::Podman));

        // Old Podman installed, distro has 4.9+ upgrade candidate -> available with upgrade hint
        let mut old_upgradeable = base_args();
        old_upgradeable.podman_installed = "4.3.1".into();
        old_upgradeable.podman_candidate = "4.9.3+ds1-1".into();
        let (p, _, def) = evaluate_engines(&old_upgradeable);
        assert!(p.available);
        assert!(p.hint.contains("will upgrade"));
        assert_eq!(def, Some(EngineChoice::Podman));

        // Old Podman installed, no 4.9+ candidate -> disabled with upgrade guidance
        let mut old_stuck = base_args();
        old_stuck.podman_installed = "4.3.1".into();
        old_stuck.podman_candidate = "4.3.1+ds1-8".into();
        let (p, d, def) = evaluate_engines(&old_stuck);
        assert!(!p.available);
        assert!(p.unavailable_reason.unwrap().contains("upgrade Podman"));
        assert!(d.available);
        assert_eq!(def, Some(EngineChoice::Docker));
    }

    #[test]
    fn host_validation_rejects_ip_for_auto_tls_only() {
        assert_eq!(
            validate_wizard_host("Rig.Example.COM.", "auto").unwrap(),
            "rig.example.com"
        );
        assert!(validate_wizard_host("203.0.113.10", "auto").is_err());
        assert_eq!(
            validate_wizard_host("203.0.113.10", "internal").unwrap(),
            "203.0.113.10"
        );
        assert!(is_ip_literal("203.0.113.10"));
        assert!(!is_ip_literal("rig.example.com"));
        assert!(is_private_ip("192.168.1.50"));
        assert!(is_private_ip("10.0.0.5"));
        assert!(!is_private_ip("203.0.113.10"));
    }

    #[test]
    fn default_access_matches_tls_and_network() {
        assert_eq!(
            default_access_mode("tailscale", "box.tail.ts.net", true),
            AccessChoice::Tailscale
        );
        assert_eq!(
            default_access_mode("internal", "192.168.1.10", false),
            AccessChoice::Lan
        );
        assert_eq!(
            default_access_mode("internal", "203.0.113.10", false),
            AccessChoice::Public
        );
        assert_eq!(
            default_access_mode("auto", "rig.example.com", true),
            AccessChoice::Public
        );
    }

    #[test]
    fn plan_file_quotes_safely() {
        let plan = InstallPlan {
            engine: "podman".into(),
            mode: "full".into(),
            domain: "rig.example.com".into(),
            tls: "auto".into(),
            access: "public".into(),
            public_url: "https://rig.example.com".into(),
            secure_cookies: true,
        };
        let out = format_plan_file(&plan);
        assert!(out.contains("PLAN_ENGINE='podman'\n"));
        assert!(out.contains("PLAN_DOMAIN='rig.example.com'\n"));
        assert!(out.contains("PLAN_TLS='auto'\n"));
        assert!(out.contains("PLAN_ACCESS='public'\n"));
        assert!(out.contains("PLAN_PUBLIC_URL='https://rig.example.com'\n"));
        assert!(out.contains("PLAN_SECURE_COOKIES='true'\n"));
    }
}
