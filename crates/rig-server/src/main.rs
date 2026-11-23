mod access;
mod alerts;
mod auth;
mod backups;
mod crypto;
mod dashboard;
mod databases;
mod deploy;
mod error;
mod github;
mod metrics;
mod proxy;
mod railpack;
mod routes;
mod runtime;
mod setup;
mod state;
mod totp;
mod watch;
mod wizard;

use std::{
    io::IsTerminal,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::state::{AppState, Config};

#[derive(Parser)]
#[command(
    name = "rig-server",
    version,
    about = "Rig: a lightweight self-hosted deployment platform"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Directory for the database, master key, build cache and backups.
    #[arg(long, env = "RIG_DATA_DIR", default_value = "./data", global = true)]
    data_dir: PathBuf,

    /// Address the API and dashboard listen on.
    #[arg(
        long,
        env = "RIG_LISTEN",
        default_value = "127.0.0.1:7460",
        global = true
    )]
    listen: SocketAddr,

    /// Mark the session cookie Secure. Turn on when serving over HTTPS.
    #[arg(long, env = "RIG_SECURE_COOKIES", global = true)]
    secure_cookies: bool,

    /// Container engine socket. Defaults to the Docker socket, then Podman's.
    #[arg(long, env = "RIG_ENGINE_SOCKET", global = true)]
    engine_socket: Option<String>,

    /// Caddy admin endpoint.
    #[arg(
        long,
        env = "RIG_CADDY_ADMIN",
        default_value = "http://127.0.0.1:2019",
        global = true
    )]
    caddy_admin: url::Url,

    /// Port Caddy serves plain HTTP on.
    #[arg(long, env = "RIG_HTTP_PORT", default_value_t = 80, global = true)]
    http_port: u16,

    /// Port Caddy serves HTTPS on.
    #[arg(long, env = "RIG_HTTPS_PORT", default_value_t = 443, global = true)]
    https_port: u16,

    /// GitHub API base URL. Change it for GitHub Enterprise Server.
    #[arg(
        long,
        env = "RIG_GITHUB_API",
        default_value = "https://api.github.com",
        global = true
    )]
    github_api: url::Url,

    /// GitHub web base URL, where the App manifest is created and repos are cloned from.
    #[arg(
        long,
        env = "RIG_GITHUB_WEB",
        default_value = "https://github.com",
        global = true
    )]
    github_web: url::Url,

    /// Seconds between checks of each auto-deploy app's branch for new
    /// commits, for servers GitHub cannot reach. 0 turns polling off.
    #[arg(long, env = "RIG_POLL_INTERVAL", default_value_t = 120, global = true)]
    poll_interval: u64,

    /// Serve the dashboard through the proxy: on its domain when one is
    /// set, otherwise on the HTTP port at any address of this machine.
    #[arg(long, env = "RIG_PROXY_DASHBOARD", global = true)]
    proxy_dashboard: bool,

    /// The address people open Rig on, used in the setup message.
    #[arg(long, env = "RIG_PUBLIC_URL", global = true)]
    public_url: Option<String>,

    /// Allow alert webhooks to connect to loopback, private, or local addresses.
    #[arg(long, env = "RIG_ALLOW_PRIVATE_ALERTS", global = true)]
    allow_private_alerts: bool,

    /// Allow Git clones and branch polling to connect to loopback, private, or local addresses.
    #[arg(long, env = "RIG_ALLOW_PRIVATE_GIT", global = true)]
    allow_private_git: bool,

    /// Serve interactive Scalar API docs at `/api/docs` (authenticated).
    #[arg(long, env = "RIG_ENABLE_API_DOCS", global = true)]
    enable_api_docs: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (default).
    Serve,
    /// Print the OpenAPI document as JSON and exit.
    Openapi,
    /// Turn off two-factor sign-in for a user who lost their authenticator
    /// and recovery codes. Run it on the server, as the user Rig runs as.
    Reset2fa {
        /// The user's email address.
        email: String,
    },
    /// Generate a new random password for a user and sign them out everywhere.
    /// Run it on the server, as the user Rig runs as.
    ResetPassword {
        /// The user's email address.
        email: String,
    },
    /// Interactive install wizard invoked by install.sh.
    #[command(hide = true)]
    InstallWizard(wizard::WizardArgs),
    /// Set who may open the dashboard: public, lan or tailscale. Use
    /// `public` to get back in after locking yourself out. Takes effect on
    /// the next start.
    DashboardAccess {
        #[arg(value_parser = ["public", "lan", "tailscale"])]
        mode: String,
    },
    /// Print a fresh one-time setup token (valid 30 minutes) while Rig has
    /// no owner yet. Works while the server is running.
    SetupToken,
    /// Serve the dashboard on a domain of its own, or `--clear` to serve it
    /// on the machine's addresses over plain HTTP again. Applied within a
    /// minute by a running server.
    DashboardDomain {
        /// The hostname, e.g. rig.example.com.
        #[arg(required_unless_present = "clear")]
        host: Option<String>,
        /// auto (public certificate), internal, tailscale or off.
        #[arg(long, default_value = "auto", value_parser = ["auto", "internal", "tailscale", "off"])]
        tls: String,
        #[arg(long, conflicts_with = "host")]
        clear: bool,
    },
    /// Remove the dashboard route from the proxy while keeping every running
    /// application route active. Invoked by `install.sh --uninstall`.
    #[command(hide = true)]
    ProxyDetachDashboard,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Openapi)) {
        println!("{}", serde_json::to_string_pretty(&routes::openapi())?);
        return Ok(());
    }
    if let Some(Command::InstallWizard(args)) = cli.command {
        if !wizard::run(args).await? {
            std::process::exit(130);
        }
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=warn,sqlx=warn".into()),
        )
        .init();

    std::fs::create_dir_all(&cli.data_dir)
        .with_context(|| format!("creating data directory {}", cli.data_dir.display()))?;
    std::fs::set_permissions(&cli.data_dir, std::fs::Permissions::from_mode(0o700))?;

    let db_path = cli.data_dir.join("rig.db");
    let db = state::open_db(&format!("sqlite://{}", db_path.display())).await?;
    let vault = crypto::Vault::open(&cli.data_dir)?;
    if let Some(Command::Reset2fa { email }) = &cli.command {
        return reset_2fa(&db, email).await;
    }
    if let Some(Command::ResetPassword { email }) = &cli.command {
        let password = reset_password(&db, email).await?;
        let normalized = email.trim().to_lowercase();
        println!(
            "\n  Password reset for {normalized}.\n  All active sessions and API tokens have been revoked.\n\n  New temporary password (valid {} days):\n\n      {password}\n\n  Sign in with this password; you will be asked to choose a new password.\n",
            routes::members::TEMP_PASSWORD_TTL_DAYS
        );
        return Ok(());
    }
    if let Some(Command::SetupToken) = &cli.command {
        match setup::issue_token_if_needed(&db).await? {
            Some(token) => println!("{token}"),
            None => {
                eprintln!("Rig is already set up; sign in instead.");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if let Some(Command::DashboardDomain { host, tls, clear }) = &cli.command {
        let host = if *clear {
            None
        } else {
            let h = host.as_deref().unwrap_or_default();
            Some(wizard::validate_wizard_host(h, tls)?)
        };
        proxy::store_dashboard_domain(&db, host.as_deref(), proxy::parse_tls(tls))
            .await?;
        match host {
            Some(h) => println!("The dashboard will be served on {h}."),
            None => println!("The dashboard will be served on this machine's addresses over HTTP."),
        }
        return Ok(());
    }
    if let Some(Command::DashboardAccess { mode }) = &cli.command {
        access::store(&db, access::parse(mode)).await?;
        println!("Dashboard access is now {mode}. Restart Rig for it to take effect.");
        return Ok(());
    }
    if matches!(cli.command, Some(Command::ProxyDetachDashboard)) {
        let state = AppState {
            db,
            vault,
            runtime: Arc::new(runtime::docker::DockerRuntime::connect(
                cli.engine_socket.as_deref(),
            )?),
            proxy: Arc::new(proxy::CaddyProxy::new(proxy::CaddyOptions {
                admin: cli.caddy_admin,
                http_port: cli.http_port,
                https_port: cli.https_port,
                dashboard_port: None,
            })?),
            deploys: Arc::new(deploy::Deployer::new()?),
            access: access::DashboardAccess::default(),
            config: Arc::new(Config {
                data_dir: cli.data_dir,
                secure_cookies: cli.secure_cookies,
                github_api: trim_slash(&cli.github_api),
                github_web: trim_slash(&cli.github_web),
                poll_interval: cli.poll_interval,
                allow_private_alerts: cli.allow_private_alerts,
                allow_private_git: cli.allow_private_git,
                enable_api_docs: cli.enable_api_docs,
            }),
        };
        proxy::sync(&state).await?;
        return Ok(());
    }
    crypto::migrate_plaintext_secrets(&db, &vault).await?;
    auth::prune_expired(&db).await;

    let dashboard_access = access::DashboardAccess::default();
    dashboard_access.set(access::load(&db).await?);
    dashboard_access.set_blocked_bridges(
        tokio::task::spawn_blocking(access::container_bridge_cidrs)
            .await
            .unwrap_or_default(),
    );
    let state = AppState {
        db,
        vault,
        runtime: Arc::new(runtime::docker::DockerRuntime::connect(
            cli.engine_socket.as_deref(),
        )?),
        proxy: Arc::new(proxy::CaddyProxy::new(proxy::CaddyOptions {
            admin: cli.caddy_admin,
            http_port: cli.http_port,
            https_port: cli.https_port,
            dashboard_port: cli.proxy_dashboard.then(|| cli.listen.port()),
        })?),
        deploys: Arc::new(deploy::Deployer::new()?),
        access: dashboard_access,
        config: Arc::new(Config {
            data_dir: cli.data_dir,
            secure_cookies: cli.secure_cookies,
            github_api: trim_slash(&cli.github_api),
            github_web: trim_slash(&cli.github_web),
            poll_interval: cli.poll_interval,
            allow_private_alerts: cli.allow_private_alerts,
            allow_private_git: cli.allow_private_git,
            enable_api_docs: cli.enable_api_docs,
        }),
    };

    if let Some(token) = setup::issue_token_if_needed(&state.db).await? {
        let url = cli
            .public_url
            .clone()
            .unwrap_or_else(|| format!("http://{}", cli.listen));
        // Only print the secret setup token to an interactive terminal so
        // systemd never captures it into journald.
        if std::io::stderr().is_terminal() {
            eprintln!(
                "\n  Rig is not set up yet.\n  Open {url} and enter this one-time setup token \
                 (valid 30 minutes; `rig-server setup-token` prints a new one):\n\n      {token}\n",
            );
        } else {
            tracing::info!(
                %url,
                "Rig is not set up yet; run `rig-server setup-token` on the server to print a one-time setup token"
            );
        }
    }
    let _ = setup::dummy_hash();
    databases::recover(&state).await?;
    backups::recover(&state).await?;
    tokio::spawn(deploy::worker(state.clone()));
    tokio::spawn(metrics::sampler(state.clone()));
    tokio::spawn(watch::watcher(state.clone()));
    dashboard::init_csp(&state.config.github_web);
    tokio::spawn(github::poller(state.clone()));
    tokio::spawn(proxy::supervisor(state.clone()));

    let listener = tokio::net::TcpListener::bind(cli.listen)
        .await
        .with_context(|| format!("binding {}", cli.listen))?;
    tracing::info!(addr = %cli.listen, "rig listening");
    let shutdown_state = state.clone();
    axum::serve(
        listener,
        routes::app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    shutdown_state.deploys.shutdown().await;
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&shutdown_state.db)
        .await;
    shutdown_state.db.close().await;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("installing SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutting down");
}

async fn reset_2fa(db: &sqlx::SqlitePool, email: &str) -> Result<()> {
    let email = email.trim().to_lowercase();
    let mut tx = db.begin().await?;
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(id) = id else {
        anyhow::bail!("no user with the email {email}");
    };
    sqlx::query(
        "UPDATE users SET totp_secret = NULL, totp_pending = NULL,
                          totp_pending_expires_at = NULL,
                          totp_enabled_at = NULL, totp_last_step = NULL WHERE id = ?",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM pending_2fa WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO audit_log (id, org_id, user_id, actor, action, target, detail)
         SELECT ?, m.org_id, ?, 'console', 'auth.totp_reset', ?, 'reset from the server console'
         FROM memberships m WHERE m.user_id = ? LIMIT 1",
    )
    .bind(state::new_id())
    .bind(&id)
    .bind(&email)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    println!("Two-factor sign-in is off for {email} and active sessions were signed out. They can sign in with their password.");
    Ok(())
}

async fn reset_password(db: &sqlx::SqlitePool, email: &str) -> Result<String> {
    let email = email.trim().to_lowercase();
    let password = crypto::random_token(15);
    let pw_for_hash = password.clone();
    let hash = tokio::task::spawn_blocking(move || crypto::hash_password(&pw_for_hash))
        .await
        .context("hashing password")??;
    let temp_expires = state::ts(
        chrono::Utc::now() + chrono::Duration::days(routes::members::TEMP_PASSWORD_TTL_DAYS),
    );

    let mut tx = db.begin().await?;
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(id) = id else {
        anyhow::bail!("no user with the email {email}");
    };
    sqlx::query(
        "UPDATE users SET password_hash = ?, must_change_password = 1, temp_password_expires_at = ?
         WHERE id = ?",
    )
    .bind(&hash)
    .bind(&temp_expires)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM api_tokens WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM pending_2fa WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM login_attempts WHERE key = ? OR key LIKE ?")
        .bind(format!("login:email:{email}"))
        .bind(format!("login:pair:{email}:%"))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO audit_log (id, org_id, user_id, actor, action, target, detail)
         SELECT ?, m.org_id, ?, 'console', 'auth.password_reset', ?, 'reset from the server console'
         FROM memberships m WHERE m.user_id = ? LIMIT 1",
    )
    .bind(state::new_id())
    .bind(&id)
    .bind(&email)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(password)
}

/// A base URL without its trailing slash, so paths can be appended.
fn trim_slash(url: &url::Url) -> String {
    url.as_str().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reset_password_updates_hash_clears_sessions_and_audits() {
        let dir = tempfile::tempdir().unwrap();
        let db = state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        let old_hash = crypto::hash_password("original-password-123").unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ('org1', 'Acme', 'acme')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash) VALUES ('u1', 'owner@example.com', 'Owner', ?)",
        )
        .bind(&old_hash)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO memberships (org_id, user_id, role) VALUES ('org1', 'u1', 'owner')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, org_id, csrf_token, expires_at)
             VALUES ('tok1', 'u1', 'org1', 'csrf1', '2099-01-01T00:00:00.000Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, prefix, role, created_at)
             VALUES ('at1', 'org1', 'u1', 'ci', 'athash1', 'rig_xx', 'owner', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO login_attempts (key, ts) VALUES ('login:email:owner@example.com', 9999999999)")
            .execute(&db)
            .await
            .unwrap();

        let new_pw = reset_password(&db, " Owner@Example.com ").await.unwrap();
        assert!(new_pw.len() >= 10);

        let (stored, must_change, temp_exp): (String, i64, Option<String>) = sqlx::query_as(
            "SELECT password_hash, must_change_password, temp_password_expires_at FROM users WHERE id = 'u1'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(crypto::verify_password(&new_pw, &stored));
        assert!(!crypto::verify_password("original-password-123", &stored));
        assert_eq!(must_change, 1);
        assert!(temp_exp.is_some());

        let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = 'u1'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(sessions, 0);

        let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_tokens WHERE user_id = 'u1'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(tokens, 0);

        // Verify reset_2fa also clears active sessions.
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, org_id, csrf_token, expires_at)
             VALUES ('tok2', 'u1', 'org1', 'csrf2', '2099-01-01T00:00:00.000Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        reset_2fa(&db, "owner@example.com").await.unwrap();
        let sessions_after_2fa: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = 'u1'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(sessions_after_2fa, 0);

        let attempts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM login_attempts WHERE key = 'login:email:owner@example.com'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(attempts, 0);

        let audit_action: String =
            sqlx::query_scalar("SELECT action FROM audit_log WHERE user_id = 'u1'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(audit_action, "auth.password_reset");

        assert!(reset_password(&db, "missing@example.com").await.is_err());
    }
}
