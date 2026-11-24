//! GitHub integration: a GitHub App per organization, created through
//! GitHub's manifest flow, used to clone private repositories and to receive
//! push events. Servers GitHub cannot reach poll each auto-deploy branch.
//!
//! Secrets (the App's private key, webhook secret and client secret) are kept
//! vault-encrypted in `github_apps`. Installation tokens are short-lived and
//! live in memory only: they never reach the database, a URL, an argument
//! list, `.git/config` or a log.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use aws_lc_rs::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde::Deserialize;
use sqlx::Row;

use crate::{
    deploy::{self, NewDeployment},
    state::{AppState, now},
};

/// How long a manifest flow may take between "Create" and GitHub's redirect.
const STATE_TTL: Duration = Duration::from_secs(10 * 60);
/// Installation tokens last an hour; refresh well before that.
const TOKEN_TTL: Duration = Duration::from_secs(50 * 60);
const LS_REMOTE_TIMEOUT: Duration = Duration::from_secs(30);

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("rig/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static client settings are valid")
});

// ------------------------------------------------------------ the App's keys

/// The decrypted credentials of an organization's GitHub App.
pub struct Creds {
    pub app_id: i64,
    pub private_key: String,
    pub webhook_secret: String,
}

pub async fn creds(state: &AppState, org_id: &str) -> Result<Option<Creds>> {
    let Some(r) =
        sqlx::query("SELECT app_id, private_key, webhook_secret FROM github_apps WHERE org_id = ?")
            .bind(org_id)
            .fetch_optional(&state.db)
            .await?
    else {
        return Ok(None);
    };
    Ok(Some(Creds {
        app_id: r.get("app_id"),
        private_key: state.vault.decrypt(r.get("private_key"))?,
        webhook_secret: state.vault.decrypt(r.get("webhook_secret"))?,
    }))
}

/// A JWT that authenticates as the App itself (RS256, valid 9 minutes, issued
/// a minute in the past to absorb clock drift).
pub fn app_jwt(app_id: i64, pem: &str, unix_now: i64) -> Result<String> {
    let key = parse_private_key(pem)?;
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::json!({ "iat": unix_now - 60, "exp": unix_now + 540, "iss": app_id.to_string() })
            .to_string(),
    );
    let signing_input = format!("{header}.{claims}");
    let mut sig = vec![0; key.public_modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        signing_input.as_bytes(),
        &mut sig,
    )
    .map_err(|_| anyhow!("signing the GitHub App token failed"))?;
    Ok(format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(sig)))
}

/// GitHub hands out PKCS#1 PEM keys; PKCS#8 is accepted too.
fn parse_private_key(pem: &str) -> Result<RsaKeyPair> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("-----"))
        .collect();
    let der = STANDARD
        .decode(body)
        .context("the GitHub App private key is not valid PEM")?;
    RsaKeyPair::from_der(&der)
        .or_else(|_| RsaKeyPair::from_pkcs8(&der))
        .map_err(|e| anyhow!("the GitHub App private key was rejected: {e}"))
}

fn unix_now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

// --------------------------------------------------------------- repositories

/// `owner/name` of a repository URL on this GitHub host, or `None` for any
/// other host. Only plain `https://host/owner/name[.git]` URLs match.
pub fn parse_repo(web: &str, url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix(web)?.strip_prefix('/')?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (ok(owner) && ok(name)).then(|| (owner.to_string(), name.to_string()))
}

#[derive(Deserialize)]
struct Installation {
    id: i64,
}

#[derive(Deserialize)]
struct AccessToken {
    token: String,
}

async fn api_get<T: serde::de::DeserializeOwned>(
    state: &AppState,
    path: &str,
    bearer: &str,
) -> Result<T> {
    let res = HTTP
        .get(format!("{}{path}", state.config.github_api))
        .bearer_auth(bearer)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .context("GitHub is unreachable")?;
    let status = res.status();
    if !status.is_success() {
        bail!("GitHub answered {status} for {path}");
    }
    res.json().await.context("unexpected reply from GitHub")
}

async fn api_post<T: serde::de::DeserializeOwned>(
    state: &AppState,
    path: &str,
    bearer: Option<&str>,
    body: &serde_json::Value,
) -> Result<T> {
    let mut req = HTTP
        .post(format!("{}{path}", state.config.github_api))
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .json(body);
    if let Some(b) = bearer {
        req = req.bearer_auth(b);
    }
    let res = req.send().await.context("GitHub is unreachable")?;
    let status = res.status();
    if !status.is_success() {
        bail!("GitHub answered {status} for {path}");
    }
    res.json().await.context("unexpected reply from GitHub")
}

static TOKENS: LazyLock<Mutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(Default::default);

/// A read-only token for one repository, or `None` when the App is not
/// installed on it (a public repository then clones without one).
async fn repo_token(
    state: &AppState,
    org_id: &str,
    owner: &str,
    name: &str,
) -> Result<Option<String>> {
    let key = format!(
        "{org_id}:{}/{}",
        owner.to_ascii_lowercase(),
        name.to_ascii_lowercase()
    );
    if let Some((token, at)) = TOKENS.lock().expect("token cache").get(&key)
        && at.elapsed() < TOKEN_TTL
    {
        return Ok(Some(token.clone()));
    }
    let Some(c) = creds(state, org_id).await? else {
        return Ok(None);
    };
    let jwt = app_jwt(c.app_id, &c.private_key, unix_now())?;
    let res = HTTP
        .get(format!(
            "{}/repos/{owner}/{name}/installation",
            state.config.github_api
        ))
        .bearer_auth(&jwt)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .context("GitHub is unreachable")?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !res.status().is_success() {
        bail!(
            "GitHub answered {} looking up the App installation",
            res.status()
        );
    }
    let inst: Installation = res.json().await.context("unexpected reply from GitHub")?;
    let tok: AccessToken = api_post(
        state,
        &format!("/app/installations/{}/access_tokens", inst.id),
        Some(&jwt),
        &serde_json::json!({ "repositories": [name], "permissions": { "contents": "read" } }),
    )
    .await?;
    let mut cache = TOKENS.lock().expect("token cache");
    cache.retain(|_, (_, at)| at.elapsed() < TOKEN_TTL);
    cache.insert(key, (tok.token.clone(), Instant::now()));
    Ok(Some(tok.token))
}

/// Forget every cached token of an organization (its App was removed).
pub fn forget_tokens(org_id: &str) {
    let prefix = format!("{org_id}:");
    TOKENS
        .lock()
        .expect("token cache")
        .retain(|k, _| !k.starts_with(&prefix));
}

/// Environment for `git` that sends an installation token to this GitHub
/// host only, through `GIT_CONFIG_*` so it stays out of argv and config files.
pub fn token_env(web: &str, token: &str) -> Vec<(String, String)> {
    let basic = STANDARD.encode(format!("x-access-token:{token}"));
    vec![
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        (
            "GIT_CONFIG_KEY_0".into(),
            format!("http.{web}/.extraheader"),
        ),
        (
            "GIT_CONFIG_VALUE_0".into(),
            format!("AUTHORIZATION: basic {basic}"),
        ),
    ]
}

/// Git environment for cloning `url`, when the organization's App can read it.
pub async fn clone_auth(
    state: &AppState,
    org_id: &str,
    url: &str,
) -> Result<Option<Vec<(String, String)>>> {
    let Some((owner, name)) = parse_repo(&state.config.github_web, url) else {
        return Ok(None);
    };
    Ok(repo_token(state, org_id, &owner, &name)
        .await?
        .map(|t| token_env(&state.config.github_web, &t)))
}

#[derive(Deserialize)]
struct InstalledRepos {
    repositories: Vec<RepoJson>,
}

#[derive(Deserialize)]
struct RepoJson {
    full_name: String,
    clone_url: String,
    default_branch: String,
    private: bool,
}

/// Repositories the App is installed on, across its installations (the first
/// 100 of each of up to 20 installations).
pub async fn installed_repos(state: &AppState, org_id: &str) -> Result<Vec<rig_api::GithubRepo>> {
    let Some(c) = creds(state, org_id).await? else {
        return Ok(Vec::new());
    };
    let jwt = app_jwt(c.app_id, &c.private_key, unix_now())?;
    let installs: Vec<Installation> =
        api_get(state, "/app/installations?per_page=20", &jwt).await?;
    let mut out = Vec::new();
    for inst in installs {
        let tok: AccessToken = api_post(
            state,
            &format!("/app/installations/{}/access_tokens", inst.id),
            Some(&jwt),
            &serde_json::json!({ "permissions": { "metadata": "read" } }),
        )
        .await?;
        let page: InstalledRepos =
            api_get(state, "/installation/repositories?per_page=100", &tok.token).await?;
        out.extend(page.repositories.into_iter().map(|r| rig_api::GithubRepo {
            full_name: r.full_name,
            clone_url: r.clone_url,
            default_branch: r.default_branch,
            private: r.private,
        }));
    }
    out.sort_by_key(|r| r.full_name.to_lowercase());
    Ok(out)
}

#[derive(Deserialize)]
struct BranchJson {
    name: String,
    #[serde(default)]
    protected: bool,
}

#[derive(Deserialize)]
struct RepoInfo {
    default_branch: String,
}

/// Branches of a repository the App is installed on, default branch first
/// (up to 500). `None` when the App cannot read the repository.
pub async fn branches(
    state: &AppState,
    org_id: &str,
    owner: &str,
    name: &str,
) -> Result<Option<Vec<rig_api::GithubBranch>>> {
    let Some(token) = repo_token(state, org_id, owner, name).await? else {
        return Ok(None);
    };
    let info: RepoInfo = api_get(state, &format!("/repos/{owner}/{name}"), &token).await?;
    let mut out = Vec::new();
    for page in 1..=5 {
        let batch: Vec<BranchJson> = api_get(
            state,
            &format!("/repos/{owner}/{name}/branches?per_page=100&page={page}"),
            &token,
        )
        .await?;
        let done = batch.len() < 100;
        out.extend(batch.into_iter().map(|b| rig_api::GithubBranch {
            default: b.name == info.default_branch,
            name: b.name,
            protected: b.protected,
        }));
        if done {
            break;
        }
    }
    out.sort_by(|a, b| {
        b.default
            .cmp(&a.default)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Some(out))
}

// -------------------------------------------------------------- manifest flow

struct Pending {
    org_id: String,
    user_id: String,
    at: Instant,
}

static STATES: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(Default::default);
const MAX_PENDING_STATES: usize = 64;

/// Remember a manifest flow started by this user, for one use within 10 minutes.
pub fn issue_state(org_id: &str, user_id: &str) -> String {
    let state = crate::crypto::random_token(24);
    let mut states = STATES.lock().expect("state map");
    states.retain(|_, p| p.at.elapsed() < STATE_TTL);
    if states.len() >= MAX_PENDING_STATES
        && let Some(oldest) = states
            .iter()
            .min_by_key(|(_, p)| p.at)
            .map(|(k, _)| k.clone())
    {
        states.remove(&oldest);
    }
    states.insert(
        state.clone(),
        Pending {
            org_id: org_id.into(),
            user_id: user_id.into(),
            at: Instant::now(),
        },
    );
    state
}

/// Consume a manifest state. It is gone afterwards whether or not it matched.
pub fn take_state(state: &str, org_id: &str, user_id: &str) -> bool {
    let Some(p) = STATES.lock().expect("state map").remove(state) else {
        return false;
    };
    p.at.elapsed() < STATE_TTL && p.org_id == org_id && p.user_id == user_id
}

#[derive(Deserialize)]
pub struct Conversion {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub html_url: String,
    pub owner: Owner,
    pub client_id: String,
    pub client_secret: String,
    pub webhook_secret: Option<String>,
    pub pem: String,
}

#[derive(Deserialize)]
pub struct Owner {
    pub login: String,
}

/// Exchange the temporary code GitHub redirected with for the new App's keys.
pub async fn convert_manifest(state: &AppState, code: &str) -> Result<Conversion> {
    if code.is_empty() || code.len() > 100 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("the code from GitHub is malformed");
    }
    api_post(
        state,
        &format!("/app-manifests/{code}/conversions"),
        None,
        &serde_json::json!({}),
    )
    .await
}

// ------------------------------------------------------------------ deploying

/// Queue a deployment of `app_id` for a new commit, unless one is already in
/// progress or the commit is already the latest one deployed.
/// Returns the deployment id, or why nothing was queued.
pub async fn deploy_commit(
    state: &AppState,
    org_id: &str,
    app_id: &str,
    sha: Option<&str>,
    trigger: &str,
) -> Result<Result<String, String>> {
    let latest: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT status, commit_sha FROM deployments WHERE app_id = ?
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;
    if let Some((status, last_sha)) = &latest {
        if matches!(status.as_str(), "queued" | "building" | "deploying") {
            return Ok(Err("a deployment is already in progress".into()));
        }
        if let (Some(a), Some(b)) = (sha, last_sha.as_deref())
            && a.eq_ignore_ascii_case(b)
        {
            return Ok(Err(format!(
                "commit {} is already deployed",
                &a[..a.len().min(10)]
            )));
        }
    }
    let id = match deploy::enqueue(
        state,
        NewDeployment {
            org_id,
            app_id,
            trigger,
            created_by: None,
            image_ref: None,
            image_id: None,
            source_dir: None,
            commit_msg: None,
        },
    )
    .await
    {
        Ok(id) => id,
        Err(e) if deploy::is_active_deploy_conflict(&e) => {
            return Ok(Err("a deployment is already in progress".into()));
        }
        Err(e) => return Err(e.into()),
    };
    sqlx::query("UPDATE apps SET status = 'deploying', updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(app_id)
        .execute(&state.db)
        .await?;
    crate::auth::audit(
        state,
        crate::auth::AuditEvent {
            org_id: Some(org_id),
            user_id: None,
            actor: trigger,
            action: "app.deploy",
            target: Some(app_id),
            detail: Some(&id),
            ip: None,
        },
    )
    .await
    .map_err(|e| anyhow!("{e}"))?;
    Ok(Ok(id))
}

/// Apps of an organization that deploy on push from `owner/name`, branch `branch`
/// (`default_branch` stands in for apps without a branch set).
pub async fn apps_for_push(
    state: &AppState,
    org_id: &str,
    full_name: &str,
    branch: &str,
    default_branch: &str,
) -> Result<Vec<String>> {
    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, git_url, git_branch FROM apps
          WHERE org_id = ? AND auto_deploy = 1 AND source_kind = 'git'",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, url, want)| {
            let repo_matches = url
                .as_deref()
                .and_then(|u| parse_repo(&state.config.github_web, u))
                .is_some_and(|(o, n)| format!("{o}/{n}").eq_ignore_ascii_case(full_name));
            let want = want
                .as_deref()
                .filter(|b| !b.is_empty())
                .unwrap_or(default_branch);
            repo_matches && want == branch
        })
        .map(|(id, _, _)| id)
        .collect())
}

/// Record a webhook delivery id. False when it was seen before.
pub async fn first_delivery(state: &AppState, id: &str) -> Result<bool> {
    sqlx::query("DELETE FROM github_deliveries WHERE received_at < ?")
        .bind(
            (time::OffsetDateTime::now_utc() - time::Duration::days(7))
                .format(&time::format_description::well_known::Rfc3339)?,
        )
        .execute(&state.db)
        .await?;
    let done = sqlx::query("INSERT OR IGNORE INTO github_deliveries (id) VALUES (?)")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(done.rows_affected() == 1)
}

// -------------------------------------------------------------------- polling

/// Check each auto-deploy app's branch every `poll_interval` seconds and
/// deploy new commits. Apps never deployed are left alone, so creating an
/// app does not start a build on its own.
pub async fn poller(state: AppState) {
    if state.config.poll_interval == 0 {
        return;
    }
    let every = Duration::from_secs(state.config.poll_interval.max(10));
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if let Err(e) = poll_once(&state).await {
            tracing::warn!("polling repositories failed: {e:#}");
        }
    }
}

/// id, org_id, git_url, git_branch, latest deployed commit.
type PollRow = (String, String, String, Option<String>, Option<String>);

async fn poll_once(state: &AppState) -> Result<()> {
    use futures_util::stream::{self, StreamExt};

    let apps: Vec<PollRow> = sqlx::query_as(
        "SELECT a.id, a.org_id, a.git_url, a.git_branch,
                (SELECT commit_sha FROM deployments d WHERE d.app_id = a.id
                  ORDER BY created_at DESC LIMIT 1)
           FROM apps a
          WHERE a.auto_deploy = 1 AND a.source_kind = 'git' AND a.git_url IS NOT NULL
            AND EXISTS (SELECT 1 FROM deployments d WHERE d.app_id = a.id)",
    )
    .fetch_all(&state.db)
    .await?;
    stream::iter(apps)
        .for_each_concurrent(4, |(app_id, org_id, url, branch, last)| async move {
            let head = match remote_head(state, &org_id, &url, branch.as_deref()).await {
                Ok(Some(h)) => h,
                Ok(None) => return,
                Err(e) => {
                    tracing::debug!("checking {url} failed: {e:#}");
                    return;
                }
            };
            if last
                .as_deref()
                .is_some_and(|l| l.eq_ignore_ascii_case(&head))
            {
                return;
            }
            match deploy_commit(state, &org_id, &app_id, Some(&head), "poll").await {
                Ok(Ok(id)) => tracing::info!("new commit {head} on {url}; queued deployment {id}"),
                Ok(Err(why)) => tracing::debug!("not deploying {url}: {why}"),
                Err(e) => tracing::warn!("queueing polled commit for {url}: {e:#}"),
            }
        })
        .await;
    Ok(())
}

/// The commit a branch (or the default branch) points at, via `git ls-remote`.
async fn remote_head(
    state: &AppState,
    org_id: &str,
    url: &str,
    branch: Option<&str>,
) -> Result<Option<String>> {
    deploy::validate_git_url_with(url, state.config.allow_private_git).map_err(|m| anyhow!(m))?;
    deploy::validate_git_remote_ip(url, state.config.allow_private_git).await?;
    let reference = match branch.filter(|b| !b.is_empty()) {
        Some(b) => format!("refs/heads/{b}"),
        None => "HEAD".into(),
    };
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(deploy::GIT_SAFE_CONFIG)
        .args(["ls-remote", "--", url, &reference])
        .stdin(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true")
        .kill_on_drop(true);
    if let Some(env) = clone_auth(state, org_id, url).await.unwrap_or(None) {
        cmd.envs(env);
    }
    let out = tokio::time::timeout(LS_REMOTE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| anyhow!("git ls-remote timed out"))??;
    if !out.status.success() {
        bail!("git ls-remote exited with {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .filter(|sha| sha.len() >= 40 && sha.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway 1024-bit key, used only by these tests.
    const TEST_KEY: &str = include_str!("../tests/github-test-key.pem");

    #[test]
    fn repo_urls() {
        let web = "https://github.com";
        assert_eq!(
            parse_repo(web, "https://github.com/acme/site.git"),
            Some(("acme".into(), "site".into()))
        );
        assert_eq!(
            parse_repo(web, "https://github.com/acme/site/"),
            Some(("acme".into(), "site".into()))
        );
        assert_eq!(parse_repo(web, "https://gitlab.com/acme/site"), None);
        assert_eq!(
            parse_repo(web, "https://github.com.evil.io/acme/site"),
            None
        );
        assert_eq!(parse_repo(web, "https://github.com/acme"), None);
        assert_eq!(
            parse_repo(web, "https://github.com/acme/site/tree/main"),
            None
        );
        assert_eq!(parse_repo(web, "https://github.com/../etc"), None);
    }

    #[test]
    fn token_stays_in_config_env() {
        let env = token_env("https://github.com", "ghs_secret");
        assert_eq!(env[1].1, "http.https://github.com/.extraheader");
        assert!(env[2].1.starts_with("AUTHORIZATION: basic "));
        assert!(!env[2].1.contains("ghs_secret"));
    }

    #[test]
    fn jwt_is_signed_rs256() {
        let jwt = app_jwt(42, TEST_KEY, 1_700_000_000).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["iss"], "42");
        assert_eq!(claims["exp"], 1_700_000_540);
        // Verify the signature against the key's public half.
        let key = parse_private_key(TEST_KEY).unwrap();
        let public = aws_lc_rs::signature::UnparsedPublicKey::new(
            &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA256,
            aws_lc_rs::signature::KeyPair::public_key(&key)
                .as_ref()
                .to_vec(),
        );
        public
            .verify(
                format!("{}.{}", parts[0], parts[1]).as_bytes(),
                &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn states_are_single_use_and_bound() {
        let s = issue_state("org1", "user1");
        assert!(!take_state(&s, "org1", "user2"));
        let s = issue_state("org1", "user1");
        assert!(take_state(&s, "org1", "user1"));
        assert!(!take_state(&s, "org1", "user1"));
    }

    #[test]
    fn states_map_is_bounded() {
        for _ in 0..80 {
            let _ = issue_state("org-cap", "user-cap");
        }
        assert!(STATES.lock().unwrap().len() <= MAX_PENDING_STATES);
    }
}
