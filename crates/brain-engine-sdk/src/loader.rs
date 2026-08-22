//! Declarative plugin mounting: the `cordis.yml` analogue for harness config.
//!
//! Validation is schema-shaped via serde and fails loud — a malformed
//! manifest is an error carrying the parser's own message, never a degraded
//! default load.

use std::fmt;

use serde::Deserialize;

/// A parsed harness manifest.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HarnessManifest {
    /// Plugins to mount, in order; later entries may inject earlier keys.
    #[serde(default)]
    pub plugins: Vec<ManifestPlugin>,
}

/// One declared plugin: a stable key plus its dependency list.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ManifestPlugin {
    pub name: String,
    pub key: String,
    /// Keys this plugin depends on (must appear earlier or already mounted).
    #[serde(default)]
    pub inject: Vec<String>,
}

/// Manifest failure — the parser's message surfaces verbatim.
#[derive(Debug)]
pub struct LoaderError(String);

impl fmt::Display for LoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "harness manifest invalid: {}", self.0)
    }
}

impl std::error::Error for LoaderError {}

/// Parse and validate a YAML manifest. Bad input fails loud (never degrades).
pub fn load_yaml(input: &str) -> Result<HarnessManifest, LoaderError> {
    let manifest: HarnessManifest =
        serde_yaml::from_str(input).map_err(|e| LoaderError(e.to_string()))?;
    let mut seen: Vec<&str> = Vec::new();
    for p in &manifest.plugins {
        for dep in &p.inject {
            if !seen.contains(&dep.as_str()) {
                return Err(LoaderError(format!(
                    "plugin `{}` depends on `{dep}`, which is not mounted before it",
                    p.key
                )));
            }
        }
        if seen.contains(&p.key.as_str()) {
            return Err(LoaderError(format!("duplicate plugin key `{}`", p.key)));
        }
        seen.push(&p.key);
    }
    Ok(manifest)
}
