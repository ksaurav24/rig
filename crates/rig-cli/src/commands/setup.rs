//! `rig setup <url>` — initialize a freshly installed (or headless) Rig server
//! from the terminal without needing a browser.

use std::io::{IsTerminal, Write};

use clap::Args;
use rig_api::{DeviceLoginRequest, SetupRequest};
use rig_client::{Client, ClientOptions};
use serde_json::json;

use crate::config::{Config, Context, normalize_url};
use crate::error::{CliError, CliResult};
use crate::output::{OutputOpts, label, print_structured};

#[derive(Args)]
pub struct SetupArgs {
    /// Server address, e.g. https://rig.example.com or http://127.0.0.1:7460
    pub url: String,
    /// One-time setup token printed by `rig-server setup-token`.
    #[arg(long)]
    pub token: Option<String>,
    /// Owner display name.
    #[arg(long)]
    pub name: Option<String>,
    /// Owner email address.
    #[arg(long)]
    pub email: Option<String>,
    /// Owner password (at least 10 characters). Prefer interactive prompt.
    #[arg(long)]
    pub password: Option<String>,
    /// Organization name.
    #[arg(long)]
    pub org: Option<String>,
    /// Optional context name to save locally after setup completes.
    #[arg(long)]
    pub context_name: Option<String>,
}

fn prompt_line(label_text: &str) -> CliResult<String> {
    if !std::io::stdin().is_terminal() {
        return Err(CliError::message(format!(
            "missing required value for {label_text} in non-interactive mode"
        )));
    }
    eprint!("{label_text}: ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn prompt_secret(label_text: &str) -> CliResult<String> {
    if !std::io::stdin().is_terminal() {
        return Err(CliError::message(format!(
            "missing required value for {label_text} in non-interactive mode"
        )));
    }
    eprint!("{label_text}: ");
    std::io::stderr().flush().ok();
    console::Term::stderr()
        .read_secure_line()
        .map_err(|e| CliError::message(format!("could not read {label_text}: {e}")))
}

pub async fn run(
    config: &mut Config,
    args: SetupArgs,
    insecure: bool,
    verbose: bool,
    out: OutputOpts,
) -> CliResult<()> {
    let url = normalize_url(&args.url, insecure)?;
    let mut opts = ClientOptions::new(url.as_str());
    opts.verbose = verbose;
    opts.user_agent = concat!("rig-cli/", env!("CARGO_PKG_VERSION")).into();
    let client = Client::build(opts)?;

    let status = client.setup_status().await?;
    if status.setup_complete {
        return Err(CliError::conflict(
            "this server is already initialized; sign in with `rig login` instead",
        ));
    }

    let token = match args.token {
        Some(t) => t,
        None => prompt_secret("One-time setup token (from `rig-server setup-token`)")?,
    };
    let org_name = match args.org {
        Some(o) => o,
        None => prompt_line("Organization name")?,
    };
    let name = match args.name {
        Some(n) => n,
        None => prompt_line("Owner name")?,
    };
    let email = match args.email {
        Some(e) => e,
        None => prompt_line("Owner email")?,
    };
    let password = match args.password {
        Some(p) => p,
        None => prompt_secret("Owner password (min 10 chars)")?,
    };

    let me = client
        .complete_setup(&SetupRequest {
            token: token.trim().to_string(),
            name: name.trim().to_string(),
            email: email.trim().to_string(),
            password: password.clone(),
            org_name: org_name.trim().to_string(),
        })
        .await?;

    // Mint a device token right away so this CLI context is immediately signed in.
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "cli".into());
    let paired = client
        .device_login(&DeviceLoginRequest {
            email: email.trim().to_string(),
            password,
            code: None,
            challenge_token: None,
            device_name: format!("{hostname} · rig CLI"),
        })
        .await?;

    let mut ctx = Context::basic(url.to_string(), paired.secret);
    ctx.user = Some(me.user.email.clone());
    ctx.org = Some(me.org.name.clone());
    ctx.role = Some(label(&me.role));

    let ctx_name = args
        .context_name
        .unwrap_or_else(|| url.host_str().unwrap_or("default").to_string());
    config.contexts.insert(ctx_name.clone(), ctx);
    config.current = Some(ctx_name.clone());
    config.save()?;

    if out.is_structured() {
        return print_structured(
            &json!({
                "context": ctx_name,
                "server": url.as_str().trim_end_matches('/'),
                "user": me.user,
                "organization": me.org,
                "role": me.role,
            }),
            out.format,
        );
    }
    println!(
        "Initialized {} and signed in as {} ({}). Context saved as: {ctx_name}",
        me.org.name,
        me.user.email,
        label(&me.role)
    );
    Ok(())
}
