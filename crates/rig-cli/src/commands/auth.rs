//! `rig login` and `rig logout`.

use std::io::{IsTerminal, Write};

use clap::Args;
use serde_json::json;

use crate::api::Client;
use crate::config::{Config, Context, normalize_url};
use crate::error::{CliError, CliResult};
use crate::output::{label, print_json};

#[derive(Args)]
pub struct LoginArgs {
    /// Server address, e.g. https://rig.example.com
    pub url: String,
    /// Name for this context. Defaults to the server's host name.
    #[arg(long)]
    pub name: Option<String>,
    /// API token. Prefer RIG_TOKEN or the interactive prompt, which keep it out of shell history.
    #[arg(long, env = "RIG_TOKEN", hide_env_values = true)]
    pub token: Option<String>,
}

#[derive(Args)]
pub struct LogoutArgs {
    /// Context name to remove. Defaults to the current context.
    pub name: Option<String>,
}

pub async fn run_login(
    config: &mut Config,
    args: LoginArgs,
    insecure: bool,
    json_out: bool,
) -> CliResult<()> {
    let url = normalize_url(&args.url, insecure)?;
    let token = match args.token {
        Some(t) => t,
        None => {
            if !std::io::stdin().is_terminal() {
                return Err(CliError::auth(
                    "no token given. Set RIG_TOKEN, or run this in a terminal to be asked for it",
                ));
            }
            eprint!("API token (create one in the dashboard under Settings → API tokens): ");
            std::io::stderr().flush().ok();
            // Read without echoing characters to the terminal (B1).
            console::Term::stderr()
                .read_secure_line()
                .map_err(|e| CliError::message(format!("could not read token: {e}")))?
        }
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(CliError::auth("the token is empty"));
    }
    let ctx = Context {
        url: url.to_string(),
        token,
    };
    // Prove the pair works before saving anything.
    let me = Client::new(ctx.clone())?
        .me()
        .await
        .map_err(|e| match e {
            CliError::Auth(m) => CliError::auth(format!("could not sign in with that token: {m}")),
            CliError::Network(m) => {
                CliError::network(format!("could not sign in with that token: {m}"))
            }
            other => CliError::message(format!("could not sign in with that token: {other}")),
        })?;
    let name = match args.name {
        Some(n) => n,
        None => url.host_str().unwrap_or("default").to_string(),
    };
    config.contexts.insert(name.clone(), ctx);
    config.current = Some(name.clone());
    config.save()?;
    if json_out {
        return print_json(&json!({
            "context": name,
            "server": url.as_str().trim_end_matches('/'),
            "user": me.user,
            "organization": me.org,
            "role": me.role,
        }));
    }
    println!(
        "Signed in to {} as {} ({}). Context: {name}",
        me.org.name,
        me.user.email,
        label(&me.role)
    );
    Ok(())
}

pub fn run_logout(
    config: &mut Config,
    args: LogoutArgs,
    context_flag: Option<&str>,
    json_out: bool,
) -> CliResult<()> {
    let name = args
        .name
        .or_else(|| context_flag.map(str::to_owned))
        .or_else(|| config.current.clone());
    let Some(name) = name else {
        return Err(CliError::not_found("there is no context to remove"));
    };
    config.remove(&name)?;
    config.save()?;
    if json_out {
        return print_json(&json!({ "removed": name }));
    }
    println!("Removed {name}");
    Ok(())
}
