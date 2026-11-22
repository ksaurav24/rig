//! Saved servers ("contexts") in `~/.config/rig/config.toml`.
//!
//! The file holds API tokens, so it is written with mode 0600.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context as _;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub current: Option<String>,
    #[serde(default)]
    pub contexts: BTreeMap<String, Context>,
}

/// Overrides accepted by every server-talking command (flags + env vars).
#[derive(Debug, Clone, Default)]
pub struct ResolveOptions<'a> {
    pub context: Option<&'a str>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
}

fn path() -> CliResult<PathBuf> {
    if let Some(explicit) = std::env::var_os("RIG_CONFIG") {
        return Ok(PathBuf::from(explicit));
    }
    let dir = dirs::config_dir()
        .context("could not find your config directory. Set RIG_CONFIG")?;
    Ok(dir.join("rig").join("config.toml"))
}

impl Config {
    pub fn load() -> CliResult<Self> {
        let path = path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("{} is not valid. Fix or delete it", path.display()))
                .map_err(CliError::from),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(anyhow::Error::from(e)
                .context(format!("could not read {}", path.display()))
                .into()),
        }
    }

    pub fn save(&self) -> CliResult<()> {
        let path = path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        // Write a private temp file and rename it, so the token is never
        // readable by others and a crash cannot leave half a config.
        let tmp = path.with_extension("toml.tmp");
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(text.as_bytes())?;
        }
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("could not write {}", path.display()))?;
        Ok(())
    }

    /// The context to use: `--server`/`RIG_URL` (with `RIG_TOKEN` or a matching saved context),
    /// else `--context`/`RIG_CONTEXT`, else the current context.
    pub fn resolve_with(&self, opts: &ResolveOptions<'_>) -> CliResult<Context> {
        if let Some(raw_server) = opts.server.map(str::trim).filter(|s| !s.is_empty()) {
            let url = normalize_url(raw_server, opts.insecure)?;
            let normalized = url.to_string();
            let token = opts
                .token
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    self.contexts
                        .values()
                        .find(|c| c.url.trim_end_matches('/') == normalized.trim_end_matches('/'))
                        .map(|c| c.token.clone())
                });
            let Some(token) = token else {
                return Err(CliError::auth(format!(
                    "no token for {normalized}. Set RIG_TOKEN or run: rig login {normalized}"
                )));
            };
            return Ok(Context {
                url: normalized,
                token,
            });
        }

        self.resolve(opts.context)
    }

    /// The saved context to use: the named one, else the current one.
    pub fn resolve(&self, name: Option<&str>) -> CliResult<Context> {
        let name = match name.or(self.current.as_deref()) {
            Some(n) => n,
            None => return Err(CliError::auth("not signed in. Run: rig login <url>")),
        };
        self.contexts.get(name).cloned().ok_or_else(|| {
            CliError::auth(format!(
                "there is no context called {name}. See: rig context"
            ))
        })
    }

    pub fn select(&mut self, name: &str) -> CliResult<()> {
        if !self.contexts.contains_key(name) {
            return Err(CliError::not_found(format!(
                "there is no context called {name}. See: rig context"
            )));
        }
        self.current = Some(name.to_string());
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> CliResult<()> {
        if self.contexts.remove(name).is_none() {
            return Err(CliError::not_found(format!(
                "there is no context called {name}. See: rig context"
            )));
        }
        if self.current.as_deref() == Some(name) {
            self.current = self.contexts.keys().next().cloned();
        }
        Ok(())
    }
}

/// Accepts `rig.example.com` or a full URL. Plain HTTP would send the token
/// in the clear, so it is only allowed to this machine unless `insecure`.
pub fn normalize_url(input: &str, insecure: bool) -> CliResult<Url> {
    let input = input.trim();
    let with_scheme = if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let mut url =
        Url::parse(&with_scheme).with_context(|| format!("{input} is not a valid address"))?;
    let host = url.host_str().unwrap_or_default().to_string();
    if host.is_empty() {
        return Err(CliError::message(format!("{input} is not a valid address")));
    }
    match url.scheme() {
        "https" => {}
        "http" => {
            let local = host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback());
            if !local && !insecure {
                return Err(CliError::message(format!(
                    "{input} uses plain HTTP, which would send your token unencrypted. Use https://, or pass --insecure on a network you trust"
                )));
            }
        }
        other => {
            return Err(CliError::message(format!(
                "{other}:// is not supported. Use https://"
            )));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CliError::message(
            "put the token in RIG_TOKEN, not in the address",
        ));
    }
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            normalize_url("rig.example.com", false).unwrap().as_str(),
            "https://rig.example.com/"
        );
        assert_eq!(
            normalize_url("https://rig.example.com/x?y#z", false)
                .unwrap()
                .as_str(),
            "https://rig.example.com/"
        );
        assert!(normalize_url("http://127.0.0.1:7460", false).is_ok());
        assert!(normalize_url("http://localhost:7460", false).is_ok());
        assert!(normalize_url("http://[::1]:7460", false).is_ok());
        assert!(normalize_url("http://rig.example.com", false).is_err());
        assert!(normalize_url("http://rig.example.com", true).is_ok());
        assert!(normalize_url("ftp://rig.example.com", false).is_err());
        assert!(normalize_url("https://user:pw@rig.example.com", false).is_err());
    }

    #[test]
    fn contexts() {
        let mut config = Config::default();
        assert_eq!(config.resolve(None).unwrap_err().exit_code(), 3);
        for name in ["a", "b"] {
            config.contexts.insert(
                name.into(),
                Context {
                    url: format!("https://{name}/"),
                    token: "t".into(),
                },
            );
        }
        config.select("b").unwrap();
        assert_eq!(config.resolve(None).unwrap().url, "https://b/");
        assert_eq!(config.resolve(Some("a")).unwrap().url, "https://a/");
        assert_eq!(config.select("zzz").unwrap_err().exit_code(), 4);
        config.remove("b").unwrap();
        assert_eq!(config.current.as_deref(), Some("a"));
    }

    #[test]
    fn stateless_server_and_token_override() {
        let mut config = Config::default();
        let ctx = config
            .resolve_with(&ResolveOptions {
                server: Some("http://127.0.0.1:7460"),
                token: Some("rig_secret"),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ctx.url, "http://127.0.0.1:7460/");
        assert_eq!(ctx.token, "rig_secret");

        // Server without token falls back to saved context matching that URL.
        config.contexts.insert(
            "local".into(),
            Context {
                url: "http://127.0.0.1:7460/".into(),
                token: "saved_tok".into(),
            },
        );
        let ctx = config
            .resolve_with(&ResolveOptions {
                server: Some("http://127.0.0.1:7460"),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ctx.token, "saved_tok");

        // Unknown server without token fails with exit code 3 (Auth).
        let err = config
            .resolve_with(&ResolveOptions {
                server: Some("https://other.example.com"),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.exit_code(), 3);
    }
}
