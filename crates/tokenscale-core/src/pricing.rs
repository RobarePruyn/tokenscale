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
///
/// **Multi-row support (v0.1.13)**: each `(provider, model)` key maps to
/// a `Vec<ModelPricing>` ordered by `valid_from` ascending. Time-anchored
/// lookup walks descending to find the latest row whose `valid_from` is
/// at-or-before the caller's `as_of_date`. Mirrors the env-side pattern
/// in `lookup_environmental_factors`.
///
/// TOML supports both the single-table form (legacy, one row per model):
///
/// ```toml
/// [providers.anthropic.models."claude-opus-4-7"]
/// valid_from = "2026-01-15"
/// input_usd_per_mtok = 5.00
/// # ...
/// ```
///
/// and the array-of-tables form (multi-row, for models with historical
/// price changes):
///
/// ```toml
/// [[providers.anthropic.models."claude-opus-4-7"]]
/// valid_from = "2026-01-15"
/// input_usd_per_mtok = 5.00
///
/// [[providers.anthropic.models."claude-opus-4-7"]]
/// valid_from = "2027-XX-XX"
/// input_usd_per_mtok = X.XX
/// ```
///
/// Both forms normalize to `Vec<ModelPricing>` at parse time. Legacy
/// single-row pricing files from v0.1.12 and earlier continue to load
/// without modification — the additive support is fully back-compat.
#[derive(Debug, Clone)]
pub struct PricingFile {
    pub schema_version: i64,

    /// Maintainer-set marker — `"production"` once values have been
    /// verified, otherwise (e.g.) `"needs_review"` or `"placeholder"`.
    /// Surfaced in the dashboard's banner.
    pub file_status: String,

    /// Human-readable file version — e.g. `"1.0"`. Mirrors the env-side
    /// `EnvironmentalFactorsFile::file_version`. Surfaced through
    /// `/api/v1/health` so the dashboard can show "pricing v1.0" next
    /// to the env factor version. `None` for files that pre-date the
    /// v0.1.13 introduction of this field.
    pub file_version: Option<String>,

    /// ISO `YYYY-MM-DD` of when the maintainer published this version of
    /// the file. Independent of per-row `valid_from` — the file may be
    /// republished without rows changing their validity windows.
    /// Surfaced through `/api/v1/health` for the dashboard's banner.
    pub file_published: Option<String>,

    pub providers: BTreeMap<String, ProviderPricing>,
}

fn default_file_status() -> String {
    "production".to_owned()
}

#[derive(Debug, Clone)]
pub struct ProviderPricing {
    pub display_name: String,
    pub models: BTreeMap<String, Vec<ModelPricing>>,
    /// D1 (v0.1.20): alias model ID -> canonical row key. Real emitted IDs
    /// vary in form (dated snapshots before the 4.6 generation, dateless
    /// from 4.6 on, convenience aliases like `claude-sonnet-4-5` that
    /// resolve to a dated snapshot). Every resolution site maps through
    /// this before matching a row; `events.model` itself stays raw.
    pub aliases: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// On-disk TOML deserialization shapes. Public types above are the runtime
// normalized form; these private types match what `pricing.toml` actually
// looks like on disk and route through serde's `untagged` enum so the
// loader accepts either single-table or array-of-tables for any given
// model row.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PricingFileToml {
    schema_version: i64,
    #[serde(default = "default_file_status")]
    file_status: String,
    #[serde(default)]
    file_version: Option<String>,
    #[serde(default)]
    file_published: Option<String>,
    #[serde(default)]
    providers: BTreeMap<String, ProviderPricingToml>,
}

#[derive(Debug, Deserialize)]
struct ProviderPricingToml {
    display_name: String,
    #[serde(default)]
    models: BTreeMap<String, ModelPricingEntry>,
    /// `[providers.<provider>.aliases]` table: `"raw-id" = "canonical-id"`.
    #[serde(default)]
    aliases: BTreeMap<String, String>,
}

/// Each model entry is either a single `ModelPricing` (legacy single-row
/// form) or a `Vec<ModelPricing>` (multi-row form). serde's `untagged`
/// tries `Single` first; if the on-disk shape is an array of tables, the
/// fallback to `Multi` succeeds.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ModelPricingEntry {
    Single(Box<ModelPricing>),
    Multi(Vec<ModelPricing>),
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
    /// Provenance for `valid_from`: the URL or citation that backs the
    /// launch-date claim, so future maintainers can re-verify. v0.1.13
    /// introduced this alongside multi-row time-anchored entries — every
    /// new row should populate it. Older rows that pre-date v0.1.13's
    /// time-anchoring (and the lazy v0.1.0 `valid_from = "2026-04-28"`
    /// rows) carry `None`; the audit subcommand flags those.
    #[serde(default)]
    pub launch_date_source: Option<String>,
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
        let toml_form: PricingFileToml = toml::from_str(raw_toml)?;
        if !SUPPORTED_SCHEMA_RANGE.contains(&toml_form.schema_version) {
            return Err(CoreError::UnsupportedSchemaVersion {
                found: toml_form.schema_version,
                supported: format!(
                    "{}..={}",
                    SUPPORTED_SCHEMA_RANGE.start(),
                    SUPPORTED_SCHEMA_RANGE.end()
                ),
            });
        }

        // Normalize the on-disk shape (single-table OR array-of-tables per
        // model entry) into the runtime shape (Vec<ModelPricing> per model,
        // sorted by valid_from ascending). Sorting at parse time means the
        // lookup walk is just `iter().rev()` — no per-call sort.
        let mut providers: BTreeMap<String, ProviderPricing> = BTreeMap::new();
        for (provider_id, provider_toml) in toml_form.providers {
            let mut models: BTreeMap<String, Vec<ModelPricing>> = BTreeMap::new();
            for (model_id, entry) in provider_toml.models {
                let mut rows = match entry {
                    ModelPricingEntry::Single(boxed) => vec![*boxed],
                    ModelPricingEntry::Multi(rows) => rows,
                };
                rows.sort_by(|a, b| a.valid_from.cmp(&b.valid_from));
                models.insert(model_id, rows);
            }
            providers.insert(
                provider_id,
                ProviderPricing {
                    display_name: provider_toml.display_name,
                    models,
                    aliases: provider_toml.aliases,
                },
            );
        }

        Ok(PricingFile {
            schema_version: toml_form.schema_version,
            file_status: toml_form.file_status,
            file_version: toml_form.file_version,
            file_published: toml_form.file_published,
            providers,
        })
    }

    /// D1: resolve a raw model ID to its canonical row key through the
    /// provider's alias table. Unknown IDs and canonical IDs pass through
    /// unchanged, so callers can apply this unconditionally before any
    /// structural `contains_key` check or lookup. The DB-side joins
    /// apply the same mapping via the `model_aliases` table.
    #[must_use]
    pub fn canonical_model<'a>(&'a self, provider: &str, model: &'a str) -> &'a str {
        self.providers
            .get(provider)
            .and_then(|p| p.aliases.get(model))
            .map_or(model, String::as_str)
    }

    /// D1 startup guard: an alias whose raw key is ALSO a model row key
    /// would make resolution self-referential (which row wins?). Returns
    /// every such conflict as `"provider/raw"`; empty means the file is
    /// consistent. The CLI refuses to serve on a non-empty result.
    #[must_use]
    pub fn alias_conflicts(&self) -> Vec<String> {
        let mut conflicts = Vec::new();
        for (provider_id, provider) in &self.providers {
            for raw in provider.aliases.keys() {
                if provider.models.contains_key(raw) {
                    conflicts.push(format!("{provider_id}/{raw}"));
                }
            }
        }
        conflicts
    }

    /// Time-anchored lookup mirroring `lookup_environmental_factors` on the
    /// env side. Returns the row whose `valid_from <= as_of_date` is latest,
    /// or `None` if no row qualifies (e.g. the event predates the model's
    /// launch — see D4 in `docs/roadmap-cost-time-anchoring.md`).
    ///
    /// `as_of_date` should be ISO `YYYY-MM-DD`. The comparison is lexical,
    /// matching the env-side convention.
    #[must_use]
    pub fn lookup(
        &self,
        provider: &str,
        model: &str,
        as_of_date: &str,
    ) -> Option<&ModelPricing> {
        let provider_pricing = self.providers.get(provider)?;
        let canonical = provider_pricing
            .aliases
            .get(model)
            .map_or(model, String::as_str);
        let rows = provider_pricing.models.get(canonical)?;
        // Rows are sorted ascending by valid_from at parse time, so walking
        // in reverse yields the latest qualifying row first.
        rows.iter()
            .rev()
            .find(|p| p.valid_from.as_str() <= as_of_date)
    }

    /// Total count of priced model rows across all providers — used by the
    /// `/api/v1/health` endpoint. A model with multiple `valid_from` rows
    /// counts once per row, mirroring `env_factors`'s per-row counting.
    #[must_use]
    pub fn model_row_count(&self) -> usize {
        self.providers
            .values()
            .map(|p| p.models.values().map(Vec::len).sum::<usize>())
            .sum()
    }

    /// Count of distinct `(provider, model)` keys — used wherever the
    /// dashboard wants "how many models the file covers" regardless of
    /// version history. Distinct from `model_row_count`.
    #[must_use]
    pub fn distinct_model_count(&self) -> usize {
        self.providers.values().map(|p| p.models.len()).sum()
    }

    /// `true` if the maintainer has not yet reviewed the seed values. The
    /// dashboard surfaces this so users know the billable view is approximate.
    #[must_use]
    pub fn is_review_pending(&self) -> bool {
        self.file_status != "production"
    }

    /// `true` if any row's `notes` field still carries a seed / assumption
    /// marker. Companion to `is_review_pending` — the v0.1.0–v0.1.10 bug
    /// shipped because `file_status` was the only gate, and a row's
    /// `notes = "Seed value — pricing assumed..."` text was never checked
    /// against anything. Now the startup gate refuses both conditions.
    ///
    /// Match is case-insensitive substring against PHRASE-level markers,
    /// not bare words, to avoid false-positives on legitimate methodology
    /// prose. (e.g. an env-factors row that disclosed "Medium response
    /// **assumed** at 1,500-2,000 tokens" is honest disclosure, not a
    /// seed-value bug.) The phrase list is the actual bug pattern:
    /// - `"seed value"` — the literal text the buggy rows carried.
    /// - `"unverified"` — common shorthand for "needs review."
    /// - `"needs_review"` — the file_status string repeated in a note.
    /// - `"assumed unchanged"` — the specific cascading-assumption phrase
    ///   that produced the Opus seed bug.
    #[must_use]
    pub fn has_seed_markers(&self) -> bool {
        const MARKERS: &[&str] = &["seed value", "unverified", "needs_review", "assumed unchanged"];
        for provider in self.providers.values() {
            for rows in provider.models.values() {
                for model in rows {
                    if let Some(notes) = &model.notes {
                        let lower = notes.to_lowercase();
                        if MARKERS.iter().any(|m| lower.contains(m)) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// The most recent `source_accessed_at` across all priced models in
    /// the file. `None` for an empty file. ISO dates sort lexically so
    /// `max()` returns the most recent. Used by the dashboard to surface
    /// "pricing as of YYYY-MM-DD" — informative even when the file has
    /// not yet been re-verified.
    #[must_use]
    pub fn most_recent_accessed_at(&self) -> Option<&str> {
        self.providers
            .values()
            .flat_map(|provider| provider.models.values().flat_map(|rows| rows.iter()))
            .map(|model| model.source_accessed_at.as_str())
            .max()
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

    /// Sentinel "any future date" used to mean "give me the latest row" —
    /// for lookups that don't care about historical anchoring (e.g. the
    /// /api/v1/factors/active provenance endpoint).
    const ANY_FUTURE_DATE: &str = "9999-12-31";

    #[test]
    fn valid_file_loads_single_row_form() {
        let parsed = PricingFile::parse(VALID_PRICING_TOML).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(!parsed.is_review_pending());
        let opus = parsed
            .lookup("anthropic", "claude-opus-4-7", ANY_FUTURE_DATE)
            .unwrap();
        assert!((opus.input_usd_per_mtok - 15.00).abs() < f64::EPSILON);
        assert!((opus.cache_write_5m_multiplier - 1.25).abs() < f64::EPSILON);
        // Single-row TOML normalizes to a one-element Vec.
        assert_eq!(
            parsed.providers["anthropic"].models["claude-opus-4-7"].len(),
            1
        );
    }

    /// The D7 back-compat claim: v0.1.12's single-row `pricing.toml` form
    /// (`[providers.<p>.models.<m>]`) still parses under v0.1.13's loader.
    /// This is the regression guard for downgrade-after-upgrade and for
    /// users who haven't yet migrated their pricing file to multi-row.
    #[test]
    fn back_compat_v0_1_12_single_row_form_still_parses() {
        // Exact shape v0.1.12 shipped, modulo the rates which were the
        // bug we're past now. Verifies the *loader*, not the contents.
        let v0_1_12_form = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-opus-4-7"]
display_name              = "Claude Opus 4.7"
valid_from                = "2026-04-28"
input_usd_per_mtok        = 5.00
output_usd_per_mtok       = 25.00
cache_read_usd_per_mtok   = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at        = "2026-05-18"
"#;
        let parsed = PricingFile::parse(v0_1_12_form).unwrap();
        let opus = parsed
            .lookup("anthropic", "claude-opus-4-7", ANY_FUTURE_DATE)
            .unwrap();
        assert!((opus.input_usd_per_mtok - 5.00).abs() < f64::EPSILON);
    }

    /// The v0.1.13 multi-row form: same model, multiple rows ordered by
    /// `valid_from`. Lookup must walk descending and pick the latest row
    /// whose `valid_from <= as_of_date`.
    #[test]
    fn multi_row_array_of_tables_form_parses() {
        let multi_row = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[[providers.anthropic.models."claude-opus-4-7"]]
display_name              = "Claude Opus 4.7"
valid_from                = "2026-01-15"
input_usd_per_mtok        = 5.00
output_usd_per_mtok       = 25.00
cache_read_usd_per_mtok   = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at        = "2026-05-18"

[[providers.anthropic.models."claude-opus-4-7"]]
display_name              = "Claude Opus 4.7"
valid_from                = "2027-06-01"
input_usd_per_mtok        = 7.00
output_usd_per_mtok       = 35.00
cache_read_usd_per_mtok   = 0.70
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at        = "2027-06-01"
"#;
        let parsed = PricingFile::parse(multi_row).unwrap();
        assert_eq!(
            parsed.providers["anthropic"].models["claude-opus-4-7"].len(),
            2,
            "multi-row TOML should normalize to a Vec of length 2"
        );

        // Event in February 2026 → first row.
        let early = parsed
            .lookup("anthropic", "claude-opus-4-7", "2026-02-10")
            .unwrap();
        assert!((early.input_usd_per_mtok - 5.00).abs() < f64::EPSILON);

        // Event in July 2027 → second row.
        let later = parsed
            .lookup("anthropic", "claude-opus-4-7", "2027-07-15")
            .unwrap();
        assert!((later.input_usd_per_mtok - 7.00).abs() < f64::EPSILON);
    }

    /// Boundary semantics: env-side uses `valid_from <= occurred_at`.
    /// Pricing must match. An event AT exactly the `valid_from` timestamp
    /// gets the new row; an event one day before gets the previous row.
    #[test]
    fn lookup_boundary_includes_the_valid_from_date() {
        let multi_row = r#"
schema_version = 1
file_status = "production"
[providers.anthropic]
display_name = "Anthropic"
[[providers.anthropic.models."claude-opus-4-7"]]
display_name = "Claude Opus 4.7"
valid_from = "2026-01-15"
input_usd_per_mtok = 5.00
output_usd_per_mtok = 25.00
cache_read_usd_per_mtok = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "https://example"
source_accessed_at = "2026-05-18"
[[providers.anthropic.models."claude-opus-4-7"]]
display_name = "Claude Opus 4.7"
valid_from = "2027-06-01"
input_usd_per_mtok = 7.00
output_usd_per_mtok = 35.00
cache_read_usd_per_mtok = 0.70
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "https://example"
source_accessed_at = "2027-06-01"
"#;
        let parsed = PricingFile::parse(multi_row).unwrap();
        // Day before the boundary → old row.
        let before = parsed
            .lookup("anthropic", "claude-opus-4-7", "2027-05-31")
            .unwrap();
        assert!((before.input_usd_per_mtok - 5.00).abs() < f64::EPSILON);
        // Exactly on the boundary → new row.
        let on = parsed
            .lookup("anthropic", "claude-opus-4-7", "2027-06-01")
            .unwrap();
        assert!((on.input_usd_per_mtok - 7.00).abs() < f64::EPSILON);
        // Day after → still new row.
        let after = parsed
            .lookup("anthropic", "claude-opus-4-7", "2027-06-02")
            .unwrap();
        assert!((after.input_usd_per_mtok - 7.00).abs() < f64::EPSILON);
    }

    /// D4: a pre-launch event returns None. The dashboard renders these
    /// as "—" rather than fabricating a fallback rate.
    #[test]
    fn pre_launch_event_returns_none() {
        let parsed = PricingFile::parse(VALID_PRICING_TOML).unwrap();
        // VALID_PRICING_TOML has Opus valid_from = 2026-04-28. An event
        // dated before that should not match.
        let pre_launch = parsed.lookup("anthropic", "claude-opus-4-7", "2026-01-01");
        assert!(pre_launch.is_none());
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
        assert!(parsed
            .lookup("anthropic", "claude-future-9-9", ANY_FUTURE_DATE)
            .is_none());
        assert!(parsed
            .lookup("openai", "gpt-99", ANY_FUTURE_DATE)
            .is_none());
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

    /// Pins the production gate against the real `pricing.toml`. If
    /// someone reintroduces `file_status = "needs_review"` or pastes a
    /// row with a seed-marker note, this test fails before the binary
    /// builds — same shape of fence as the startup `anyhow::bail!` but
    /// caught at `cargo test` time.
    #[test]
    fn the_real_repo_pricing_file_passes_production_gate() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("pricing.toml");
        let parsed = PricingFile::load_from_path(&path).unwrap();
        assert!(
            !parsed.is_review_pending(),
            "pricing.toml file_status = {:?} — must be \"production\". This is the v0.1.0–v0.1.10 \
             bug guard. See docs/cost-methodology.md.",
            parsed.file_status,
        );
        assert!(
            !parsed.has_seed_markers(),
            "pricing.toml has a row whose `notes` carries a seed/assumption marker. Verify \
             the row's rates against Anthropic's pricing page and rewrite/remove the notes field."
        );
    }

    #[test]
    fn has_seed_markers_detects_each_phrase() {
        let tmpl = |notes: &str| {
            format!(
                r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-opus-4-7"]
display_name = "Claude Opus 4.7"
valid_from = "2026-04-28"
input_usd_per_mtok = 5.00
output_usd_per_mtok = 25.00
cache_read_usd_per_mtok = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at = "2026-05-18"
notes = "{notes}"
"#
            )
        };
        // Each phrase should trip the gate. Case-insensitive.
        for marker in &["Seed value", "unverified", "needs_review", "assumed unchanged"] {
            let toml = tmpl(&format!("This row has a {marker} marker."));
            let parsed = PricingFile::parse(&toml).unwrap();
            assert!(
                parsed.has_seed_markers(),
                "phrase {:?} should be detected",
                marker
            );
        }
        // The actual v0.1.0–v0.1.10 buggy phrasing — the bug guard, end-to-end:
        let buggy = tmpl("Seed value — pricing assumed unchanged from Opus 4 family. Verify.");
        assert!(
            PricingFile::parse(&buggy).unwrap().has_seed_markers(),
            "the literal v0.1.0–v0.1.10 buggy notes text must trip the gate"
        );
        // Legitimate methodology prose using "assumed" or "verify" as bare
        // words must NOT trip the gate (the env-factors file has rows like
        // "Medium response assumed at 1,500-2,000 tokens" that are honest
        // disclosure, not seed bugs).
        let legit_assumed = tmpl("Medium response assumed at 1,500-2,000 tokens per Jegham v6.");
        assert!(
            !PricingFile::parse(&legit_assumed).unwrap().has_seed_markers(),
            "bare \"assumed\" in methodology prose must NOT trip the gate"
        );
        let legit_verify = tmpl("Verified against Anthropic docs 2026-05-18.");
        assert!(
            !PricingFile::parse(&legit_verify).unwrap().has_seed_markers(),
            "bare \"verified\" in clean-state notes must NOT trip the gate"
        );
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

#[cfg(test)]
mod alias_tests {
    use super::*;

    const ALIASED: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.aliases]
"claude-haiku-4-5" = "claude-haiku-4-5-20251001"
"claude-sonnet-4-5" = "claude-sonnet-4-5-20250929"

[[providers.anthropic.models."claude-haiku-4-5-20251001"]]
display_name = "Claude Haiku 4.5"
valid_from = "2025-10-01"
input_usd_per_mtok = 1.00
output_usd_per_mtok = 5.00
cache_read_usd_per_mtok = 0.10
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "https://example.test"
source_accessed_at = "2026-08-02"
"#;

    #[test]
    fn alias_resolves_to_canonical_row_in_lookup() {
        let f = PricingFile::parse(ALIASED).unwrap();
        // Undated alias resolves to the dated canonical row.
        let via_alias = f.lookup("anthropic", "claude-haiku-4-5", "2026-01-01");
        let direct = f.lookup("anthropic", "claude-haiku-4-5-20251001", "2026-01-01");
        assert!(via_alias.is_some());
        assert!((via_alias.unwrap().input_usd_per_mtok - direct.unwrap().input_usd_per_mtok).abs() < f64::EPSILON);
        // Alias to a model with no rows (sonnet-4-5 here) resolves to None, not a panic.
        assert!(f.lookup("anthropic", "claude-sonnet-4-5", "2026-01-01").is_none());
    }

    #[test]
    fn canonical_model_passes_unknown_and_canonical_through() {
        let f = PricingFile::parse(ALIASED).unwrap();
        assert_eq!(f.canonical_model("anthropic", "claude-haiku-4-5"), "claude-haiku-4-5-20251001");
        assert_eq!(f.canonical_model("anthropic", "claude-haiku-4-5-20251001"), "claude-haiku-4-5-20251001");
        assert_eq!(f.canonical_model("anthropic", "claude-never-heard-of"), "claude-never-heard-of");
        assert_eq!(f.canonical_model("nope", "claude-haiku-4-5"), "claude-haiku-4-5");
    }

    #[test]
    fn alias_conflicts_flags_self_referential_alias() {
        let f = PricingFile::parse(ALIASED).unwrap();
        assert!(f.alias_conflicts().is_empty());
        let conflicting = ALIASED.replace(
            r#""claude-sonnet-4-5" = "claude-sonnet-4-5-20250929""#,
            r#""claude-haiku-4-5-20251001" = "claude-haiku-4-5""#,
        );
        let f2 = PricingFile::parse(&conflicting).unwrap();
        assert_eq!(f2.alias_conflicts(), vec!["anthropic/claude-haiku-4-5-20251001".to_owned()]);
    }

    #[test]
    fn files_without_an_aliases_table_still_parse() {
        let plain = ALIASED.replace(
            "[providers.anthropic.aliases]\n\"claude-haiku-4-5\" = \"claude-haiku-4-5-20251001\"\n\"claude-sonnet-4-5\" = \"claude-sonnet-4-5-20250929\"\n",
            "",
        );
        let f = PricingFile::parse(&plain).unwrap();
        assert!(f.providers["anthropic"].aliases.is_empty());
        assert_eq!(f.canonical_model("anthropic", "claude-haiku-4-5"), "claude-haiku-4-5");
    }
}
