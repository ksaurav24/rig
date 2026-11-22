//! Repo-local `rig.toml` linking so commands can omit `<app>` inside a project directory.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};

pub const FILE_NAME: &str = "rig.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigLink {
    /// Linked app name, slug, `project/app`, or app id.
    pub app: String,
    /// Optional project name or id to disambiguate `app`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Optional saved context name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Optional server URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

impl RigLink {
    /// Returns `project/app` when `project` is set and `app` isn't already qualified.
    pub fn app_ref(&self) -> String {
        if self.app.contains('/') {
            return self.app.clone();
        }
        match &self.project {
            Some(p) if !p.trim().is_empty() => format!("{p}/{}", self.app),
            _ => self.app.clone(),
        }
    }
}

/// Walks upward from `start` looking for `rig.toml`.
pub fn find_from(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

/// Loads the nearest `rig.toml` walking upward from the current working directory.
pub fn discover() -> CliResult<Option<(PathBuf, RigLink)>> {
    let cwd = std::env::current_dir()?;
    let Some(path) = find_from(&cwd) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let link: RigLink = toml::from_str(&text)
        .with_context(|| format!("{} is not valid TOML", path.display()))?;
    if link.app.trim().is_empty() {
        return Err(CliError::message(format!(
            "{} is missing `app`",
            path.display()
        )));
    }
    Ok(Some((path, link)))
}

/// Saves `link` to `dir/rig.toml`.
pub fn save(dir: &Path, link: &RigLink) -> CliResult<PathBuf> {
    let path = dir.join(FILE_NAME);
    let body = toml::to_string_pretty(link)?;
    std::fs::write(&path, body).with_context(|| format!("could not write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_parses_rig_toml_upward() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("packages/web/src");
        std::fs::create_dir_all(&nested).unwrap();

        let link = RigLink {
            app: "web".into(),
            project: Some("Alpha".into()),
            context: Some("prod".into()),
            server: None,
        };
        let written = save(dir.path(), &link).unwrap();
        assert_eq!(find_from(&nested).as_deref(), Some(written.as_path()));
        assert_eq!(link.app_ref(), "Alpha/web");

        let parsed: RigLink = toml::from_str(&std::fs::read_to_string(&written).unwrap()).unwrap();
        assert_eq!(parsed, link);
    }
}
