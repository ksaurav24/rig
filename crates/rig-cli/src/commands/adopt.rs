//! `rig adopt`: discover and adopt running Docker/Podman containers, Compose stacks,
//! Coolify, or Dokploy workloads into Rig with zero downtime and automatic rollback.

use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use clap::Args;
use rig_api::{
    Access, AddDomainRequest, AdoptExecuteRequest, AdoptPreview, AdoptPreviewRequest,
    DiscoveredContainer, TlsMode,
};
use serde_json::json;

use crate::{
    api::Client,
    error::{CliError, CliResult},
    ops,
    output::{OutputOpts, confirm, domain_url, print_structured, short, style_status, table},
};

#[derive(Args)]
pub struct AdoptArgs {
    /// Container names, IDs, or Compose stack names to adopt. Omit to select interactively.
    pub containers: Vec<String>,
    /// Target Rig server/VM (saved context name or server URL).
    #[arg(long, value_name = "VM")]
    pub node: Option<String>,
    /// Target Rig project (name, slug, or id) to register adopted services under.
    #[arg(long)]
    pub project: Option<String>,
    /// Override app name (only when adopting a single container).
    #[arg(long)]
    pub app: Option<String>,
    /// Override detected container port (only when adopting a single container).
    #[arg(long)]
    pub port: Option<u16>,
    /// Override HTTP health check path (e.g. `/health`).
    #[arg(long)]
    pub health_path: Option<String>,
    /// Override auto-detected domains (repeatable, e.g. `--domain app.example.com`).
    #[arg(long = "domain", value_name = "HOST")]
    pub domains: Vec<String>,
    /// Force specific environment variable keys to be stored as encrypted secrets (repeatable).
    #[arg(long = "secret", value_name = "KEY")]
    pub secret_keys: Vec<String>,
    /// Force specific environment variable keys to be stored unmasked (repeatable).
    #[arg(long = "plain", value_name = "KEY")]
    pub plain_keys: Vec<String>,
    /// Preview generated `rig.toml` and Quadlet files without modifying anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Write generated Podman Quadlet `.container` and `.network` files to this directory.
    #[arg(long, value_name = "DIR")]
    pub export_quadlets: Option<PathBuf>,
    /// Write generated `rig.toml` file(s) to this directory.
    #[arg(long, value_name = "DIR")]
    pub write_rig_toml: Option<PathBuf>,
    /// Keep the old container running alongside the adopted one after cutover.
    #[arg(long)]
    pub keep_old: bool,
    /// Permanently remove the old container after stopping it (default: keep stopped as safety net).
    #[arg(long, conflicts_with = "keep_old")]
    pub remove_old: bool,
}

pub async fn run(api: &Client, args: AdoptArgs, out: OutputOpts) -> CliResult<()> {
    let discovered = api.list_adoptable_containers().await?;
    if discovered.is_empty() {
        if out.is_structured() {
            return print_structured(&Vec::<DiscoveredContainer>::new(), out.format);
        }
        println!("No unmanaged Docker or Podman containers found on this server.");
        return Ok(());
    }

    // 1. Select target container(s).
    let selected = select_containers(&discovered, &args.containers, out)?;
    if selected.is_empty() {
        println!("No containers selected.");
        return Ok(());
    }
    if selected.len() > 1 && (args.app.is_some() || args.port.is_some()) {
        return Err(CliError::message(
            "--app and --port can only be used when adopting a single container",
        ));
    }

    // 2. Resolve target project (needed for execution; optional for dry-run preview).
    let project = resolve_target_project(api, args.project.as_deref(), args.dry_run).await?;

    let explicit_domains = (!args.domains.is_empty()).then(|| {
        args.domains
            .iter()
            .map(|host| AddDomainRequest {
                host: host.clone(),
                tls_mode: TlsMode::Auto,
                access: Access::Public,
                port: None,
            })
            .collect::<Vec<_>>()
    });
    let secret_keys = (!args.secret_keys.is_empty()).then(|| args.secret_keys.clone());
    let plain_keys = (!args.plain_keys.is_empty()).then(|| args.plain_keys.clone());

    // 3. Generate dry-run previews for every selected container BEFORE touching anything.
    let mut previews: Vec<AdoptPreview> = Vec::with_capacity(selected.len());
    for c in &selected {
        let preview = api
            .preview_adopt(&AdoptPreviewRequest {
                container_id: c.id.clone(),
                project_id: project.as_ref().map(|p| p.id.clone()),
                app_name: args.app.clone(),
                port: args.port,
                health_path: args.health_path.clone(),
                domains: explicit_domains.clone(),
                secret_keys: secret_keys.clone(),
                plain_keys: plain_keys.clone(),
            })
            .await?;
        previews.push(preview);
    }

    if let Some(dir) = &args.export_quadlets {
        for p in &previews {
            write_quadlet_files(dir, &p.app_name, &p.quadlet_container, &p.quadlet_network)?;
        }
        if !out.is_structured() {
            println!("Exported Quadlet unit previews to {}", dir.display());
        }
    }
    if let Some(dir) = &args.write_rig_toml {
        for p in &previews {
            write_rig_toml_file(dir, &p.app_name, previews.len() > 1, &p.rig_toml)?;
        }
        if !out.is_structured() {
            println!("Wrote rig.toml preview(s) to {}", dir.display());
        }
    }

    if args.dry_run {
        if out.is_structured() {
            return print_structured(&previews, out.format);
        }
        for p in &previews {
            print_preview(p);
        }
        println!("Dry run complete. Re-run without --dry-run to execute safe cutover.");
        return Ok(());
    }

    // Show full preview before asking for confirmation.
    if !out.is_structured() {
        for p in &previews {
            print_preview(p);
        }
    }

    let project = project.ok_or_else(|| {
        CliError::message(
            "specify a target project with --project <name>, or create one with: rig projects create <name>",
        )
    })?;

    confirm(
        &format!(
            "Adopt {} container(s) into project `{}` and hand off traffic?",
            previews.len(),
            project.name
        ),
        out.yes,
    )?;

    // 4. Execute safe cutover one container at a time.
    let mut results = Vec::with_capacity(previews.len());
    for p in &previews {
        if !out.is_structured() {
            eprintln!(
                "Adopting `{}` ({}) via {}...",
                p.container.name,
                short(&p.container.id),
                p.cutover_strategy
            );
        }
        let res = api
            .execute_adopt(&AdoptExecuteRequest {
                container_id: p.container.id.clone(),
                project_id: project.id.clone(),
                app_name: Some(p.app_name.clone()),
                port: Some(p.port),
                health_path: p.health_path.clone(),
                domains: explicit_domains.clone(),
                secret_keys: secret_keys.clone(),
                plain_keys: plain_keys.clone(),
                stop_old: !args.keep_old,
                remove_old: args.remove_old,
            })
            .await?;

        if let Some(dir) = &args.export_quadlets {
            write_quadlet_files(dir, &res.app.slug, &res.quadlet_container, &res.quadlet_network)?;
        }
        if let Some(dir) = &args.write_rig_toml {
            write_rig_toml_file(dir, &res.app.slug, previews.len() > 1, &res.rig_toml)?;
        }

        if !out.is_structured() {
            println!(
                "Adopted `{}` -> app `{}` (port {}, {} secret(s) encrypted, {} mount(s) preserved)",
                p.container.name,
                res.app.name,
                res.app.port,
                res.secrets_encrypted,
                res.volumes_preserved.len()
            );
            for d in &res.domains_registered {
                println!("  Live at {}", domain_url(d, false));
            }
            if res.old_container_removed {
                println!("  Old container {} removed", short(&res.old_container_id));
            } else if res.old_container_stopped {
                println!(
                    "  Old container {} gracefully stopped (kept as backup)",
                    short(&res.old_container_id)
                );
            }
        }
        results.push(res);
    }

    if out.is_structured() {
        return print_structured(&json!({ "adopted": results }), out.format);
    }
    Ok(())
}

fn select_containers<'a>(
    discovered: &'a [DiscoveredContainer],
    wanted: &[String],
    out: OutputOpts,
) -> CliResult<Vec<&'a DiscoveredContainer>> {
    if !wanted.is_empty() {
        let mut matched = Vec::new();
        for w in wanted {
            let w_trim = w.trim();
            let hits: Vec<&DiscoveredContainer> = discovered
                .iter()
                .filter(|c| {
                    c.id == w_trim
                        || c.id.starts_with(w_trim)
                        || c.name.eq_ignore_ascii_case(w_trim)
                        || c.stack
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(w_trim))
                })
                .collect();
            if hits.is_empty() {
                return Err(CliError::not_found(format!(
                    "no unmanaged container or stack matching `{w_trim}`"
                )));
            }
            for h in hits {
                if !matched.iter().any(|m: &&DiscoveredContainer| m.id == h.id) {
                    matched.push(h);
                }
            }
        }
        return Ok(matched);
    }

    // Print discovered containers table first.
    if !out.is_structured() {
        let rows: Vec<Vec<String>> = discovered
            .iter()
            .map(|c| {
                let origin = match &c.stack {
                    Some(s) => format!("{}:{s}", c.origin),
                    None => c.origin.clone(),
                };
                let port = c
                    .detected_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".into());
                let domains = if c.domains.is_empty() {
                    "-".into()
                } else {
                    c.domains
                        .iter()
                        .map(|d| d.host.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let secrets = c.env.iter().filter(|e| e.is_secret_guess).count();
                vec![
                    c.name.clone(),
                    style_status(&c.state),
                    origin,
                    c.image.clone(),
                    port,
                    domains,
                    format!("{}m / {}s", c.mounts.len(), secrets),
                ]
            })
            .collect();
        table(
            &["NAME", "STATE", "ORIGIN", "IMAGE", "PORT", "DOMAINS", "MOUNTS/SECRETS"],
            rows,
            None,
            out.wide,
        );
        println!();
    }

    if !std::io::stdin().is_terminal() {
        return Err(CliError::message(
            "pass container name(s) or ID(s) on the command line when running non-interactively",
        ));
    }

    let mut prompt = cliclack::multiselect("Select container(s) to adopt into Rig");
    for (idx, c) in discovered.iter().enumerate() {
        let hint = format!(
            "{} · {} · port {}",
            c.image,
            c.state,
            c.detected_port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".into())
        );
        prompt = prompt.item(idx, &c.name, hint);
    }
    let indices = prompt
        .interact()
        .map_err(|_| CliError::message("cancelled"))?;
    Ok(indices.into_iter().map(|i| &discovered[i]).collect())
}

async fn resolve_target_project(
    api: &Client,
    project_arg: Option<&str>,
    dry_run: bool,
) -> CliResult<Option<rig_api::Project>> {
    if let Some(p) = project_arg.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(Some(ops::projects::resolve_project(api, p).await?));
    }
    let projects = api.list_projects().await?;
    match projects.len() {
        1 => Ok(projects.into_iter().next()),
        _ if dry_run => Ok(None),
        0 => Err(CliError::message(
            "no projects exist on this server yet. Create one with: rig projects create <name>",
        )),
        _ => {
            if std::io::stdin().is_terminal() {
                let mut sel = cliclack::select("Choose the target Rig project");
                for (i, p) in projects.iter().enumerate() {
                    sel = sel.item(i, &p.name, &p.slug);
                }
                let chosen = sel.interact().map_err(|_| CliError::message("cancelled"))?;
                Ok(Some(projects[chosen].clone()))
            } else {
                let names: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();
                Err(CliError::message(format!(
                    "multiple projects exist ({}). Pass --project <name>",
                    names.join(", ")
                )))
            }
        }
    }
}

fn print_preview(p: &AdoptPreview) {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "Adopt Preview: {} -> app `{}`",
        p.container.name, p.app_name
    );
    println!("  Image:    {}", p.image);
    println!("  Port:     {}", p.port);
    println!("  Strategy: {}", p.cutover_strategy);
    if !p.domains.is_empty() {
        let hosts: Vec<&str> = p.domains.iter().map(|d| d.host.as_str()).collect();
        println!("  Domains:  {}", hosts.join(", "));
    }
    if !p.mounts.is_empty() {
        println!("  Mounts (preserved in-place):");
        for m in &p.mounts {
            let mode = if m.read_only { "ro" } else { "rw" };
            println!("    - [{}] {} -> {} ({mode})", m.kind, m.source, m.destination);
        }
    }
    let secrets: Vec<&str> = p
        .env_keys
        .iter()
        .filter(|e| e.is_secret)
        .map(|e| e.key.as_str())
        .collect();
    let plain_count = p.env_keys.len().saturating_sub(secrets.len());
    println!(
        "  Env vars: {} total ({} vault secret(s), {} plain)",
        p.env_keys.len(),
        secrets.len(),
        plain_count
    );
    if !secrets.is_empty() {
        println!("    Secrets: {}", secrets.join(", "));
    }
    if !p.warnings.is_empty() {
        println!("  Warnings:");
        for w in &p.warnings {
            println!("    ! {w}");
        }
    }
    println!("\n--- rig.toml ---\n{}", p.rig_toml.trim_end());
    println!(
        "\n--- Podman Quadlet (.container) ---\n{}",
        p.quadlet_container.trim_end()
    );
    println!(
        "\n--- Podman Quadlet (.network) ---\n{}\n",
        p.quadlet_network.trim_end()
    );
}

fn write_quadlet_files(dir: &Path, name: &str, container: &str, network: &str) -> CliResult<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating directory {}", dir.display()))?;
    let safe: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    std::fs::write(dir.join(format!("rig-{safe}.container")), container)?;
    std::fs::write(dir.join(format!("rig-{safe}.network")), network)?;
    Ok(())
}

fn write_rig_toml_file(dir: &Path, name: &str, multi: bool, content: &str) -> CliResult<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating directory {}", dir.display()))?;
    let filename = if multi {
        let safe: String = name
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        format!("rig-{safe}.toml")
    } else {
        "rig.toml".to_string()
    };
    std::fs::write(dir.join(filename), content)?;
    Ok(())
}
