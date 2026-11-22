//! `rig login` and `rig logout [--revoke]`.

use std::io::{IsTerminal, Write};

use clap::Args;
use serde_json::json;

use crate::api::Client;
use crate::config::{Config, Context, normalize_url};
use crate::error::{CliError, CliResult};
use crate::output::{OutputOpts, label, print_structured};

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
    /// Also revoke this API token on the server before removing the local context.
    #[arg(long)]
    pub revoke: bool,
}

pub async fn run_login(
    config: &mut Config,
    args: LoginArgs,
    insecure: bool,
    verbose: bool,
    out: OutputOpts,
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
            console::Term::stderr()
                .read_secure_line()
                .map_err(|e| CliError::message(format!("could not read token: {e}")))?
        }
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(CliError::auth("the token is empty"));
    }
    let mut ctx = Context::basic(url.to_string(), token);
    let me = Client::with_options(ctx.clone(), verbose, !out.is_structured())?
        .me()
        .await
        .map_err(|e| match e {
            CliError::Auth(m) => CliError::auth(format!("could not sign in with that token: {m}")),
            CliError::Network(m) => {
                CliError::network(format!("could not sign in with that token: {m}"))
            }
            other => CliError::message(format!("could not sign in with that token: {other}")),
        })?;
    ctx.user = Some(me.user.email.clone());
    ctx.org = Some(me.org.name.clone());
    ctx.role = Some(label(&me.role));

    let name = match args.name {
        Some(n) => n,
        None => url.host_str().unwrap_or("default").to_string(),
    };
    config.contexts.insert(name.clone(), ctx);
    config.current = Some(name.clone());
    config.save()?;
    if out.is_structured() {
        return print_structured(
            &json!({
                "context": name,
                "server": url.as_str().trim_end_matches('/'),
                "user": me.user,
                "organization": me.org,
                "role": me.role,
            }),
            out.format,
        );
    }
    if out.is_name() {
        println!("{name}");
        return Ok(());
    }
    println!(
        "Signed in to {} as {} ({}). Context: {name}",
        me.org.name,
        me.user.email,
        label(&me.role)
    );
    Ok(())
}

pub async fn run_logout(
    config: &mut Config,
    args: LogoutArgs,
    context_flag: Option<&str>,
    verbose: bool,
    out: OutputOpts,
) -> CliResult<()> {
    let name = args
        .name
        .or_else(|| context_flag.map(str::to_owned))
        .or_else(|| config.current.clone());
    let Some(name) = name else {
        return Err(CliError::not_found("there is no context to remove"));
    };
    let mut revoked_id: Option<String> = None;
    if args.revoke {
        let ctx = config.resolve(Some(&name))?;
        let api = Client::with_options(ctx.clone(), verbose, !out.is_structured())?;
        let tokens = api.list_tokens().await?;
        let matching: Vec<_> = tokens
            .iter()
            .filter(|t| ctx.token.starts_with(&t.prefix))
            .collect();
        match matching.as_slice() {
            [tok] => {
                api.delete_token(&tok.id).await?;
                revoked_id = Some(tok.id.clone());
            }
            [] => {
                return Err(CliError::not_found(
                    "could not match this token on the server to revoke it; revoke it manually with: rig tokens ls",
                ));
            }
            _ => {
                return Err(CliError::message(
                    "multiple server tokens share this prefix; revoke the specific token with: rig tokens revoke <id>",
                ));
            }
        }
    }
    config.remove(&name)?;
    config.save()?;
    if out.is_structured() {
        return print_structured(
            &json!({
                "removed": name,
                "revoked": revoked_id.is_some(),
                "revoked_token_id": revoked_id,
            }),
            out.format,
        );
    }
    if out.is_name() {
        println!("{name}");
        return Ok(());
    }
    if let Some(id) = revoked_id {
        println!("Revoked token {id} and removed context {name}");
    } else {
        println!("Removed {name}");
    }
    Ok(())
}
