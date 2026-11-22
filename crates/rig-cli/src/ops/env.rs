//! Environment variable operations, `.env` parsing/formatting, and diffing
//! shared between CLI and MCP.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;

use rig_api::{BulkSetEnvRequest, EnvVar, SetEnvVarRequest};
use serde::Serialize;

use crate::api::Client;
use crate::error::{CliError, CliResult};

/// Parses and validates every `KEY=VALUE` argument before any API call is sent.
///
/// Supports `KEY=-` (at most once) to read the value from stdin, trimming a
/// single trailing newline if present.
pub fn parse_pairs(pairs: &[String]) -> CliResult<Vec<(String, String)>> {
    let mut stdin_used = false;
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
        let resolved_val = if value == "-" {
            if stdin_used {
                return Err(CliError::message(
                    "only one variable can read its value from stdin (`KEY=-`)",
                ));
            }
            stdin_used = true;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf.trim_end_matches(['\r', '\n']).to_string()
        } else {
            value.to_string()
        };
        out.push((key.to_string(), resolved_val));
    }
    Ok(out)
}

/// Parses `.env` text into `(key, value)` pairs.
///
/// Supports blank lines, `#` comments, optional `export ` prefix, and single/double quotes.
/// Later duplicate keys override earlier ones (preserving last-seen value).
pub fn parse_dotenv(text: &str) -> CliResult<Vec<(String, String)>> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line
            .strip_prefix("export ")
            .or_else(|| line.strip_prefix("export\t"))
            .map(str::trim_start)
            .unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            return Err(CliError::message(format!(
                "line {}: expected KEY=VALUE",
                idx + 1
            )));
        };
        let key = k.trim();
        if key.is_empty() {
            return Err(CliError::message(format!(
                "line {}: empty variable name",
                idx + 1
            )));
        }
        let value = unquote_dotenv_value(v.trim());
        map.insert(key.to_string(), value);
    }
    Ok(map.into_iter().collect())
}

fn unquote_dotenv_value(v: &str) -> String {
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let inner = &v[1..v.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].to_string();
    }
    // Strip inline comments after whitespace for unquoted values.
    if let Some(pos) = v.find(" #") {
        return v[..pos].trim_end().to_string();
    }
    v.to_string()
}

/// Formats variables in `.env` syntax.
pub fn format_dotenv(vars: &[EnvVar], revealed: &HashMap<String, String>) -> String {
    let mut out = String::new();
    for v in vars {
        if let Some(val) = v.value.as_deref().or_else(|| revealed.get(&v.key).map(String::as_str)) {
            out.push_str(&v.key);
            out.push('=');
            out.push_str(&quote_dotenv_value(val));
            out.push('\n');
        } else {
            out.push_str(&format!("# {}=<secret> (run with --reveal to include)\n", v.key));
        }
    }
    out
}

fn quote_dotenv_value(val: &str) -> String {
    let needs_quotes = val.is_empty()
        || val.contains([' ', '\t', '\n', '\r', '#', '"', '\'', '$', '\\']);
    if !needs_quotes {
        return val.to_string();
    }
    let escaped = val
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffKind {
    Added,
    Removed,
    Changed,
    SecretMasked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvDiffEntry {
    pub key: String,
    pub kind: DiffKind,
    pub local: Option<String>,
    pub remote: Option<String>,
}

/// Computes the diff between a local `.env` set and the server's variables.
pub fn diff_env(
    local: &[(String, String)],
    remote: &[EnvVar],
    revealed: &HashMap<String, String>,
) -> Vec<EnvDiffEntry> {
    let local_map: BTreeMap<&str, &str> = local
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let remote_map: BTreeMap<&str, &EnvVar> =
        remote.iter().map(|v| (v.key.as_str(), v)).collect();
    let all_keys: BTreeSet<&str> = local_map
        .keys()
        .chain(remote_map.keys())
        .copied()
        .collect();

    let mut diffs = Vec::new();
    for key in all_keys {
        match (local_map.get(key), remote_map.get(key)) {
            (Some(&lval), None) => diffs.push(EnvDiffEntry {
                key: key.to_string(),
                kind: DiffKind::Added,
                local: Some(lval.to_string()),
                remote: None,
            }),
            (None, Some(rvar)) => diffs.push(EnvDiffEntry {
                key: key.to_string(),
                kind: DiffKind::Removed,
                local: None,
                remote: rvar
                    .value
                    .clone()
                    .or_else(|| revealed.get(key).cloned())
                    .or_else(|| Some("(secret)".into())),
            }),
            (Some(&lval), Some(rvar)) => {
                let rval = rvar
                    .value
                    .as_deref()
                    .or_else(|| revealed.get(key).map(String::as_str));
                match rval {
                    Some(rv) if rv == lval => {}
                    Some(rv) => diffs.push(EnvDiffEntry {
                        key: key.to_string(),
                        kind: DiffKind::Changed,
                        local: Some(lval.to_string()),
                        remote: Some(rv.to_string()),
                    }),
                    None => diffs.push(EnvDiffEntry {
                        key: key.to_string(),
                        kind: DiffKind::SecretMasked,
                        local: Some("(local)".into()),
                        remote: Some("(secret)".into()),
                    }),
                }
            }
            (None, None) => {}
        }
    }
    diffs
}

/// Applies a batch of environment variables atomically via `PUT /api/v1/apps/{id}/env/bulk`
/// when possible, falling back to sequential single `PUT` calls on older servers.
pub async fn set_batch(
    api: &Client,
    app_id: &str,
    pairs: &[(String, String)],
    is_secret: bool,
    replace: bool,
) -> CliResult<Vec<EnvVar>> {
    if pairs.len() == 1 && !replace {
        let (key, value) = &pairs[0];
        let saved = api
            .set_env(
                app_id,
                &SetEnvVarRequest {
                    key: key.clone(),
                    value: value.clone(),
                    is_secret,
                },
            )
            .await?;
        return Ok(vec![saved]);
    }

    let req = BulkSetEnvRequest {
        vars: pairs
            .iter()
            .map(|(k, v)| SetEnvVarRequest {
                key: k.clone(),
                value: v.clone(),
                is_secret,
            })
            .collect(),
        replace,
    };

    match api.set_env_bulk(app_id, &req).await {
        Ok(saved) => Ok(saved),
        Err(err)
            if !replace
                && matches!(
                    &err,
                    CliError::NotFound(m) if m.contains("the server answered 404")
                )
                || (!replace
                    && err.to_string().contains("405 Method Not Allowed")) =>
        {
            set_sequential(api, app_id, pairs, is_secret).await
        }
        Err(e) => Err(e),
    }
}

async fn set_sequential(
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

    #[test]
    fn parses_and_formats_dotenv() {
        let input = r#"
# Comment
export APP_ENV=production
QUOTED="hello\nworld"
SINGLE='raw $val'
INLINE=abc # trailing note
"#;
        let parsed = parse_dotenv(input).unwrap();
        assert_eq!(
            parsed,
            vec![
                ("APP_ENV".into(), "production".into()),
                ("INLINE".into(), "abc".into()),
                ("QUOTED".into(), "hello\nworld".into()),
                ("SINGLE".into(), "raw $val".into()),
            ]
        );

        let vars = vec![
            EnvVar {
                key: "PLAIN".into(),
                value: Some("hello world".into()),
                is_secret: false,
                updated_at: String::new(),
            },
            EnvVar {
                key: "SECRET".into(),
                value: None,
                is_secret: true,
                updated_at: String::new(),
            },
        ];
        let out = format_dotenv(&vars, &HashMap::new());
        assert!(out.contains("PLAIN=\"hello world\""));
        assert!(out.contains("# SECRET=<secret>"));
    }

    #[test]
    fn diffs_local_and_remote_env() {
        let local = vec![
            ("A".into(), "1".into()),
            ("B".into(), "new".into()),
            ("LOCAL_ONLY".into(), "x".into()),
            ("SEC".into(), "val".into()),
        ];
        let remote = vec![
            EnvVar {
                key: "A".into(),
                value: Some("1".into()),
                is_secret: false,
                updated_at: String::new(),
            },
            EnvVar {
                key: "B".into(),
                value: Some("old".into()),
                is_secret: false,
                updated_at: String::new(),
            },
            EnvVar {
                key: "REMOTE_ONLY".into(),
                value: Some("y".into()),
                is_secret: false,
                updated_at: String::new(),
            },
            EnvVar {
                key: "SEC".into(),
                value: None,
                is_secret: true,
                updated_at: String::new(),
            },
        ];
        let diffs = diff_env(&local, &remote, &HashMap::new());
        assert_eq!(diffs.len(), 4);
        assert!(diffs.iter().any(|d| d.key == "B" && d.kind == DiffKind::Changed));
        assert!(diffs.iter().any(|d| d.key == "LOCAL_ONLY" && d.kind == DiffKind::Added));
        assert!(diffs.iter().any(|d| d.key == "REMOTE_ONLY" && d.kind == DiffKind::Removed));
        assert!(diffs.iter().any(|d| d.key == "SEC" && d.kind == DiffKind::SecretMasked));
    }
}
