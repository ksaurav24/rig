//! Push-to-deploy. Each app has its own webhook secret; a request deploys the
//! app only when it proves it knows that secret, either as a GitHub-style
//! HMAC signature over the body (`X-Hub-Signature-256` or `X-Rig-Signature`)
//! or as GitLab's plain `X-Gitlab-Token`. No session or API token is involved,
//! so the endpoint works from any Git host or CI job.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use rig_api::{ErrorBody, WebhookResult};
use sha2::Sha256;
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{AuditEvent, audit},
    crypto::constant_time_eq,
    deploy::{self, NewDeployment},
    error::{ApiError, ApiResult},
    state::{AppState, now},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(app_hook))
}

/// Whether the request carries a valid HMAC-SHA256 signature over `body`.
/// Used by GitHub App webhooks (which must never accept plain `X-Gitlab-Token`).
pub fn verify_hmac(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let Some(sig) = header("x-hub-signature-256").or_else(|| header("x-rig-signature")) else {
        return false;
    };
    let Some(hex_sig) = sig.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    // `verify_slice` compares in constant time.
    mac.verify_slice(&expected).is_ok()
}

/// Whether the request carries proof of `secret` (HMAC signature or GitLab token).
pub fn verify(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if header("x-hub-signature-256").is_some() || header("x-rig-signature").is_some() {
        return verify_hmac(secret, headers, body);
    }
    header("x-gitlab-token").is_some_and(|t| constant_time_eq(t, secret))
}

/// The branch a push event is for, when the body says so.
fn pushed_branch(headers: &HeaderMap, body: &[u8]) -> Option<String> {
    let event = headers
        .get("x-github-event")
        .or_else(|| headers.get("x-gitlab-event"))
        .and_then(|v| v.to_str().ok())?;
    if !event.eq_ignore_ascii_case("push") && !event.eq_ignore_ascii_case("push hook") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("ref")?
        .as_str()?
        .strip_prefix("refs/heads/")
        .map(str::to_owned)
}

/// Deploy an app from a Git host or CI webhook.
#[utoipa::path(post, path = "/api/v1/hooks/apps/{id}", tag = "deployments",
    params(("id" = String, Path)),
    request_body(content = String, description = "Any payload. Signed with the app's webhook secret."),
    responses((status = 202, body = WebhookResult), (status = 200, body = WebhookResult),
        (status = 401, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn app_hook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<WebhookResult>)> {
    let row = sqlx::query(
        "SELECT org_id, name, source_kind, git_branch, webhook_secret FROM apps WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await?;
    // An unknown app and a bad signature look the same from outside.
    let Some(row) = row else {
        return Err(ApiError::Unauthorized);
    };
    let secret_enc: String = row.get("webhook_secret");
    let secret = state.vault.decrypt(&secret_enc)?;
    if !verify(&secret, &headers, &body) {
        return Err(ApiError::Unauthorized);
    }

    let skipped = |why: String| {
        Ok((
            StatusCode::OK,
            Json(WebhookResult {
                deployment_id: None,
                skipped: Some(why),
            }),
        ))
    };
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if event == "ping" {
        return skipped("ping received. The webhook is set up correctly".into());
    }
    let branch: Option<String> = row.get("git_branch");
    if row.get::<String, _>("source_kind") == "git"
        && let (Some(want), Some(got)) = (branch, pushed_branch(&headers, &body))
        && want != got
    {
        return skipped(format!("push to {got}; this app deploys {want}"));
    }

    let in_progress: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM deployments
          WHERE app_id = ? AND status IN ('queued', 'building', 'deploying')",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    if in_progress > 0 {
        return Err(ApiError::Conflict(
            "a deployment is already in progress for this app".into(),
        ));
    }
    let org_id: String = row.get("org_id");
    let deployment = deploy::enqueue(
        &state,
        NewDeployment {
            org_id: &org_id,
            app_id: &id,
            trigger: "webhook",
            created_by: None,
            image_ref: None,
            image_id: None,
        },
    )
    .await
    .map_err(|e| {
        if deploy::is_active_deploy_conflict(&e) {
            ApiError::Conflict("a deployment is already in progress for this app".into())
        } else {
            e.into()
        }
    })?;
    sqlx::query("UPDATE apps SET status = 'deploying', updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(&id)
        .execute(&state.db)
        .await?;
    audit(
        &state,
        AuditEvent {
            org_id: Some(&org_id),
            user_id: None,
            actor: "webhook",
            action: "app.deploy",
            target: Some(&id),
            detail: Some(&deployment),
            ip: None,
        },
    )
    .await?;
    tracing::info!(
        "webhook queued a deployment of {}",
        row.get::<String, _>("name")
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(WebhookResult {
            deployment_id: Some(deployment),
            skipped: None,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn hmac_signatures_verify() {
        let body = br#"{"ref":"refs/heads/main"}"#;
        let good = sign("s3cret", body);
        assert!(verify(
            "s3cret",
            &headers(&[("x-hub-signature-256", &good)]),
            body
        ));
        assert!(verify(
            "s3cret",
            &headers(&[("x-rig-signature", &good)]),
            body
        ));
        assert!(!verify(
            "other",
            &headers(&[("x-hub-signature-256", &good)]),
            body
        ));
        assert!(!verify(
            "s3cret",
            &headers(&[("x-hub-signature-256", &good)]),
            b"tampered"
        ));
        assert!(!verify(
            "s3cret",
            &headers(&[("x-hub-signature-256", "sha256=zz")]),
            body
        ));
        assert!(!verify(
            "s3cret",
            &headers(&[("x-hub-signature-256", "md5=00")]),
            body
        ));
        assert!(!verify("s3cret", &HeaderMap::new(), body));
    }

    #[test]
    fn gitlab_token_verifies() {
        assert!(verify("tok", &headers(&[("x-gitlab-token", "tok")]), b""));
        assert!(!verify("tok", &headers(&[("x-gitlab-token", "nope")]), b""));
        // Strict HMAC verifier used by GitHub App hooks must reject X-Gitlab-Token.
        assert!(!verify_hmac(
            "tok",
            &headers(&[("x-gitlab-token", "tok")]),
            b""
        ));
    }

    #[test]
    fn a_bad_signature_is_not_rescued_by_a_token() {
        let h = headers(&[
            ("x-hub-signature-256", "sha256=00"),
            ("x-gitlab-token", "tok"),
        ]);
        assert!(!verify("tok", &h, b""));
    }

    #[test]
    fn push_branch_is_read_from_the_payload() {
        let body = br#"{"ref":"refs/heads/feature/x"}"#;
        let h = headers(&[("x-github-event", "push")]);
        assert_eq!(pushed_branch(&h, body).as_deref(), Some("feature/x"));
        let tag = br#"{"ref":"refs/tags/v1"}"#;
        assert_eq!(pushed_branch(&h, tag), None);
        assert_eq!(
            pushed_branch(&headers(&[("x-github-event", "issues")]), body),
            None
        );
    }
}
