//! Environment variable operations shared between CLI and MCP.

use rig_api::{EnvVar, SetEnvVarRequest};

use crate::api::Client;
use crate::error::{CliError, CliResult};

/// Parses and validates every `KEY=VALUE` argument before any API call is sent.
pub fn parse_pairs(pairs: &[String]) -> CliResult<Vec<(String, String)>> {
    let mut out = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(CliError::message(format!("`{pair}` is not KEY=VALUE")));
        };
        if key.trim().is_empty() {
            return Err(CliError::message(format!(
                "`{pair}` has an empty variable name"
            )));
        }
        out.push((key.to_string(), value.to_string()));
    }
    Ok(out)
}

/// Applies a batch of environment variables one-by-one.
///
/// If a later key fails, reports which earlier keys were already saved on the server
/// so callers know the partial state until a bulk server endpoint lands in Phase 2.
pub async fn set_batch(
    api: &Client,
    app_id: &str,
    pairs: &[(String, String)],
    is_secret: bool,
) -> CliResult<Vec<EnvVar>> {
    let mut saved = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        let req = SetEnvVarRequest {
            key: key.clone(),
            value: value.clone(),
            is_secret,
        };
        match api.set_env(app_id, &req).await {
            Ok(var) => saved.push(var),
            Err(err) => {
                if saved.is_empty() {
                    return Err(err);
                }
                let already: Vec<&str> = saved.iter().map(|v| v.key.as_str()).collect();
                let detail = format!(
                    "failed to set {key}: {err} (already saved: {})",
                    already.join(", ")
                );
                return Err(match err {
                    CliError::Auth(_) => CliError::auth(detail),
                    CliError::NotFound(_) => CliError::not_found(detail),
                    CliError::Network(_) => CliError::network(detail),
                    _ => CliError::message(detail),
                });
            }
        }
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_and_rejects_bad_pairs() {
        let parsed = parse_pairs(&["FOO=bar=baz".into(), "EMPTY=".into()]).unwrap();
        assert_eq!(
            parsed,
            vec![
                ("FOO".into(), "bar=baz".into()),
                ("EMPTY".into(), "".into()),
            ]
        );
        assert!(parse_pairs(&["NO_EQUALS".into()]).is_err());
        assert!(parse_pairs(&["=val".into()]).is_err());
    }
}
