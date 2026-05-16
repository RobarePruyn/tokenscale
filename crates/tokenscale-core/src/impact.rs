//! Environmental-impact computation — the math centerpiece.
//!
//! # Methodology
//!
//! Per Cowork research (see `docs/research-log.md`, 2026-04-28), this
//! crate adopts Google's "comprehensive methodology" from Elsworth et al.
//! 2025, [arXiv:2508.15734]. The key consequence: **`compute_impact` does
//! NOT publish "active GPU only" numbers.** Per-token Wh values in the
//! factor file are already comprehensive (active accelerator + host
//! CPU/RAM + idle capacity). What this module adds is the PUE-weighted
//! facility overhead and the carbon / water multiplications.
//!
//! Elsworth et al. show comprehensive figures are roughly 2.4× higher
//! than active-accelerator-only ones. We do not expose a separate
//! active-only path — Cowork explicitly flagged that as the
//! "under-counting" pattern the methodology was designed to displace.
//!
//! # Math
//!
//! ```text
//! energy_wh   = Σ_token_types(tokens × wh_per_mtok / 1_000_000)
//! facility_wh = energy_wh × pue
//! co2e_g      = (facility_wh / 1000) × co2e_kg_per_kwh × 1000
//!             = facility_wh × co2e_kg_per_kwh                   // simplified
//! water_l     = (facility_wh / 1000) × water_l_per_kwh
//! ```
//!
//! # Null handling
//!
//! Per the kickoff prompt:
//!
//! - A null `wh_per_mtok_*` for a token type contributes **zero** to
//!   `energy_wh` for that type. The dashboard later renders an
//!   "incomplete model factor" footnote when this happens.
//! - A null `pue` falls back to `defaults.fallback_pue` (e.g., AWS 1.15
//!   when the factor file says so). `used_fallback_pue = true` lands in
//!   the provenance struct.
//! - A null `water_l_per_kwh` falls back to `defaults.fallback_wue_l_per_kwh`
//!   if present, otherwise yields `water_l = None` — distinct from "0.0,"
//!   which the dashboard renders as "unavailable" rather than zero.
//! - A null `co2e_kg_per_kwh` yields `co2e_g = None` — same semantics.
//!
//! # Energy view headline
//!
//! `EnvironmentalImpact::facility_wh` is the user-facing "energy" number.
//! `energy_wh` (model-compute only, before PUE) is exposed for the
//! methodology page that breaks down compute vs. facility overhead.

use serde::Serialize;

use crate::factors::{FactorDefaults, GridFactors, ModelFactors};

/// Five token-type counts. Lets `compute_impact` accept either a single
/// event's tokens or a bucket-aggregated sum without an `Event` import
/// dependency — keeps the math portable to a future framework crate.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventTokenCounts {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

/// Environmental impact attributable to one event (or one aggregated
/// bucket). All values are in SI units; the API surface re-scales for
/// presentation (Wh → kWh / MWh; mL → L; gCO₂e → kgCO₂e).
#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentalImpact {
    /// Model-compute energy only — active accelerator + host CPU/RAM +
    /// idle capacity, BEFORE PUE-weighted facility overhead. The
    /// methodology page exposes this; the dashboard's "energy" view uses
    /// `facility_wh`.
    pub energy_wh: f64,

    /// `energy_wh × pue` — the user-facing comprehensive-methodology
    /// "energy" number. **This is what the dashboard's energy view
    /// surfaces.**
    pub facility_wh: f64,

    /// Carbon equivalent in grams. `None` when the grid row's
    /// `co2e_kg_per_kwh` is null — the dashboard renders this as
    /// "unavailable," distinct from a computed near-zero.
    pub co2e_g: Option<f64>,

    /// **Direct** (on-site, DC cooling) water in liters — scope-1 water
    /// in Ren et al.'s framing. `None` when neither the grid row's
    /// `water_l_per_kwh` nor the file's `defaults.fallback_wue_l_per_kwh`
    /// is set.
    pub water_l: Option<f64>,

    /// **Indirect** (off-site, power-plant cooling) water in liters —
    /// scope-2 water in Ren et al. 2024 framing. `None` when the grid
    /// row doesn't publish `indirect_water_l_per_kwh` (no fallback
    /// available because indirect water is fundamentally per-region;
    /// applying a global default would be misleading).
    pub indirect_water_l: Option<f64>,

    /// Uncertainty on `facility_wh` in percent. Currently inherits the
    /// model factor's `uncertainty_range_pct` only — PUE uncertainty is
    /// not separately tracked, so it folds into this band.
    pub energy_uncertainty_pct: i32,

    /// Uncertainty on `co2e_g`, in percent. Quadrature of model + grid
    /// CO₂e uncertainties when both are present; degrades to whichever
    /// one is non-zero. See [`combine_uncertainty_pct`].
    pub co2e_uncertainty_pct: i32,

    /// Uncertainty on direct `water_l`, in percent. Same combination
    /// rule as `co2e_uncertainty_pct` but with the grid's direct water
    /// uncertainty.
    pub water_uncertainty_pct: i32,

    /// Uncertainty on `indirect_water_l`, in percent. Quadrature of
    /// model uncertainty + grid `indirect_water_uncertainty_range_pct`.
    pub indirect_water_uncertainty_pct: i32,

    pub factors_used: FactorsProvenance,
}

/// Combine two independent uncertainty bands (in percent) via quadrature:
/// `√(a² + b²)`. The standard error-propagation rule for a product of two
/// values with independent fractional uncertainties — applies to
/// `facility_wh × co2e_kg_per_kwh` (model and grid are independent) and
/// `facility_wh × water_l_per_kwh` for the same reason.
///
/// Returns an `i32` rounded to nearest. When both inputs are zero the
/// result is zero (no published bands → no combined band).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn combine_uncertainty_pct(model_pct: i32, grid_pct: i32) -> i32 {
    let m = f64::from(model_pct);
    let g = f64::from(grid_pct);
    (m.mul_add(m, g * g)).sqrt().round() as i32
}

/// Audit trail attached to every `EnvironmentalImpact`. Phase 2 doesn't
/// render this in the dashboard yet, but the Phase 3 methodology page
/// hovers depend on it being populated from day one.
#[derive(Debug, Clone, Serialize)]
pub struct FactorsProvenance {
    pub provider: String,
    pub model: String,

    /// `valid_from` of the model factor row used.
    pub model_factor_valid_from: String,

    /// `docs/sources.md#G.1` etc.
    pub model_factor_source_doc: String,

    /// `"primary"` / `"secondary"` / `"superseded"` per `docs/sources.md`.
    pub model_factor_confidence: Option<String>,

    /// Honest uncertainty band on the model row, %. The energy-side
    /// surface for `± X%` rendering; combines with grid bands via
    /// quadrature for CO₂e and water (see [`combine_uncertainty_pct`]).
    pub model_factor_uncertainty_pct: i32,

    pub region: String,
    pub grid_factor_valid_from: String,

    pub grid_factor_source_url: String,

    pub grid_factor_egrid_subregion: Option<String>,

    /// Honest uncertainty band on the grid row's `co2e_kg_per_kwh`, %.
    /// Zero when the row didn't publish one; in v0.1.4 this happens only
    /// for legacy / placeholder grid rows.
    pub grid_co2e_uncertainty_pct: i32,

    /// Honest uncertainty band on the grid row's `water_l_per_kwh`, %.
    /// Same zero-means-not-published convention.
    pub grid_water_uncertainty_pct: i32,

    /// Honest uncertainty band on the grid row's
    /// `indirect_water_l_per_kwh` (off-site / power-plant cooling), %.
    /// Zero when the grid row doesn't publish indirect water at all
    /// (legacy factor-file rows pre-Sweep #2).
    pub grid_indirect_water_uncertainty_pct: i32,

    pub schema_version: i64,

    /// `true` when the grid row's `pue` was null and we fell back to
    /// `defaults.fallback_pue`.
    pub used_fallback_pue: bool,

    /// `true` when the grid row's `water_l_per_kwh` was null and we
    /// fell back to `defaults.fallback_wue_l_per_kwh`.
    pub used_fallback_wue: bool,
}

/// Required inputs for `compute_impact`. Bundled into a struct so the
/// signature stays stable as the math grows (e.g., when v0.2 introduces
/// indirect water modeling per Ren et al.).
pub struct ImpactInputs<'a> {
    pub tokens: EventTokenCounts,
    pub provider: &'a str,
    pub region: &'a str,
    pub schema_version: i64,
    pub model_factors: &'a ModelFactors,
    /// `model` value from the event row. Carried into provenance.
    pub model_id: &'a str,
    pub grid_factors: &'a GridFactors,
    pub defaults: &'a FactorDefaults,
}

/// Pure function — no I/O, no async. Maps `ImpactInputs` to an
/// `EnvironmentalImpact`. Suitable for per-event use (called from a
/// handler) or aggregated-bucket use (called once with summed token
/// counts).
//
// Bound by reference rather than by value: `ImpactInputs` carries
// references, but the struct itself is small and `&` makes it natural
// to call repeatedly per-event without `Copy`-derive ceremony.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn compute_impact(inputs: &ImpactInputs<'_>) -> EnvironmentalImpact {
    let energy_wh = compute_energy_wh(&inputs.tokens, inputs.model_factors);

    let (pue_factor, pue_was_fallback) = match inputs.grid_factors.pue {
        Some(grid_pue) => (grid_pue, false),
        None => (
            inputs.defaults.fallback_pue.unwrap_or(1.0),
            inputs.defaults.fallback_pue.is_some(),
        ),
    };
    let facility_wh = energy_wh * pue_factor;

    let co2e_g = inputs
        .grid_factors
        .co2e_kg_per_kwh
        .map(|kg_per_kwh| facility_wh * kg_per_kwh);

    let (water_litres, water_was_fallback) = match inputs.grid_factors.water_l_per_kwh {
        Some(grid_wue) => (Some((facility_wh / 1000.0) * grid_wue), false),
        None => match inputs.defaults.fallback_wue_l_per_kwh {
            Some(fallback_wue) => (Some((facility_wh / 1000.0) * fallback_wue), true),
            None => (None, false),
        },
    };

    // Indirect water (off-site / power-plant cooling). Distinct from
    // direct: applies the regional EWIF (electricity-water-intensity
    // factor) to facility energy. No fallback path — indirect water is
    // fundamentally region-specific; a global default would be
    // misleading. When the grid row doesn't publish it, we return None.
    let indirect_water_litres = inputs
        .grid_factors
        .indirect_water_l_per_kwh
        .map(|grid_ewif| (facility_wh / 1000.0) * grid_ewif);

    let model_uncertainty_pct = inputs.model_factors.uncertainty_range_pct.unwrap_or(0);
    let grid_co2e_uncertainty_pct = inputs
        .grid_factors
        .co2e_uncertainty_range_pct
        .unwrap_or(0);
    let grid_water_uncertainty_pct = inputs
        .grid_factors
        .water_uncertainty_range_pct
        .unwrap_or(0);
    let grid_indirect_water_uncertainty_pct = inputs
        .grid_factors
        .indirect_water_uncertainty_range_pct
        .unwrap_or(0);

    EnvironmentalImpact {
        energy_wh,
        facility_wh,
        co2e_g,
        water_l: water_litres,
        indirect_water_l: indirect_water_litres,
        // facility_wh inherits model uncertainty only — PUE has no
        // separately-tracked uncertainty band yet, so it folds in here.
        energy_uncertainty_pct: model_uncertainty_pct,
        co2e_uncertainty_pct: combine_uncertainty_pct(
            model_uncertainty_pct,
            grid_co2e_uncertainty_pct,
        ),
        water_uncertainty_pct: combine_uncertainty_pct(
            model_uncertainty_pct,
            grid_water_uncertainty_pct,
        ),
        indirect_water_uncertainty_pct: combine_uncertainty_pct(
            model_uncertainty_pct,
            grid_indirect_water_uncertainty_pct,
        ),
        factors_used: FactorsProvenance {
            provider: inputs.provider.to_owned(),
            model: inputs.model_id.to_owned(),
            model_factor_valid_from: inputs.model_factors.valid_from.clone().unwrap_or_default(),
            model_factor_source_doc: inputs.model_factors.source_doc.clone().unwrap_or_default(),
            model_factor_confidence: inputs.model_factors.confidence.clone(),
            model_factor_uncertainty_pct: model_uncertainty_pct,
            region: inputs.region.to_owned(),
            grid_factor_valid_from: inputs.grid_factors.valid_from.clone().unwrap_or_default(),
            grid_factor_source_url: inputs
                .grid_factors
                .source_url_co2e
                .clone()
                .unwrap_or_default(),
            grid_factor_egrid_subregion: inputs.grid_factors.egrid_subregion.clone(),
            grid_co2e_uncertainty_pct,
            grid_water_uncertainty_pct,
            grid_indirect_water_uncertainty_pct,
            schema_version: inputs.schema_version,
            used_fallback_pue: pue_was_fallback,
            used_fallback_wue: water_was_fallback,
        },
    }
}

/// Sum the token-type contributions to model-compute energy. A null
/// `wh_per_mtok_*` for a token type contributes zero rather than
/// erroring out — partial models still produce a meaningful number for
/// the token types that ARE populated, and the dashboard separately
/// surfaces the "factor missing" status.
#[allow(clippy::cast_precision_loss)]
fn compute_energy_wh(tokens: &EventTokenCounts, model: &ModelFactors) -> f64 {
    let mut energy_wh = 0.0;
    if let Some(wh_per_mtok) = model.wh_per_mtok_input {
        energy_wh += tokens.input as f64 * wh_per_mtok / 1_000_000.0;
    }
    if let Some(wh_per_mtok) = model.wh_per_mtok_output {
        energy_wh += tokens.output as f64 * wh_per_mtok / 1_000_000.0;
    }
    if let Some(wh_per_mtok) = model.wh_per_mtok_cache_read {
        energy_wh += tokens.cache_read as f64 * wh_per_mtok / 1_000_000.0;
    }
    if let Some(wh_per_mtok) = model.wh_per_mtok_cache_write_5m {
        energy_wh += tokens.cache_write_5m as f64 * wh_per_mtok / 1_000_000.0;
    }
    if let Some(wh_per_mtok) = model.wh_per_mtok_cache_write_1h {
        energy_wh += tokens.cache_write_1h as f64 * wh_per_mtok / 1_000_000.0;
    }
    energy_wh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factors::{FactorDefaults, GridFactors, ModelFactors};

    /// Sonnet 4.6's v0.1 production values (Wh/MTok), copied from
    /// `environmental-factors.toml`. Used as the golden-value anchor.
    fn sonnet_4_6_factors() -> ModelFactors {
        ModelFactors {
            display_name: "Claude Sonnet 4.6".to_owned(),
            released_at: None,
            valid_from: Some("2026-04-28".to_owned()),
            source_doc: Some("docs/sources.md#G.1".to_owned()),
            wh_per_mtok_input: Some(195.0),
            wh_per_mtok_output: Some(965.0),
            wh_per_mtok_cache_read: Some(20.0),
            wh_per_mtok_cache_write_5m: Some(244.0),
            wh_per_mtok_cache_write_1h: Some(390.0),
            confidence: Some("secondary".to_owned()),
            uncertainty_range_pct: Some(35),
            notes: None,
        }
    }

    /// us-east-1's v0.1 production values.
    fn us_east_1_grid() -> GridFactors {
        GridFactors {
            display_name: "AWS US East (N. Virginia)".to_owned(),
            valid_from: Some("2026-04-28".to_owned()),
            co2e_kg_per_kwh: Some(0.270),
            co2e_uncertainty_range_pct: None,
            water_l_per_kwh: Some(0.15),
            water_uncertainty_range_pct: None,
            indirect_water_l_per_kwh: None,
            indirect_water_uncertainty_range_pct: None,
            pue: Some(1.15),
            egrid_subregion: Some("SRVC".to_owned()),
            egrid_subregion_full_name: Some("SERC Virginia/Carolina".to_owned()),
            source_url_co2e: Some("https://www.epa.gov/egrid".to_owned()),
            source_url_water: None,
            source_url_indirect_water: None,
            source_url_pue: None,
            source_accessed_at: Some("2026-04-28".to_owned()),
            notes: None,
        }
    }

    fn defaults_with_fallbacks() -> FactorDefaults {
        FactorDefaults {
            fallback_pue: Some(1.15),
            fallback_wue_l_per_kwh: Some(0.15),
            methodology: Some("google-comprehensive-aug-2025".to_owned()),
        }
    }

    #[test]
    fn sonnet_4_6_us_east_1_golden_value() {
        // Hand-derived golden value:
        //   1,000,000 input × 195 Wh/MTok / 1e6 = 195 Wh
        //     100,000 output × 965 Wh/MTok / 1e6 = 96.5 Wh
        //     energy_wh = 195 + 96.5 = 291.5
        //     facility_wh = 291.5 × 1.15 = 335.225
        //     co2e_g = 335.225 × 0.270 = 90.51075
        //     water_l = 335.225 / 1000 × 0.15 = 0.05028375
        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1_000_000,
                output: 100_000,
                cache_read: 0,
                cache_write_5m: 0,
                cache_write_1h: 0,
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &us_east_1_grid(),
            defaults: &defaults_with_fallbacks(),
        });

        assert!(
            (impact.energy_wh - 291.5).abs() < 1e-6,
            "energy_wh = {}",
            impact.energy_wh
        );
        assert!(
            (impact.facility_wh - 335.225).abs() < 1e-6,
            "facility_wh = {}",
            impact.facility_wh
        );
        assert!(
            (impact.co2e_g.unwrap() - 90.51075).abs() < 1e-6,
            "co2e_g = {:?}",
            impact.co2e_g
        );
        assert!(
            (impact.water_l.unwrap() - 0.050_283_75).abs() < 1e-9,
            "water_l = {:?}",
            impact.water_l
        );
        assert_eq!(impact.factors_used.model_factor_uncertainty_pct, 35);
        assert_eq!(impact.factors_used.region, "us-east-1");
        assert!(!impact.factors_used.used_fallback_pue);
        assert!(!impact.factors_used.used_fallback_wue);
    }

    #[test]
    fn null_water_in_grid_yields_water_l_none_without_fallback() {
        let mut grid = us_east_1_grid();
        grid.water_l_per_kwh = None;
        let mut defaults = defaults_with_fallbacks();
        defaults.fallback_wue_l_per_kwh = None;

        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1_000,
                ..Default::default()
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &grid,
            defaults: &defaults,
        });

        assert!(
            impact.water_l.is_none(),
            "water_l should be None when no fallback"
        );
        assert!(!impact.factors_used.used_fallback_wue);
    }

    #[test]
    fn null_water_in_grid_uses_default_fallback_when_present() {
        let mut grid = us_east_1_grid();
        grid.water_l_per_kwh = None;

        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1_000_000,
                ..Default::default()
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &grid,
            defaults: &defaults_with_fallbacks(),
        });

        // 1M input × 195 Wh/MTok = 195 Wh; ×1.15 PUE = 224.25 facility_wh
        // /1000 × 0.15 fallback WUE = 0.0336375 L
        assert!(
            (impact.water_l.unwrap() - 0.033_637_5).abs() < 1e-9,
            "water_l = {:?}",
            impact.water_l
        );
        assert!(impact.factors_used.used_fallback_wue);
    }

    #[test]
    fn null_co2e_in_grid_yields_co2e_g_none() {
        let mut grid = us_east_1_grid();
        grid.co2e_kg_per_kwh = None;

        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1_000,
                ..Default::default()
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &grid,
            defaults: &defaults_with_fallbacks(),
        });

        assert!(
            impact.co2e_g.is_none(),
            "co2e_g should be None when grid co2e is null"
        );
    }

    #[test]
    fn null_pue_falls_back_to_defaults_fallback_pue() {
        let mut grid = us_east_1_grid();
        grid.pue = None;

        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1_000_000,
                ..Default::default()
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &grid,
            defaults: &defaults_with_fallbacks(),
        });

        // energy_wh = 195; with fallback PUE 1.15 → facility_wh = 224.25
        assert!((impact.facility_wh - 224.25).abs() < 1e-6);
        assert!(impact.factors_used.used_fallback_pue);
    }

    #[test]
    fn null_wh_per_mtok_for_a_token_type_contributes_zero() {
        let mut model = sonnet_4_6_factors();
        // Drop the input rate; output stays.
        model.wh_per_mtok_input = None;

        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1_000_000, // doesn't contribute since input rate is null
                output: 100_000,
                cache_read: 0,
                cache_write_5m: 0,
                cache_write_1h: 0,
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &model,
            model_id: "claude-sonnet-4-6",
            grid_factors: &us_east_1_grid(),
            defaults: &defaults_with_fallbacks(),
        });

        // Only output contributes: 100K × 965 / 1e6 = 96.5 Wh
        assert!((impact.energy_wh - 96.5).abs() < 1e-9);
    }

    #[test]
    fn combined_uncertainty_quadrature_when_both_model_and_grid_bands_present() {
        // Model = 35%, grid CO₂e = 15%, grid water = 50% — Sweep #1's
        // SRVC profile combined with Sonnet 4.6's "secondary" band.
        let mut grid = us_east_1_grid();
        grid.co2e_uncertainty_range_pct = Some(15);
        grid.water_uncertainty_range_pct = Some(50);

        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1_000_000,
                ..Default::default()
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &grid,
            defaults: &defaults_with_fallbacks(),
        });

        // facility_wh inherits model only (no separate PUE band yet).
        assert_eq!(impact.energy_uncertainty_pct, 35);
        // CO₂e: √(35² + 15²) = √1450 ≈ 38.08 → 38
        assert_eq!(impact.co2e_uncertainty_pct, 38);
        // Water: √(35² + 50²) = √3725 ≈ 61.03 → 61
        assert_eq!(impact.water_uncertainty_pct, 61);

        // Provenance carries the raw grid bands for the dashboard's
        // sources panel to display alongside the combined badge.
        assert_eq!(impact.factors_used.grid_co2e_uncertainty_pct, 15);
        assert_eq!(impact.factors_used.grid_water_uncertainty_pct, 50);
    }

    #[test]
    fn combined_uncertainty_degrades_to_model_when_grid_bands_absent() {
        // Grid file with NULL uncertainty fields (the v0.1 legacy state
        // before Sweep #1 — present in tests via DB-backed lookups that
        // hit grid_factors rows from older factor-file syncs).
        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1,
                ..Default::default()
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &us_east_1_grid(), // co2e/water uncertainty = None
            defaults: &defaults_with_fallbacks(),
        });

        assert_eq!(impact.energy_uncertainty_pct, 35);
        assert_eq!(impact.co2e_uncertainty_pct, 35);
        assert_eq!(impact.water_uncertainty_pct, 35);
    }

    #[test]
    fn combine_uncertainty_pct_math() {
        assert_eq!(combine_uncertainty_pct(0, 0), 0);
        assert_eq!(combine_uncertainty_pct(35, 0), 35);
        assert_eq!(combine_uncertainty_pct(0, 50), 50);
        // 3-4-5 triangle: √(3² + 4²) = 5
        assert_eq!(combine_uncertainty_pct(3, 4), 5);
        // √(35² + 15²) = √1450 ≈ 38.08 → 38 (rounds down by .08)
        assert_eq!(combine_uncertainty_pct(35, 15), 38);
        // √(35² + 50²) = √3725 ≈ 61.03 → 61
        assert_eq!(combine_uncertainty_pct(35, 50), 61);
    }

    #[test]
    fn provenance_carries_audit_trail_fields() {
        let impact = compute_impact(&ImpactInputs {
            tokens: EventTokenCounts {
                input: 1,
                ..Default::default()
            },
            provider: "anthropic",
            region: "us-east-1",
            schema_version: 1,
            model_factors: &sonnet_4_6_factors(),
            model_id: "claude-sonnet-4-6",
            grid_factors: &us_east_1_grid(),
            defaults: &defaults_with_fallbacks(),
        });

        let p = &impact.factors_used;
        assert_eq!(p.provider, "anthropic");
        assert_eq!(p.model, "claude-sonnet-4-6");
        assert_eq!(p.model_factor_valid_from, "2026-04-28");
        assert_eq!(p.model_factor_source_doc, "docs/sources.md#G.1");
        assert_eq!(p.model_factor_confidence.as_deref(), Some("secondary"));
        assert_eq!(p.model_factor_uncertainty_pct, 35);
        assert_eq!(p.region, "us-east-1");
        assert_eq!(p.grid_factor_valid_from, "2026-04-28");
        assert_eq!(p.grid_factor_egrid_subregion.as_deref(), Some("SRVC"));
        assert_eq!(p.schema_version, 1);
    }
}
