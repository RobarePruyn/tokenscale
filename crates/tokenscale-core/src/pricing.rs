//! Pricing model — loaded from `pricing.toml` at the repo root.
//!
//! Mirrors the shape and discipline of `environmental-factors.toml`:
//!
//! - A top-level `schema_version` integer guards compatibility. The loader
//!   refuses to run against a file outside its supported range.
//! - A `file_status` field flags whether the values have been reviewed.
//!   The dashboard surfaces "needs_review" prominently so users know the
//!   billable view is approximate.
//! - Every value carries a `source_url` and `source_accessed_at` so the
//!   provenance is recoverable from the file alone.
//!
//! Prices use Anthropic's standard convention:
//!
//! - `input_usd_per_mtok`, `output_usd_per_mtok`, `cache_read_usd_per_mtok`
//!   are absolute dollar prices per million tokens of that type.
//! - `cache_write_5m_multiplier` and `cache_write_1h_multiplier` are
//!   multipliers on the input price, matching the way Anthropic's docs
//!   express the prompt-caching surcharge.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::Path;

use crate::error::{CoreError, Result};

/// Schema versions this build understands. A file outside this range is
/// rejected at load time — the loader-side guard the CHARTER calls for.
const SUPPORTED_SCHEMA_RANGE: RangeInclusive<i64> = 1..=1;

/// `pricing.toml` from the repo root, embedded in the binary at compile
/// time. Used as the default when the user's config does not point at an
/// override. This keeps `tokenscale serve` working on a fresh install
/// without any extra setup; users who want to customize prices can drop a
/// file at `~/.config/tokenscale/pricing.toml` (or wherever) and point
/// `pricing.file` in the config at it.
const EMBEDDED_PRICING_TOML: &str = include_str!("../../../pricing.toml");

/// In-memory representation of `pricing.toml`. Cheap to clone and pass
/// around as `Arc<PricingFile>`.
#[derive(Debug, Clone, Deserialize)]
pub struct PricingFile {
    pub schema_version: i64,

    /// Maintainer-set marker — `"production"` once values have been
    /// verified, otherwise (e.g.) `"needs_review"` or `"placeholder"`.
    /// Surfaced in the dashboard's banner.
    #[serde(default = "default_file_status")]
    pub file_status: String,

    #[serde(default)]
    pub providers: BTreeMap<String, ProviderPricing>,
}

fn default_file_status() -> String {
    "production".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderPricing {
    pub display_name: String,
    #[serde(default)]
    pub models: BTreeMap<String, ModelPricing>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelPricing {
    pub display_name: String,
    pub valid_from: String,
    pub input_usd_per_mtok: f64,
    pub output_usd_per_mtok: f64,
    pub cache_read_usd_per_mtok: f64,
    /// Multiplier on `input_usd_per_mtok`. Anthropic's published convention.
    pub cache_write_5m_multiplier: f64,
    pub cache_write_1h_multiplier: f64,
    pub source_url: String,
    pub source_accessed_at: String,
    #[serde(default)]
    pub notes: Option<String>,
}

impl PricingFile {
    /// Parse and validate a pricing TOML file from disk.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    /// Parse the pricing file embedded in the binary at compile time. Used
    /// when no configuration override is set, so a fresh install of
    /// `tokenscale` runs without the user having to copy a file.
    pub fn embedded_default() -> Result<Self> {
        Self::parse(EMBEDDED_PRICING_TOML)
    }

    /// Parse from a TOML string. Useful for tests and for tools that want
    /// to validate a file without touching disk.
    pub fn parse(raw_toml: &str) -> Result<Self> {
        let parsed: Self = toml::from_str(raw_toml)?;
        if !SUPPORTED_SCHEMA_RANGE.contains(&parsed.schema_version) {
            return Err(CoreError::UnsupportedSchemaVersion {
                found: parsed.schema_version,
                supported: format!(
                    "{}..={}",
                    SUPPORTED_SCHEMA_RANGE.start(),
                    SUPPORTED_SCHEMA_RANGE.end()
                ),
            });
        }
        Ok(parsed)
    }

    /// Fast lookup by `(provider, model)`. Returns `None` for any model not
    /// in the file — the dashboard treats this as "billable view unavailable
    /// for this model" rather than failing the whole response.
    #[must_use]
    pub fn lookup(&self, provider: &str, model: &str) -> Option<&ModelPricing> {
        self.providers.get(provider)?.models.get(model)
    }

    /// `true` if the maintainer has not yet reviewed the seed values. The
    /// dashboard surfaces this so users know the billable view is approximate.
    #[must_use]
    pub fn is_review_pending(&self) -> bool {
        self.file_status != "production"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_PRICING_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-opus-4-7"]
display_name = "Claude Opus 4.7"
valid_from = "2026-04-28"
input_usd_per_mtok = 15.00
output_usd_per_mtok = 75.00
cache_read_usd_per_mtok = 1.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "https://example.test/pricing"
source_accessed_at = "2026-04-28"
"#;

    #[test]
    fn valid_file_loads() {
        let parsed = PricingFile::parse(VALID_PRICING_TOML).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(!parsed.is_review_pending());
        let opus = parsed.lookup("anthropic", "claude-opus-4-7").unwrap();
        assert!((opus.input_usd_per_mtok - 15.00).abs() < f64::EPSILON);
        assert!((opus.cache_write_5m_multiplier - 1.25).abs() < f64::EPSILON);
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let bad = r#"
schema_version = 999
[providers.anthropic]
display_name = "Anthropic"
"#;
        let result = PricingFile::parse(bad);
        assert!(matches!(
            result,
            Err(CoreError::UnsupportedSchemaVersion { found: 999, .. })
        ));
    }

    #[test]
    fn lookup_returns_none_for_unknown_model() {
        let parsed = PricingFile::parse(VALID_PRICING_TOML).unwrap();
        assert!(parsed.lookup("anthropic", "claude-future-9-9").is_none());
        assert!(parsed.lookup("openai", "gpt-99").is_none());
    }

    #[test]
    fn missing_file_status_defaults_to_production() {
        let no_status = r#"
schema_version = 1
[providers.anthropic]
display_name = "Anthropic"
"#;
        let parsed = PricingFile::parse(no_status).unwrap();
        assert!(!parsed.is_review_pending());
    }

    #[test]
    fn needs_review_status_is_flagged() {
        let parsed = PricingFile::parse(
            r#"
schema_version = 1
file_status = "needs_review"
[providers.anthropic]
display_name = "Anthropic"
"#,
        )
        .unwrap();
        assert!(parsed.is_review_pending());
    }

    #[test]
    fn the_real_repo_pricing_file_loads() {
        // Sanity check: the pricing.toml committed at the repo root should
        // always parse cleanly under the current schema. This catches
        // accidental breakage when someone edits the file by hand.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("pricing.toml");
        let parsed = PricingFile::load_from_path(&path).unwrap();
        assert_eq!(parsed.schema_version, 1);
        // At least one Anthropic model must be present so the billable view
        // works on day one.
        let anthropic = parsed.providers.get("anthropic").expect("anthropic block");
        assert!(!anthropic.models.is_empty());
    }

    #[test]
    fn embedded_default_parses_cleanly() {
        // The embedded copy is the same file, captured at compile time.
        // Goal of this test: catch include_str! path drift.
        let parsed = PricingFile::embedded_default().unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(parsed
            .providers
            .get("anthropic")
            .is_some_and(|p| !p.models.is_empty()));
    }
}
