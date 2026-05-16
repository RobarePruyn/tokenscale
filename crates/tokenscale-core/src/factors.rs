//! Environmental-factor model — loaded from `environmental-factors.toml`.
//!
//! Mirrors the shape and discipline of `pricing.rs`:
//!
//! - A top-level `schema_version` integer guards compatibility. Loader
//!   refuses files outside the supported range.
//! - A `file_status` field flags whether the values have been merged in
//!   from Cowork research. Phase 1 ships with `"placeholder"` and every
//!   numeric field set to `null`.
//! - Every numeric value is `Option<f64>`; `null` means "not yet
//!   disclosed / not yet estimated." Per the kickoff prompt, the
//!   application MUST handle nulls gracefully — never invent a value to
//!   fill a gap.
//!
//! Phase 1 stops here: the loader and types exist, and the file is synced
//! into the `env_factors` / `grid_factors` tables on startup so historical
//! events resolve against versioned factors. Phase 2 is the per-event
//! impact computation (Google's August 2025 "comprehensive" methodology)
//! and the dashboard view that surfaces it.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::Path;

use crate::error::{CoreError, Result};

const SUPPORTED_SCHEMA_RANGE: RangeInclusive<i64> = 1..=1;

/// `environmental-factors.toml` from the repo root, embedded at compile
/// time. Used as the default when no override path is configured —
/// keeps `tokenscale serve` working on a fresh install.
const EMBEDDED_FACTORS_TOML: &str = include_str!("../../../environmental-factors.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct EnvironmentalFactorsFile {
    pub schema_version: i64,

    /// `"placeholder"` — every numeric value is `null` (Phase 1 default).
    /// `"production"` — values reviewed and merged (Phase 2 onwards).
    /// Other values treated as "review pending."
    #[serde(default = "default_file_status")]
    pub file_status: String,

    /// Human-readable file version — e.g., `"0.1"`. Surfaced through
    /// `/api/v1/health` so the dashboard can show "factors v0.1".
    #[serde(default)]
    pub file_version: Option<String>,

    /// ISO date the file was published by the maintainer. Independent of
    /// per-row `valid_from` — the file may be republished without rows
    /// changing their validity windows.
    #[serde(default)]
    pub file_published: Option<String>,

    /// Methodology identifier. Phase 2 expects
    /// `"google-comprehensive-aug-2025"`. The compute path may dispatch
    /// on this if/when alternative methodologies are supported.
    #[serde(default)]
    pub methodology: Option<String>,

    /// URL the methodology is sourced from (e.g., the Elsworth et al.
    /// arXiv link). Surfaced through health for the methodology page.
    #[serde(default)]
    pub methodology_source: Option<String>,

    #[serde(default)]
    pub providers: BTreeMap<String, ProviderFactors>,

    /// Indexed by region identifier — e.g., `"us-east-1"`.
    #[serde(default)]
    pub grid_factors: BTreeMap<String, GridFactors>,

    #[serde(default)]
    pub defaults: FactorDefaults,
}

fn default_file_status() -> String {
    "production".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderFactors {
    pub display_name: String,
    /// The cloud provider hosting this provider's inference. Joins to the
    /// grid_factors table for facility math.
    #[serde(default)]
    pub inference_provider: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, ModelFactors>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelFactors {
    pub display_name: String,

    /// When the model was released by the vendor (informational).
    #[serde(default)]
    pub released_at: Option<String>,

    /// ISO date the most recent source the value rests on was accessed.
    /// Versions the row in `env_factors`.
    #[serde(default)]
    pub valid_from: Option<String>,

    /// Anchor reference — `docs/sources.md#G.1` or a direct URL.
    #[serde(default)]
    pub source_doc: Option<String>,

    /// Wh per million tokens of each token type. `None` means "not
    /// disclosed / not estimated"; the application falls back to a
    /// per-provider default or skips the impact computation for that
    /// token type.
    #[serde(default)]
    pub wh_per_mtok_input: Option<f64>,
    #[serde(default)]
    pub wh_per_mtok_output: Option<f64>,
    #[serde(default)]
    pub wh_per_mtok_cache_read: Option<f64>,
    #[serde(default)]
    pub wh_per_mtok_cache_write_5m: Option<f64>,
    #[serde(default)]
    pub wh_per_mtok_cache_write_1h: Option<f64>,

    /// `"primary"` (vendor disclosure or peer-reviewed) /
    /// `"secondary"` (independent benchmark, blog, derivation) /
    /// `"superseded"`. v0.1 mostly populates this on every model.
    #[serde(default)]
    pub confidence: Option<String>,

    /// Honest uncertainty band, in percent. v0.1 populates this on every
    /// model — direct anchors are ±30%, derivations are ±35–60%. The API
    /// surfaces the per-bucket maximum so the dashboard can render
    /// "12.3 Wh ± 35%" rather than a false-precision point.
    #[serde(default)]
    pub uncertainty_range_pct: Option<i32>,

    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GridFactors {
    pub display_name: String,

    #[serde(default)]
    pub valid_from: Option<String>,

    /// kg CO₂e per kWh. The dominant factor in carbon math.
    #[serde(default)]
    pub co2e_kg_per_kwh: Option<f64>,

    /// Honest ± band on `co2e_kg_per_kwh`, in percent. Captures three
    /// sources of uncertainty in one number: year-to-year drift across
    /// eGRID releases (the EPA's own methodology variability),
    /// secular decarbonization between the eGRID year and the event
    /// date the dashboard is attributing emissions to, and the gap
    /// between the subregion-average value here and the specific
    /// datacenter's actual grid mix. Added in research-log entry
    /// 2026-05-12 (Sweep #1 — Grid-factor uncertainty bands).
    #[serde(default)]
    pub co2e_uncertainty_range_pct: Option<i32>,

    /// Liters of water per kWh — on-site DC cooling (scope-1 in
    /// Ren et al.'s framing). Bounded by the WUE the datacenter
    /// operator publishes for its own facility.
    #[serde(default)]
    pub water_l_per_kwh: Option<f64>,

    /// Honest ± band on `water_l_per_kwh`. AWS publishes only a global
    /// WUE figure; we apply it flat across all AWS regions. This
    /// percentage reflects the "global-to-regional" gap — a specific
    /// datacenter's real water draw can differ substantially from the
    /// fleetwide average, especially in arid regions vs. wet ones.
    #[serde(default)]
    pub water_uncertainty_range_pct: Option<i32>,

    /// **Indirect** (off-site, power-plant cooling) water per kWh of
    /// electricity drawn — scope-2 water in Ren et al. 2024 "Making AI
    /// Less Thirsty". This is the water consumed at the generating
    /// plants whose electricity the datacenter pulls, derived from the
    /// regional fuel-mix × per-fuel water coefficients (Macknick 2012).
    /// For thermoelectric-heavy grids it's typically 10×–60× larger than
    /// the on-site WUE; for renewable-heavy grids it's much smaller.
    /// Surfaces in the dashboard via the "Include indirect water" toggle.
    #[serde(default)]
    pub indirect_water_l_per_kwh: Option<f64>,

    /// Honest ± band on `indirect_water_l_per_kwh`. The hydro-attribution
    /// methodology dominates this for hydro-heavy regions (Macknick's
    /// reservoir-evaporation figures are contested 5×–10×); thermoelectric
    /// coefficients are well-characterized, so coal/gas/nuclear-dominated
    /// grids carry a smaller band.
    #[serde(default)]
    pub indirect_water_uncertainty_range_pct: Option<i32>,

    /// Power Usage Effectiveness — facility energy / IT energy. Multiplier
    /// applied AFTER per-token energy to get total facility energy.
    #[serde(default)]
    pub pue: Option<f64>,

    /// EPA eGRID subregion code — e.g. `"SRVC"` for SERC Virginia/Carolina
    /// (us-east-1). Surfaced through `/api/v1/health` so users can
    /// understand which grid is being attributed to their usage.
    #[serde(default)]
    pub egrid_subregion: Option<String>,
    #[serde(default)]
    pub egrid_subregion_full_name: Option<String>,

    #[serde(default)]
    pub source_url_co2e: Option<String>,
    #[serde(default)]
    pub source_url_water: Option<String>,
    #[serde(default)]
    pub source_url_indirect_water: Option<String>,
    #[serde(default)]
    pub source_url_pue: Option<String>,

    #[serde(default)]
    pub source_accessed_at: Option<String>,

    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FactorDefaults {
    /// Used when a region's `pue` is null. Typically a vendor's published
    /// global PUE (e.g., AWS 1.15 for 2024).
    #[serde(default)]
    pub fallback_pue: Option<f64>,

    /// Used when a region's `water_l_per_kwh` is null. Typically a
    /// vendor's published global WUE (e.g., AWS 0.15 L/kWh for 2024).
    #[serde(default)]
    pub fallback_wue_l_per_kwh: Option<f64>,

    /// Identifier for the methodology the values follow. Duplicated from
    /// the top-level for backward compatibility — top-level is the
    /// canonical location post-v0.1.
    #[serde(default)]
    pub methodology: Option<String>,
}

impl EnvironmentalFactorsFile {
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    pub fn parse(raw_toml: &str) -> Result<Self> {
        let preprocessed = rewrite_null_assignments(raw_toml);
        let parsed: Self = toml::from_str(&preprocessed)?;
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

    /// Parse the file embedded in the binary. Used when no override path
    /// is configured.
    pub fn embedded_default() -> Result<Self> {
        Self::parse(EMBEDDED_FACTORS_TOML)
    }

    #[must_use]
    pub fn lookup_model(&self, provider: &str, model: &str) -> Option<&ModelFactors> {
        self.providers.get(provider)?.models.get(model)
    }

    #[must_use]
    pub fn lookup_grid(&self, region: &str) -> Option<&GridFactors> {
        self.grid_factors.get(region)
    }

    /// `true` when the file is explicitly a placeholder. Phase 1 ships
    /// with this; Phase 2 lights up when the maintainer flips
    /// `file_status` to `"production"` after merging Cowork's values.
    #[must_use]
    pub fn is_placeholder(&self) -> bool {
        self.file_status == "placeholder"
    }

    /// `true` when the file isn't `"production"` — placeholder or anything
    /// else the maintainer used to flag review-pending. The dashboard's
    /// future "Impact" view will show this prominently so users know the
    /// numbers are not yet authoritative.
    #[must_use]
    pub fn is_review_pending(&self) -> bool {
        self.file_status != "production"
    }

    /// Most recent access date across grid factors. Each model also has
    /// a `valid_from` but that's more of a "this row is valid since X"
    /// version — `source_accessed_at` is the "I checked the source on
    /// this date" stamp the dashboard surfaces.
    #[must_use]
    pub fn most_recent_grid_accessed_at(&self) -> Option<&str> {
        self.grid_factors
            .values()
            .filter_map(|grid| grid.source_accessed_at.as_deref())
            .max()
    }

    /// Number of `(provider, model)` factor entries currently loaded.
    #[must_use]
    pub fn model_count(&self) -> usize {
        self.providers
            .values()
            .map(|provider| provider.models.len())
            .sum()
    }

    #[must_use]
    pub fn region_count(&self) -> usize {
        self.grid_factors.len()
    }

    /// Effective fallback PUE — the file's `defaults.fallback_pue` if
    /// present, else `1.0` (the "no facility overhead" identity). Used
    /// by `compute_impact` when a region's `pue` is null.
    #[must_use]
    pub fn effective_fallback_pue(&self) -> f64 {
        self.defaults.fallback_pue.unwrap_or(1.0)
    }

    /// Effective fallback WUE in L/kWh. Returns `None` when neither the
    /// region nor the defaults block carries a value — `compute_impact`
    /// surfaces water as `Option<f64>` rather than substituting a sentinel.
    #[must_use]
    pub fn effective_fallback_wue_l_per_kwh(&self) -> Option<f64> {
        self.defaults.fallback_wue_l_per_kwh
    }
}

/// TOML 1.0 doesn't have a `null` literal, but the
/// `environmental-factors.toml` convention from the kickoff prompt uses
/// `wh_per_mtok_input = null` to mean "explicitly unknown, populated
/// later." We honor that by rewriting any `key = null [# comment]` line
/// to a comment before handing off to the TOML parser. Missing keys
/// then surface as `None` through `#[serde(default)]` on the struct
/// fields — which is exactly the semantics the maintainer intended.
///
/// The rewrite is line-oriented and strict: only lines whose right-hand
/// side is `null` (optionally followed by a comment) are touched. Any
/// `null` appearing inside a string literal is left alone because we
/// only consider identifier-style left-hand sides; we never alter
/// content inside `"..."`.
fn rewrite_null_assignments(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            // Skip lines that are already comments or empty.
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return line.to_owned();
            }
            let Some(equals_pos) = trimmed.find('=') else {
                return line.to_owned();
            };
            let right_hand_side = trimmed[equals_pos + 1..].trim_start();
            let Some(after_null) = right_hand_side.strip_prefix("null") else {
                return line.to_owned();
            };
            let after_null_trimmed = after_null.trim_start();
            // Accept `null`, `null # comment`, or `null` at end of line.
            if after_null_trimmed.is_empty() || after_null_trimmed.starts_with('#') {
                return format!("# {line}");
            }
            line.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOML: &str = r#"
schema_version = 1
file_status = "placeholder"
file_version = "0.0-test"
file_published = "2026-04-28"
methodology = "google-comprehensive-aug-2025"
methodology_source = "https://arxiv.org/abs/2508.15734"

[providers.anthropic]
display_name = "Anthropic"
inference_provider = "aws"

[providers.anthropic.models."claude-opus-4-7"]
display_name = "Claude Opus 4.7"
valid_from = "2026-04-28"
source_doc = "docs/sources.md#G.1"
confidence = "secondary"
uncertainty_range_pct = 40
wh_per_mtok_input = null
wh_per_mtok_output = null
wh_per_mtok_cache_read = null
wh_per_mtok_cache_write_5m = null
wh_per_mtok_cache_write_1h = null

[grid_factors."us-east-1"]
display_name = "AWS US East (N. Virginia)"
valid_from = "2026-04-28"
egrid_subregion = "SRVC"
egrid_subregion_full_name = "SERC Virginia/Carolina"
co2e_kg_per_kwh = null
water_l_per_kwh = null
pue = null
source_accessed_at = "2026-04-28"

[grid_factors."us-west-2"]
display_name = "AWS US West (Oregon)"
co2e_kg_per_kwh = null
source_accessed_at = "2026-03-15"

[defaults]
fallback_pue = 1.15
fallback_wue_l_per_kwh = 0.15
methodology = "google-comprehensive-aug-2025"
"#;

    #[test]
    fn placeholder_file_loads_with_all_nulls() {
        let parsed = EnvironmentalFactorsFile::parse(VALID_TOML).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(parsed.is_placeholder());
        assert!(parsed.is_review_pending());

        let opus = parsed.lookup_model("anthropic", "claude-opus-4-7").unwrap();
        assert_eq!(opus.display_name, "Claude Opus 4.7");
        assert!(opus.wh_per_mtok_input.is_none()); // null in source

        let grid = parsed.lookup_grid("us-east-1").unwrap();
        assert!(grid.pue.is_none());
        assert_eq!(grid.source_accessed_at.as_deref(), Some("2026-04-28"));

        assert!((parsed.defaults.fallback_pue.unwrap() - 1.15).abs() < f64::EPSILON);
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let bad = "schema_version = 99";
        let result = EnvironmentalFactorsFile::parse(bad);
        assert!(matches!(
            result,
            Err(CoreError::UnsupportedSchemaVersion { found: 99, .. })
        ));
    }

    #[test]
    fn lookups_return_none_for_unknown_keys() {
        let parsed = EnvironmentalFactorsFile::parse(VALID_TOML).unwrap();
        assert!(parsed
            .lookup_model("anthropic", "claude-future-9-9")
            .is_none());
        assert!(parsed.lookup_grid("eu-west-1").is_none());
    }

    #[test]
    fn most_recent_grid_accessed_at_returns_lex_max() {
        let parsed = EnvironmentalFactorsFile::parse(VALID_TOML).unwrap();
        // The fixture has us-east-1=2026-04-28 and us-west-2=2026-03-15.
        assert_eq!(parsed.most_recent_grid_accessed_at(), Some("2026-04-28"));
    }

    #[test]
    fn model_and_region_counts_match_fixture() {
        let parsed = EnvironmentalFactorsFile::parse(VALID_TOML).unwrap();
        assert_eq!(parsed.model_count(), 1);
        assert_eq!(parsed.region_count(), 2);
    }

    #[test]
    fn v0_1_phase2_fields_round_trip() {
        // Confirms the loader exposes Phase 2 fields the API + migration
        // depend on: confidence + uncertainty_range_pct on models;
        // egrid_subregion + full name on grid; file_version /
        // file_published / methodology / methodology_source at the top
        // level; fallback_wue_l_per_kwh in defaults.
        let parsed = EnvironmentalFactorsFile::parse(VALID_TOML).unwrap();

        assert_eq!(parsed.file_version.as_deref(), Some("0.0-test"));
        assert_eq!(parsed.file_published.as_deref(), Some("2026-04-28"));
        assert_eq!(
            parsed.methodology.as_deref(),
            Some("google-comprehensive-aug-2025")
        );
        assert_eq!(
            parsed.methodology_source.as_deref(),
            Some("https://arxiv.org/abs/2508.15734")
        );

        let opus = parsed.lookup_model("anthropic", "claude-opus-4-7").unwrap();
        assert_eq!(opus.confidence.as_deref(), Some("secondary"));
        assert_eq!(opus.uncertainty_range_pct, Some(40));

        let grid = parsed.lookup_grid("us-east-1").unwrap();
        assert_eq!(grid.egrid_subregion.as_deref(), Some("SRVC"));
        assert_eq!(
            grid.egrid_subregion_full_name.as_deref(),
            Some("SERC Virginia/Carolina")
        );

        assert!(
            (parsed.effective_fallback_pue() - 1.15).abs() < f64::EPSILON,
            "fallback_pue defaults to 1.15 in fixture"
        );
        assert_eq!(
            parsed.effective_fallback_wue_l_per_kwh(),
            Some(0.15),
            "fallback_wue_l_per_kwh exposed via defaults"
        );
    }

    #[test]
    fn v0_1_production_repo_file_loads_with_phase2_fields() {
        // The committed environmental-factors.toml should always carry the
        // Phase 2 fields the API and the dashboard depend on. Catches
        // accidental shape regressions in v0.x file edits.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("environmental-factors.toml");
        let parsed = EnvironmentalFactorsFile::load_from_path(&path).unwrap();
        // file_version is required from v0.1 onward
        assert!(
            parsed.file_version.is_some(),
            "file_version must be populated from v0.1 onward"
        );
        assert!(parsed.methodology.is_some());
        assert!(
            parsed
                .lookup_grid("us-east-1")
                .and_then(|g| g.egrid_subregion.clone())
                .is_some(),
            "us-east-1 must have egrid_subregion populated"
        );
    }

    #[test]
    fn missing_file_status_defaults_to_production() {
        let no_status = "schema_version = 1\n[providers.anthropic]\ndisplay_name = \"x\"\n";
        let parsed = EnvironmentalFactorsFile::parse(no_status).unwrap();
        assert!(!parsed.is_placeholder());
        assert!(!parsed.is_review_pending());
    }

    #[test]
    fn the_real_repo_factors_file_loads() {
        // Catches accidental shape breakage when someone hand-edits
        // the staged `environmental-factors.toml`.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("environmental-factors.toml");
        let parsed = EnvironmentalFactorsFile::load_from_path(&path).unwrap();
        assert_eq!(parsed.schema_version, 1);
        // The repo file flipped from placeholder to production when Cowork's
        // v0.1 values landed (see docs/research-log.md, 2026-04-28). Keep
        // this assertion narrow — schema parses, providers and grid_factors
        // are populated — without locking to a specific file_status, since
        // the maintainer may toggle that during local edits.
        assert!(!parsed.providers.is_empty());
        assert!(!parsed.grid_factors.is_empty());
    }

    #[test]
    fn embedded_default_parses_cleanly() {
        let parsed = EnvironmentalFactorsFile::embedded_default().unwrap();
        assert_eq!(parsed.schema_version, 1);
    }

    #[test]
    fn null_assignments_are_rewritten_to_comments() {
        let input = "\
key1 = 1.5
key2 = null
key3 = null  # because reasons
key4 = \"keep null in strings\"
# key5 = null
";
        let rewritten = rewrite_null_assignments(input);
        // `key1` left alone
        assert!(rewritten.contains("key1 = 1.5"));
        // `key2 = null` → comment
        assert!(rewritten.contains("# key2 = null"));
        // `key3 = null # ...` → comment, preserving trailing remark
        assert!(rewritten.contains("# key3 = null  # because reasons"));
        // String literals untouched
        assert!(rewritten.contains("key4 = \"keep null in strings\""));
        // Already-commented lines untouched
        assert!(rewritten.contains("# key5 = null"));
    }
}
