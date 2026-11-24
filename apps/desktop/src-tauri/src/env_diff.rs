//! Local `.env` parser and diff engine for drag-and-drop import (§9.2).
//!
//! Runs completely locally in Rust. Existing secret values are NEVER returned
//! by the Rig server, so diffs mark overwriting an existing secret explicitly
//! while keeping secret values masked in previews.

use std::collections::{BTreeMap, HashMap};

use rig_api::{BulkSetEnvRequest, EnvVar, SetEnvVarRequest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedEnvEntry {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvDiffKind {
    Added,
    Changed,
    SecretOverwrite,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvDiffItem {
    pub key: String,
    pub kind: EnvDiffKind,
    pub current_value: Option<String>,
    pub current_is_secret: bool,
    pub incoming_preview: String,
    pub incoming_is_secret: bool,
    /// Actual incoming value kept for one-click apply; redacted in logs.
    pub incoming_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvDiffPreview {
    pub items: Vec<EnvDiffItem>,
    pub added: usize,
    pub changed: usize,
    pub secret_overwrites: usize,
    pub unchanged: usize,
    pub skipped_lines: usize,
}

/// Heuristic matching `rig-server`'s secret classifier for newly added keys.
pub fn looks_like_secret(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "TOKEN",
        "API_KEY",
        "APIKEY",
        "PRIVATE_KEY",
        "DATABASE_URL",
        "REDIS_URL",
        "DSN",
        "AUTH",
        "SIGNING",
        "CREDENTIAL",
        "CERT",
    ]
    .iter()
    .any(|kw| upper.contains(kw))
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parses `.env` content (supports comments, `export KEY=VAL`, single/double
/// quotes, `\n`/`\r`/`\t` escapes inside double quotes, and inline `#` comments
/// on unquoted values). Duplicate keys keep the last occurrence.
pub fn parse_dotenv(content: &str) -> (Vec<ParsedEnvEntry>, usize) {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut skipped = 0usize;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed
            .strip_prefix("export ")
            .map(str::trim_start)
            .unwrap_or(trimmed);
        let Some((raw_key, raw_val)) = body.split_once('=') else {
            skipped += 1;
            continue;
        };
        let key = raw_key.trim();
        if !is_valid_env_key(key) {
            skipped += 1;
            continue;
        }
        let val = parse_value(raw_val.trim());
        map.insert(key.to_owned(), val);
    }

    let entries = map
        .into_iter()
        .map(|(key, value)| {
            let is_secret = looks_like_secret(&key);
            ParsedEnvEntry {
                key,
                value,
                is_secret,
            }
        })
        .collect();
    (entries, skipped)
}

fn parse_value(raw: &str) -> String {
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let inner = &raw[1..raw.len() - 1];
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
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return raw[1..raw.len() - 1].to_owned();
    }
    // Strip trailing inline comment preceded by whitespace for unquoted values.
    if let Some(pos) = raw.find(" #") {
        return raw[..pos].trim_end().to_owned();
    }
    raw.to_owned()
}

pub fn mask_preview(value: &str) -> String {
    if value.is_empty() {
        return "(empty)".into();
    }
    "••••••••".into()
}

/// Computes a diff between parsed `.env` entries and the server's current `EnvVar`s.
pub fn diff_env(content: &str, current: &[EnvVar]) -> EnvDiffPreview {
    let (entries, skipped_lines) = parse_dotenv(content);
    let existing: HashMap<&str, &EnvVar> = current.iter().map(|e| (e.key.as_str(), e)).collect();

    let mut items = Vec::with_capacity(entries.len());
    let mut added = 0;
    let mut changed = 0;
    let mut secret_overwrites = 0;
    let mut unchanged = 0;

    for entry in entries {
        match existing.get(entry.key.as_str()) {
            None => {
                added += 1;
                let preview = if entry.is_secret {
                    mask_preview(&entry.value)
                } else {
                    entry.value.clone()
                };
                items.push(EnvDiffItem {
                    key: entry.key,
                    kind: EnvDiffKind::Added,
                    current_value: None,
                    current_is_secret: false,
                    incoming_preview: preview,
                    incoming_is_secret: entry.is_secret,
                    incoming_value: entry.value,
                });
            }
            Some(cur) if cur.is_secret => {
                // Server never returns secret values, so we cannot compare equality;
                // always flag as a masked secret overwrite and keep is_secret=true.
                secret_overwrites += 1;
                items.push(EnvDiffItem {
                    key: entry.key,
                    kind: EnvDiffKind::SecretOverwrite,
                    current_value: None,
                    current_is_secret: true,
                    incoming_preview: mask_preview(&entry.value),
                    incoming_is_secret: true,
                    incoming_value: entry.value,
                });
            }
            Some(cur) => {
                let cur_val = cur.value.as_deref().unwrap_or("");
                if cur_val == entry.value {
                    unchanged += 1;
                    items.push(EnvDiffItem {
                        key: entry.key,
                        kind: EnvDiffKind::Unchanged,
                        current_value: cur.value.clone(),
                        current_is_secret: false,
                        incoming_preview: entry.value.clone(),
                        incoming_is_secret: false,
                        incoming_value: entry.value,
                    });
                } else {
                    changed += 1;
                    items.push(EnvDiffItem {
                        key: entry.key,
                        kind: EnvDiffKind::Changed,
                        current_value: cur.value.clone(),
                        current_is_secret: false,
                        incoming_preview: entry.value.clone(),
                        incoming_is_secret: false,
                        incoming_value: entry.value,
                    });
                }
            }
        }
    }

    EnvDiffPreview {
        items,
        added,
        changed,
        secret_overwrites,
        unchanged,
        skipped_lines,
    }
}

/// Converts selected diff items into a single `BulkSetEnvRequest` (skipping unchanged).
pub fn to_bulk_request(items: &[EnvDiffItem]) -> BulkSetEnvRequest {
    BulkSetEnvRequest {
        replace: false,
        vars: items
            .iter()
            .filter(|i| i.kind != EnvDiffKind::Unchanged)
            .map(|i| SetEnvVarRequest {
                key: i.key.clone(),
                value: i.incoming_value.clone(),
                is_secret: i.incoming_is_secret || i.current_is_secret,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotenv_and_masks_secret_diffs() {
        let dotenv = r#"
# comment
export PORT=8080 # inline comment
APP_NAME="My Cool App"
API_SECRET='top-secret-123'
NEW_FLAG=true
INVALID LINE
"#;
        let current = vec![
            EnvVar {
                key: "PORT".into(),
                value: Some("8080".into()),
                is_secret: false,
                updated_at: "2026-11-24T00:00:00Z".into(),
            },
            EnvVar {
                key: "APP_NAME".into(),
                value: Some("Old Name".into()),
                is_secret: false,
                updated_at: "2026-11-24T00:00:00Z".into(),
            },
            EnvVar {
                key: "API_SECRET".into(),
                value: None,
                is_secret: true,
                updated_at: "2026-11-24T00:00:00Z".into(),
            },
        ];

        let diff = diff_env(dotenv, &current);
        assert_eq!(diff.unchanged, 1);
        assert_eq!(diff.changed, 1);
        assert_eq!(diff.secret_overwrites, 1);
        assert_eq!(diff.added, 1);
        assert_eq!(diff.skipped_lines, 1);

        let sec = diff.items.iter().find(|i| i.key == "API_SECRET").unwrap();
        assert_eq!(sec.kind, EnvDiffKind::SecretOverwrite);
        assert_eq!(sec.incoming_preview, "••••••••");
        assert!(sec.incoming_is_secret);

        let bulk = to_bulk_request(&diff.items);
        assert_eq!(bulk.vars.len(), 3);
    }
}
