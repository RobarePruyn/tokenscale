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
use tracing::warn;

/// Default AWS region applied when neither `[inference]
/// .default_inference_region` nor the legacy top-level
/// `default_inference_region` is set. v1 ships with us-east-1 as the
/// dominant Anthropic-on-AWS region; users in other regions override
/// in their config.
pub const DEFAULT_INFERENCE_REGION: &str = "us-east-1";

/// In-memory representation of the configuration file.
///
/// Every field is `Option`-wrapped at the surface so the file can omit any
/// section and still be valid; callers should query the resolved value via
/// the helpers (e.g., `effective_database_path`) rather than touching the
/// raw fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// **Deprecated:** the canonical home is `[inference]
    /// .default_inference_region`. Kept here so v0.x configs that wrote
    /// the field at the top level continue to load. When both this and
    /// `[inference].default_inference_region` are set, the
    /// `[inference]` value wins; when only this is set, it is used and
    /// a one-time warning fires on load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_inference_region: Option<String>,

    pub inference: InferenceConfig,
    pub ingest: IngestConfig,
    pub storage: StorageConfig,
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub pricing: PricingConfig,
    pub factors: FactorsConfig,
}

/// Per-event impact compute settings. Lives in its own table because
/// Phase 2's environmental view exposes more knobs over time (water
/// methodology selection, uncertainty rendering) and grouping them
/// keeps the file readable as the surface grows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InferenceConfig {
    /// AWS region to apply when looking up grid factors. Anthropic does
    /// not disclose which region served any given request — this is a
    /// declared user assumption, surfaced prominently in the dashboard.
    /// Defaults to [`DEFAULT_INFERENCE_REGION`] when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_inference_region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IngestConfig {
    /// Persist the original JSONL line in the `events.raw` column. Disable
    /// to keep token counts and metadata only — see README "Privacy".
    pub store_raw: bool,

    /// **Deprecated:** prefer `claude_code_roots` (plural) so multiple
    /// machines' synced log directories can be scanned in one pass.
    /// Read for back-compat with v0.x configs; emits a one-time warning
    /// on load when set without the plural form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_code_root: Option<PathBuf>,

    /// One or more directories to walk for Claude Code JSONL session
    /// logs. When unset, defaults to the platform-standard
    /// `~/.claude/projects`. Power users running multi-machine setups
    /// (Syncthing / Dropbox / iCloud mirroring laptop sessions to a
    /// desktop, etc.) list every synced root here — the scan walks
    /// each in turn and `_ingest_file_state` keys by full path, so
    /// roots don't collide even if filenames repeat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_code_roots: Option<Vec<PathBuf>>,

    /// How often `tokenscale serve` should run an incremental scan in the
    /// background, in seconds. The first scan runs at startup; the
    /// interval only governs subsequent runs. Set to `0` to disable
    /// auto-scan entirely (the user is responsible for running
    /// `tokenscale scan` themselves).
    pub scan_interval_seconds: u64,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            store_raw: true,
            claude_code_root: None,
            claude_code_roots: None,
            scan_interval_seconds: 60,
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
    ///
    /// If the config carries the deprecated top-level
    /// `default_inference_region` without an `[inference]
    /// .default_inference_region` shadowing it, a one-time warning is
    /// emitted via `tracing::warn` on the first load. The value is
    /// still honored — back-compat does not break.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Self =
            toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
        if config.default_inference_region.is_some()
            && config.inference.default_inference_region.is_none()
        {
            warn!(
                "config field `default_inference_region` is at the top level; \
                 move it under `[inference]` — top-level reads are deprecated \
                 and will be removed in a future version."
            );
        }
        if config.ingest.claude_code_root.is_some() && config.ingest.claude_code_roots.is_none() {
            warn!(
                "config field `[ingest].claude_code_root` (singular) is deprecated; \
                 use `claude_code_roots` (plural, list of paths) so multi-machine \
                 setups can scan synced log directories alongside the local one."
            );
        }
        Ok(config)
    }

    /// Resolved inference region, preferring `[inference]
    /// .default_inference_region`, then the deprecated top-level
    /// field, then [`DEFAULT_INFERENCE_REGION`]. Returned as `&str` so
    /// callers can pass it directly into the factor-lookup queries.
    pub fn effective_inference_region(&self) -> &str {
        self.inference
            .default_inference_region
            .as_deref()
            .or(self.default_inference_region.as_deref())
            .unwrap_or(DEFAULT_INFERENCE_REGION)
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

    /// Resolved list of Claude Code root paths. Resolution order:
    ///   1. `[ingest].claude_code_roots` (plural) — preferred form.
    ///   2. `[ingest].claude_code_root` (singular, deprecated) wrapped
    ///      in a single-element Vec.
    ///   3. Platform default `~/.claude/projects`.
    ///
    /// The Vec is always non-empty when this returns Ok. Empty config
    /// (Some([])) is treated as "use the default" rather than "scan
    /// nothing" — that's the less-surprising behavior, since an empty
    /// list is almost certainly a typo.
    pub fn effective_claude_code_roots(&self) -> Result<Vec<PathBuf>> {
        let raw = if let Some(plural) = self.ingest.claude_code_roots.as_ref() {
            if plural.is_empty() {
                vec![default_claude_code_root()?]
            } else {
                plural.clone()
            }
        } else if let Some(singular) = self.ingest.claude_code_root.clone() {
            vec![singular]
        } else {
            vec![default_claude_code_root()?]
        };
        raw.into_iter().map(expand_tilde).collect()
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

/// Platform-default Claude Code session root: `~/.claude/projects`.
/// Used by `Config::effective_claude_code_roots` when no override is
/// configured.
fn default_claude_code_root() -> Result<PathBuf> {
    let home = home_directory()?;
    Ok(home.join(".claude").join("projects"))
}

/// Expand a leading `~/` (or bare `~`) to the user's home directory.
/// TOML stores paths as plain strings — `~` is not a shell, no
/// expansion happens for free. Without this helper, a config like
/// `claude_code_roots = ["~/.claude/projects"]` would try to open
/// a directory literally named `~` and fail with `RootNotFound`.
///
/// Paths without a leading `~` are returned unchanged. Non-UTF-8
/// paths (rare; macOS / Windows can have them) are passed through —
/// we can't tilde-expand without converting to a string.
fn expand_tilde(path: PathBuf) -> Result<PathBuf> {
    let Some(as_str) = path.to_str() else {
        return Ok(path);
    };
    if let Some(rest) = as_str.strip_prefix("~/") {
        return Ok(home_directory()?.join(rest));
    }
    if as_str == "~" {
        return home_directory();
    }
    Ok(path)
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
        assert!(config.inference.default_inference_region.is_none());
        assert_eq!(config.effective_inference_region(), DEFAULT_INFERENCE_REGION);
    }

    #[test]
    fn inference_section_overrides_top_level_for_back_compat() {
        let toml_with_both = r#"
default_inference_region = "us-east-2"
[inference]
default_inference_region = "us-west-2"
"#;
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_with_both).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        assert_eq!(config.effective_inference_region(), "us-west-2");
    }

    #[test]
    fn legacy_top_level_only_still_resolves() {
        let toml_legacy = r#"
default_inference_region = "us-east-2"
"#;
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_legacy).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        assert_eq!(config.effective_inference_region(), "us-east-2");
    }

    #[test]
    fn inference_section_only_resolves() {
        let toml_inference = r#"
[inference]
default_inference_region = "us-east-2"
"#;
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_inference).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        assert_eq!(config.effective_inference_region(), "us-east-2");
    }

    #[test]
    fn tilde_paths_expand_to_absolute_paths() {
        // TOML stores paths verbatim — `~/...` won't expand unless we
        // do it ourselves. Without expansion the scan silently fails.
        let toml_with_tildes = r#"
[ingest]
claude_code_roots = ["~/.claude/projects", "~/.claude-synced/laptop/projects"]
"#;
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_with_tildes).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        let roots = config.effective_claude_code_roots().unwrap();
        assert_eq!(roots.len(), 2);
        for root in &roots {
            assert!(
                root.is_absolute(),
                "tilde-prefixed config should expand to absolute path, got {root:?}",
            );
            assert!(
                !root.to_string_lossy().starts_with('~'),
                "leading ~ should be replaced, got {root:?}",
            );
        }
    }

    #[test]
    fn claude_code_roots_plural_wins_over_singular_back_compat() {
        let toml_both = r#"
[ingest]
claude_code_root = "/old/singular"
claude_code_roots = ["/new/plural/a", "/new/plural/b"]
"#;
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_both).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        let roots = config.effective_claude_code_roots().unwrap();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0], std::path::PathBuf::from("/new/plural/a"));
        assert_eq!(roots[1], std::path::PathBuf::from("/new/plural/b"));
    }

    #[test]
    fn singular_only_resolves_to_one_root_for_back_compat() {
        let toml_singular = r#"
[ingest]
claude_code_root = "/legacy/path"
"#;
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_singular).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        let roots = config.effective_claude_code_roots().unwrap();
        assert_eq!(roots, vec![std::path::PathBuf::from("/legacy/path")]);
    }

    #[test]
    fn empty_plural_falls_back_to_default_root() {
        // Empty list is treated as "unset" — assume typo, not "scan nothing."
        let toml_empty = r"
[ingest]
claude_code_roots = []
";
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, toml_empty).unwrap();
        let config = Config::load_or_default(&path).unwrap();
        let roots = config.effective_claude_code_roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with(".claude/projects"));
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
