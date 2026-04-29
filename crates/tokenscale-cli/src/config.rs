//! Configuration — read, write, and resolve the `tokenscale` config file.
//!
//! The configuration file is TOML at one of three paths, in resolution
//! order:
//!
//! 1. `--config <path>` on the CLI.
//! 2. `$TOKENSCALE_CONFIG`, if set.
//! 3. `~/.config/tokenscale/config.toml` (cross-platform default).
//!
//! Defaults are designed so that `tokenscale init` produces a working file
//! the user shouldn't need to edit for the local-only happy path.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// In-memory representation of the configuration file.
///
/// Every field is `Option`-wrapped at the surface so the file can omit any
/// section and still be valid; callers should query the resolved value via
/// the helpers (e.g., `effective_database_path`) rather than touching the
/// raw fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// AWS region to apply when looking up grid factors. Anthropic does not
    /// disclose which region served any given request — this is a declared
    /// user assumption, surfaced prominently in the dashboard.
    pub default_inference_region: Option<String>,

    pub ingest: IngestConfig,
    pub storage: StorageConfig,
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub pricing: PricingConfig,
    pub factors: FactorsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IngestConfig {
    /// Persist the original JSONL line in the `events.raw` column. Disable
    /// to keep token counts and metadata only — see README "Privacy".
    pub store_raw: bool,

    /// Override for `~/.claude/projects`. If unset, the default path under
    /// the user's home directory is used.
    pub claude_code_root: Option<PathBuf>,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            store_raw: true,
            claude_code_root: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// Override for the SQLite database path. If unset, a platform-default
    /// is used (`~/Library/Application Support/tokenscale/tokenscale.db` on
    /// macOS; `~/.local/share/tokenscale/tokenscale.db` on Linux).
    pub database_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Address the local HTTP server binds to. Default is loopback.
    pub bind: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8787".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// `localhost` (no auth, loopback bind) or `network` (passkey required).
    /// `network` is Phase 3.
    pub mode: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: "localhost".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PricingConfig {
    /// Override path for `pricing.toml`. If unset, the binary's embedded
    /// copy is used — the seed values shipped with this build of
    /// `tokenscale`. Set this to a local file when you want to ship custom
    /// or freshly-verified prices without rebuilding the binary.
    pub file: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FactorsConfig {
    /// Override path for `environmental-factors.toml`. If unset, the
    /// binary's embedded copy is used. Power users running the dashboard
    /// against a locally-edited factor file (the "local research mode"
    /// from the CHARTER) point this at their working copy.
    pub file: Option<PathBuf>,
}

impl Config {
    /// Load the config from `path`, or return defaults if the file does not
    /// exist. Parse errors are surfaced as anyhow errors with context.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Self =
            toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
        Ok(config)
    }

    /// Serialize to a TOML string. Used by tests to verify the default
    /// config round-trips cleanly. Production `tokenscale init` writes a
    /// hand-authored annotated starter from `render_starter_config` in
    /// main.rs — TOML serialization of the struct itself emits no
    /// comments, which is unhelpful for new users.
    #[cfg(test)]
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("serializing config to TOML")
    }

    /// Resolved Claude Code root path: the configured override, or the
    /// platform default `~/.claude/projects`.
    pub fn effective_claude_code_root(&self) -> Result<PathBuf> {
        if let Some(configured) = self.ingest.claude_code_root.clone() {
            return Ok(configured);
        }
        let home = home_directory()?;
        Ok(home.join(".claude").join("projects"))
    }

    /// Resolved database path: the configured override, or the platform
    /// default under the user's data directory.
    pub fn effective_database_path(&self) -> Result<PathBuf> {
        if let Some(configured) = self.storage.database_path.clone() {
            return Ok(configured);
        }
        Ok(default_data_directory()?.join("tokenscale.db"))
    }
}

/// Resolve the configuration-file path the CLI should read or write.
pub fn resolve_config_path(cli_override: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = cli_override {
        return Ok(path.to_path_buf());
    }
    if let Ok(path_from_env) = std::env::var("TOKENSCALE_CONFIG") {
        return Ok(PathBuf::from(path_from_env));
    }
    let home = home_directory()?;
    Ok(home.join(".config").join("tokenscale").join("config.toml"))
}

fn home_directory() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|base| base.home_dir().to_path_buf())
        .context("could not resolve user home directory")
}

fn default_data_directory() -> Result<PathBuf> {
    let base = directories::BaseDirs::new().context("could not resolve user data directory")?;
    Ok(base.data_dir().join("tokenscale"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_config_roundtrips_through_toml() {
        let original = Config::default();
        let serialized = original.to_toml().unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert!(parsed.ingest.store_raw);
        assert_eq!(parsed.server.bind, "127.0.0.1:8787");
        assert_eq!(parsed.auth.mode, "localhost");
    }

    #[test]
    fn missing_config_file_returns_defaults() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("does-not-exist.toml");
        let config = Config::load_or_default(&path).unwrap();
        assert!(config.default_inference_region.is_none());
    }

    #[test]
    fn unknown_field_in_config_is_an_error() {
        let toml_with_extra = "
mystery_field = 42
[ingest]
store_raw = false
";
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_with_extra).unwrap();
        let result = Config::load_or_default(&path);
        assert!(result.is_err(), "expected parse error for unknown field");
    }

    #[test]
    fn config_with_store_raw_off_is_parsed() {
        let toml_off = "
[ingest]
store_raw = false
";
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_off).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        assert!(!config.ingest.store_raw);
    }
}
