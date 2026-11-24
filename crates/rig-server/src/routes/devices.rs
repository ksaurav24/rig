//! Device pairing and device token management for Rig Desktop (and mobile).
//!
//! Device tokens live in `api_tokens` with `kind = 'device'`, so they reuse
//! the existing bearer auth, RBAC role ceiling, `verify_still_valid` stream
//! revocation, and audit log.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
};
use qrcode::{QrCode, render::svg};
use rand::RngExt;
use rig_api::{
    ApiToken, CreatePairingRequest, DeviceLoginRequest, ErrorBody, ExchangePairingRequest,
    Organization, PairedDevice, PairingCode, Role, TokenKind, User,
};
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{self, AuditEvent, Auth, TOKEN_PREFIX, Viewer},
    crypto,
    error::{ApiError, ApiResult},
    state::{AppState, new_id, now, ts},
};

const PAIRING_DEFAULT_TTL_SECS: u32 = 300;
const PAIRING_MAX_TTL_SECS: u32 = 900;
const MAX_PAIRING_ATTEMPTS_PER_IP: i64 = 12;
const MAX_DEVICES_PER_USER: i64 = 32;

/// Crockford-like alphabet without ambiguous chars (0/O, 1/I/L).
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_devices))
        .routes(routes!(revoke_device))
        .routes(routes!(create_pairing))
        .routes(routes!(exchange_pairing))
        .routes(routes!(device_login))
}

fn generate_pairing_code() -> String {
    let mut rng = rand::rng();
    let mut raw = [0u8; 8];
    for b in &mut raw {
        let idx = rng.random_range(0..CODE_ALPHABET.len());
        *b = CODE_ALPHABET[idx];
    }
    format!(
        "{}-{}",
        std::str::from_utf8(&raw[..4]).unwrap_or("XXXX"),
        std::str::from_utf8(&raw[4..]).unwrap_or("XXXX")
    )
}

pub fn normalize_pairing_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn render_pairing_qr(code: &str, tls_fingerprint: Option<&str>) -> String {
    let mut uri = format!("rig://pair?code={code}");
    if let Some(fp) = tls_fingerprint {
        uri.push_str("&fp=");
        uri.push_str(fp);
    }
    QrCode::new(uri.as_bytes())
        .map(|q| {
            q.render::<svg::Color>()
                .min_dimensions(180, 180)
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default()
}

fn device_token_from_row(r: &sqlx::sqlite::SqliteRow) -> ApiToken {
    ApiToken {
        id: r.get("id"),
        name: r.get("name"),
        prefix: r.get("prefix"),
        role: Role::parse(r.get("role")).unwrap_or(Role::Viewer),
        kind: TokenKind::parse(r.get("kind")),
        last_ip: r.get("last_ip"),
        last_used_at: r.get("last_used_at"),
        expires_at: r.get("expires_at"),
        created_at: r.get("created_at"),
    }
}

/// Issue a one-time pairing code directly in SQLite (shared by the HTTP route
/// and the on-box `rig-server devices pair` command).
pub async fn issue_pairing_code(
    db: &sqlx::SqlitePool,
    org_id: &str,
    user_id: &str,
    role: Role,
    ttl_secs: u32,
) -> anyhow::Result<PairingCode> {
    let id = new_id();
    let code = generate_pairing_code();
    let normalized = normalize_pairing_code(&code);
    let code_hash = crypto::hash_token(&normalized);
    let expires_at = ts(chrono::Utc::now() + chrono::Duration::seconds(i64::from(ttl_secs)));

    // Opportunistically clean up expired/used codes.
    let _ = sqlx::query("DELETE FROM device_pairings WHERE expires_at <= ? OR used_at IS NOT NULL")
        .bind(now())
        .execute(db)
        .await;

    sqlx::query(
        "INSERT INTO device_pairings (id, org_id, user_id, code_hash, role, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(org_id)
    .bind(user_id)
    .bind(&code_hash)
    .bind(role.as_str())
    .bind(&expires_at)
    .execute(db)
    .await?;

    let qr_svg = render_pairing_qr(&code, None);
    Ok(PairingCode {
        id,
        code,
        role,
        expires_at,
        tls_fingerprint: None,
        qr_svg,
    })
}

async fn mint_device_token(
    state: &AppState,
    org_id: &str,
    user_id: &str,
    role: Role,
    device_name: &str,
    ip: &str,
) -> ApiResult<(ApiToken, String)> {
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens
          WHERE org_id = ? AND user_id = ? AND kind = 'device'
            AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(now())
    .fetch_one(&state.db)
    .await?;
    if active_count >= MAX_DEVICES_PER_USER {
        return Err(ApiError::Validation(format!(
            "you can have at most {MAX_DEVICES_PER_USER} paired devices; revoke an unused one first"
        )));
    }
    let secret = format!("{TOKEN_PREFIX}{}", crypto::random_token(32));
    let prefix: String = secret.chars().take(TOKEN_PREFIX.len() + 6).collect();
    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, prefix, role, kind, last_ip, last_used_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'device', ?, ?, ?)",
    )
    .bind(&id)
    .bind(org_id)
    .bind(user_id)
    .bind(device_name)
    .bind(crypto::hash_token(&secret))
    .bind(&prefix)
    .bind(role.as_str())
    .bind(ip)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.db)
    .await?;

    Ok((
        ApiToken {
            id,
            name: device_name.to_owned(),
            prefix,
            role,
            kind: TokenKind::Device,
            last_ip: Some(ip.to_owned()),
            last_used_at: Some(created_at.clone()),
            expires_at: None,
            created_at,
        },
        secret,
    ))
}

async fn load_user_org_role(
    state: &AppState,
    org_id: &str,
    user_id: &str,
) -> ApiResult<(User, Organization, Role)> {
    let row = sqlx::query(
        "SELECT u.id AS user_id, u.email, u.name AS user_name, u.avatar_url,
                o.id AS org_id, o.name AS org_name, o.slug, o.logo_url,
                o.accent_color, o.env_label, o.env_color, m.role
         FROM users u
         JOIN memberships m ON m.user_id = u.id AND m.org_id = ?
         JOIN organizations o ON o.id = m.org_id
         WHERE u.id = ? AND u.disabled = 0",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    let role = Role::parse(row.get("role"))
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("invalid stored role")))?;
    let user = User {
        id: row.get("user_id"),
        email: row.get("email"),
        name: row.get("user_name"),
        avatar_url: row.get("avatar_url"),
    };
    let org = Organization {
        id: row.get("org_id"),
        name: row.get("org_name"),
        slug: row.get("slug"),
        logo_url: row.get("logo_url"),
        accent_color: row.get("accent_color"),
        env_label: row.get("env_label"),
        env_color: row.get("env_color"),
    };
    Ok((user, org, role))
}

/// Create a short-lived one-time pairing code for Rig Desktop or mobile.
#[utoipa::path(post, path = "/api/v1/devices/pair", tag = "devices",
    request_body = CreatePairingRequest,
    responses((status = 200, body = PairingCode), (status = 403, body = ErrorBody)))]
async fn create_pairing(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<CreatePairingRequest>,
) -> ApiResult<Json<PairingCode>> {
    let role = req.role.unwrap_or(auth.role);
    if role > auth.role {
        return Err(ApiError::Forbidden(
            "a pairing code cannot grant a higher role than your own",
        ));
    }
    let ttl = req
        .expires_in_secs
        .unwrap_or(PAIRING_DEFAULT_TTL_SECS)
        .clamp(60, PAIRING_MAX_TTL_SECS);
    let pairing = issue_pairing_code(&state.db, &auth.org_id, &auth.user_id, role, ttl)
        .await
        .map_err(ApiError::Internal)?;
    auth.audit(&state, "device.pair_code", Some(&pairing.id), Some(role.as_str()))
        .await?;
    Ok(Json(pairing))
}

/// Exchange a one-time pairing code for a revocable device token.
#[utoipa::path(post, path = "/api/v1/devices/exchange", tag = "devices",
    request_body = ExchangePairingRequest,
    responses((status = 200, body = PairedDevice), (status = 401, body = ErrorBody),
              (status = 429, body = ErrorBody)))]
async fn exchange_pairing(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ExchangePairingRequest>,
) -> ApiResult<Json<PairedDevice>> {
    let ip = auth::real_ip(&headers, addr.ip()).to_string();
    let rate_key = format!("pair:ip:{ip}");
    auth::check_rate(&state, &rate_key, MAX_PAIRING_ATTEMPTS_PER_IP).await?;

    let device_name = super::auth::validate_name("device name", &req.device_name, 80)?;
    let normalized = normalize_pairing_code(&req.code);
    if normalized.len() < 6 {
        auth::record_failure(&state, &rate_key).await?;
        return Err(ApiError::Unauthorized);
    }
    let code_hash = crypto::hash_token(&normalized);
    let current = now();

    let row = sqlx::query(
        "SELECT id, org_id, user_id, role
         FROM device_pairings
         WHERE code_hash = ? AND used_at IS NULL AND expires_at > ?",
    )
    .bind(&code_hash)
    .bind(&current)
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        auth::record_failure(&state, &rate_key).await?;
        return Err(ApiError::Unauthorized);
    };

    let pairing_id: String = row.get("id");
    // Consume atomically so concurrent exchanges cannot both succeed.
    let consumed = sqlx::query(
        "UPDATE device_pairings SET used_at = ? WHERE id = ? AND used_at IS NULL AND expires_at > ?",
    )
    .bind(&current)
    .bind(&pairing_id)
    .bind(&current)
    .execute(&state.db)
    .await?
    .rows_affected();
    if consumed != 1 {
        auth::record_failure(&state, &rate_key).await?;
        return Err(ApiError::Unauthorized);
    }

    let org_id: String = row.get("org_id");
    let user_id: String = row.get("user_id");
    let requested_role = Role::parse(row.get("role")).unwrap_or(Role::Viewer);
    let (user, org, member_role) = load_user_org_role(&state, &org_id, &user_id).await?;
    let effective_role = requested_role.min(member_role);

    let (token, secret) =
        mint_device_token(&state, &org.id, &user.id, effective_role, &device_name, &ip).await?;
    auth::clear_failures(&state, &rate_key).await?;

    auth::audit(
        &state,
        AuditEvent {
            org_id: Some(&org.id),
            user_id: Some(&user.id),
            actor: &user.email,
            action: "device.pair",
            target: Some(&device_name),
            detail: Some(effective_role.as_str()),
            ip: Some(&ip),
        },
    )
    .await?;

    Ok(Json(PairedDevice {
        token,
        secret,
        user,
        org,
        role: effective_role,
    }))
}

/// Sign in with email, password (and optional TOTP) from Rig Desktop, directly
/// issuing a revocable device token stored in the OS keychain.
#[utoipa::path(post, path = "/api/v1/devices/login", tag = "devices",
    request_body = DeviceLoginRequest,
    responses((status = 200, body = PairedDevice), (status = 401, body = ErrorBody),
              (status = 429, body = ErrorBody)))]
async fn device_login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<DeviceLoginRequest>,
) -> ApiResult<Json<PairedDevice>> {
    let device_name = super::auth::validate_name("device name", &req.device_name, 80)?;
    let login_req = rig_api::LoginRequest {
        email: req.email,
        password: req.password,
        code: req.code,
        challenge_token: req.challenge_token,
    };
    let verified = super::auth::verify_login_credentials(&state, addr, &headers, login_req).await?;
    if verified.must_change_password {
        return Err(ApiError::Forbidden(
            "you must change your temporary password before pairing a device",
        ));
    }

    let (token, secret) = mint_device_token(
        &state,
        &verified.org.id,
        &verified.user.id,
        verified.role,
        &device_name,
        &verified.ip,
    )
    .await?;

    auth::audit(
        &state,
        AuditEvent {
            org_id: Some(&verified.org.id),
            user_id: Some(&verified.user.id),
            actor: &verified.user.email,
            action: "device.login",
            target: Some(&device_name),
            detail: Some(verified.role.as_str()),
            ip: Some(&verified.ip),
        },
    )
    .await?;

    Ok(Json(PairedDevice {
        token,
        secret,
        user: verified.user,
        org: verified.org,
        role: verified.role,
    }))
}

/// List paired devices for the signed-in user.
#[utoipa::path(get, path = "/api/v1/devices", tag = "devices",
    responses((status = 200, body = Vec<ApiToken>)))]
async fn list_devices(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<Vec<ApiToken>>> {
    let rows = sqlx::query(
        "SELECT id, name, prefix, role, kind, last_ip, last_used_at, expires_at, created_at
         FROM api_tokens
         WHERE org_id = ? AND user_id = ? AND kind = 'device'
         ORDER BY created_at DESC",
    )
    .bind(&auth.org_id)
    .bind(&auth.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(device_token_from_row).collect()))
}

/// Revoke a paired device token. Active streams are cut at the next auth re-check.
#[utoipa::path(delete, path = "/api/v1/devices/{id}", tag = "devices",
    params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = ErrorBody)))]
async fn revoke_device(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let n = sqlx::query(
        "DELETE FROM api_tokens WHERE id = ? AND org_id = ? AND user_id = ? AND kind = 'device'",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .bind(&auth.user_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("device"));
    }
    auth.audit(&state, "device.revoke", Some(&id), None).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_api::{Access, ServerMeta, ServerMode};
    use std::sync::Arc;

    #[test]
    fn pairing_code_format_and_normalization() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        let norm = normalize_pairing_code(&code.to_lowercase());
        assert_eq!(norm.len(), 8);
        assert!(norm.bytes().all(|b| CODE_ALPHABET.contains(&b)));
    }

    #[tokio::test]
    async fn headless_meta_and_device_pairing_end_to_end() {
        struct StubRuntime;
        #[async_trait::async_trait]
        impl crate::runtime::Runtime for StubRuntime {
            async fn info(&self) -> anyhow::Result<crate::runtime::EngineInfo> {
                Ok(crate::runtime::EngineInfo {
                    name: "stub".into(),
                    version: "1.0".into(),
                })
            }
            async fn pull(&self, _: &str, _: &crate::runtime::LogSink) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build(
                &self,
                _: crate::runtime::BuildSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build_plan(
                &self,
                _: crate::runtime::PlanSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn run(
                &self,
                _: &crate::runtime::ContainerSpec,
            ) -> anyhow::Result<crate::runtime::Started> {
                anyhow::bail!("unused")
            }
            async fn state(&self, _: &str) -> anyhow::Result<crate::runtime::ContainerState> {
                Ok(crate::runtime::ContainerState::Running)
            }
            async fn health(&self, _: &str) -> anyhow::Result<Option<crate::runtime::Health>> {
                Ok(None)
            }
            async fn start(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn stop(&self, _: &str, _: u32) -> anyhow::Result<()> {
                Ok(())
            }
            async fn remove(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            fn logs_with(
                &self,
                _: &str,
                _: crate::runtime::LogOptions,
            ) -> crate::runtime::LogStream {
                Box::pin(futures_util::stream::empty())
            }
            async fn list_by_label(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
                Ok(vec![])
            }
            async fn remove_volume(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn remove_network(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<crate::runtime::Shell> {
                anyhow::bail!("unused")
            }
            async fn image_id(&self, _: &str) -> anyhow::Result<Option<String>> {
                Ok(None)
            }
            async fn usage(&self, _: &str) -> anyhow::Result<Option<crate::runtime::Usage>> {
                Ok(None)
            }
            async fn resize_shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<()> {
                Ok(())
            }
            async fn exec(
                &self,
                _: &str,
                _: Vec<String>,
                _: Vec<String>,
                _: Option<crate::runtime::ByteReader>,
                _: crate::runtime::ByteWriter,
            ) -> anyhow::Result<crate::runtime::ExecOutcome> {
                anyhow::bail!("unused")
            }
        }

        struct StubProxy;
        #[async_trait::async_trait]
        impl crate::proxy::Proxy for StubProxy {
            async fn ready(&self) -> bool {
                true
            }
            async fn apply(
                &self,
                _: &[crate::proxy::Route],
                _: Option<&rig_api::DashboardDomain>,
                _: &[String],
            ) -> anyhow::Result<()> {
                Ok(())
            }
            fn ports(&self) -> (u16, u16) {
                (80, 443)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db = crate::state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        let vault = crate::crypto::Vault::open(dir.path()).unwrap();
        let access = crate::access::DashboardAccess::default();
        access.set(Access::Local);

        let state = AppState {
            db: db.clone(),
            vault,
            config: Arc::new(crate::state::Config {
                data_dir: dir.path().to_path_buf(),
                secure_cookies: false,
                github_api: "http://127.0.0.1".into(),
                github_web: "http://127.0.0.1".into(),
                poll_interval: 0,
                allow_private_alerts: false,
                allow_private_git: false,
                enable_api_docs: false,
                mode: ServerMode::Headless,
            }),
            runtime: Arc::new(StubRuntime),
            proxy: Arc::new(StubProxy),
            deploys: Arc::new(crate::deploy::Deployer::new().unwrap()),
            access,
            caches: Arc::new(crate::state::RuntimeCaches::default()),
            events: Arc::new(crate::events::EventBus::new()),
        };

        let pw_hash = crate::crypto::hash_password("desktop-test-password-123").unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ('org1', 'Acme', 'acme')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash) VALUES ('u1', 'owner@acme.test', 'Owner', ?)",
        )
        .bind(&pw_hash)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO memberships (org_id, user_id, role) VALUES ('org1', 'u1', 'owner')")
            .execute(&db)
            .await
            .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = tokio::spawn(async move {
            axum::serve(
                listener,
                crate::routes::app(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let base = format!("http://{addr}");
        let http = reqwest::Client::new();

        // 1. Headless mode returns 404 for dashboard root.
        let res = http.get(format!("{base}/")).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        // 2. Unauthenticated /api/v1/meta hides mode & access.
        let meta: ServerMeta = http
            .get(format!("{base}/api/v1/meta"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(meta.api_version, 1);
        assert!(meta.mode.is_none());
        assert!(meta.access.is_none());
        assert!(meta.capabilities.iter().any(|c| c == "device_pairing"));

        // 3. Issue pairing code on-box and exchange for device token.
        let pairing = issue_pairing_code(&db, "org1", "u1", Role::Developer, 300)
            .await
            .unwrap();
        let ex_req = ExchangePairingRequest {
            code: pairing.code.clone(),
            device_name: "thinkpad · Rig Desktop".into(),
        };
        let paired: PairedDevice = http
            .post(format!("{base}/api/v1/devices/exchange"))
            .json(&ex_req)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(paired.role, Role::Developer);
        assert_eq!(paired.token.kind, TokenKind::Device);

        // 4. Replaying the same single-use pairing code fails 401.
        let replay = http
            .post(format!("{base}/api/v1/devices/exchange"))
            .json(&ex_req)
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);

        // 5. Authenticated /api/v1/meta reveals mode=headless & access=local.
        let meta_authed: ServerMeta = http
            .get(format!("{base}/api/v1/meta"))
            .bearer_auth(&paired.secret)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(meta_authed.mode, Some(ServerMode::Headless));
        assert_eq!(meta_authed.access, Some(Access::Local));

        // 6. Password device login mints a device token directly.
        let logged: PairedDevice = http
            .post(format!("{base}/api/v1/devices/login"))
            .json(&DeviceLoginRequest {
                email: "owner@acme.test".into(),
                password: "desktop-test-password-123".into(),
                code: None,
                challenge_token: None,
                device_name: "workstation · Rig Desktop".into(),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(logged.role, Role::Owner);
        assert_eq!(logged.token.kind, TokenKind::Device);

        // 7. Revoke paired device token and verify immediate rejection.
        let del = http
            .delete(format!("{base}/api/v1/devices/{}", paired.token.id))
            .bearer_auth(&paired.secret)
            .send()
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);

        let after = http
            .get(format!("{base}/api/v1/auth/me"))
            .bearer_auth(&paired.secret)
            .send()
            .await
            .unwrap();
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

        srv.abort();
    }
}
