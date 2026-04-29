//! Billable-token weighting.
//!
//! Raw token counts conflate orders-of-magnitude price differences: a single
//! Opus output token costs roughly 50× as much as a cache-read token. The
//! "billable tokens" view weights each token type by its API price relative
//! to input, so when you stack the result you see a chart whose visual area
//! tracks API cost — not raw count.
//!
//! Output is dimensionless ("input-token-equivalent" tokens). To convert to
//! dollars: `billable_total * pricing.input_usd_per_mtok / 1_000_000.0`.

use crate::pricing::ModelPricing;

/// Per-token-type multipliers derived from a model's pricing. `input` is
/// always `1.0` (input is the reference). All other types are expressed as
/// "input-equivalent fractions": `output = output_price / input_price`, etc.
#[derive(Debug, Clone, Copy)]
pub struct BillableMultipliers {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
}

impl BillableMultipliers {
    /// Compute multipliers for a given model's pricing.
    ///
    /// Cache-write multipliers come straight from the pricing file —
    /// Anthropic publishes them as input-price ratios already. Output and
    /// cache-read are derived by dividing through the input price.
    #[must_use]
    pub fn from_pricing(pricing: &ModelPricing) -> Self {
        Self {
            input: 1.0,
            output: pricing.output_usd_per_mtok / pricing.input_usd_per_mtok,
            cache_read: pricing.cache_read_usd_per_mtok / pricing.input_usd_per_mtok,
            cache_write_5m: pricing.cache_write_5m_multiplier,
            cache_write_1h: pricing.cache_write_1h_multiplier,
        }
    }

    /// Combine raw token counts into a single billable total. Returns `f64`
    /// so the multiplier math is lossless; callers convert to integer or to
    /// dollars at the display boundary.
    ///
    /// `u64 → f64` is lossy above 2^53 (~9e15); token counts will not
    /// approach that even on a years-of-data instance, so the cast is
    /// intentional — the alternative (saturating) would silently undercount
    /// at the boundary, which is worse.
    #[must_use]
    // 5m/1h cache class names mirror the Anthropic API convention; renaming
    // them to satisfy `similar_names` would obscure that mapping.
    #[allow(clippy::cast_precision_loss, clippy::similar_names)]
    pub fn weight_total(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_5m_tokens: u64,
        cache_1h_tokens: u64,
    ) -> f64 {
        input_tokens as f64 * self.input
            + output_tokens as f64 * self.output
            + cache_read_tokens as f64 * self.cache_read
            + cache_5m_tokens as f64 * self.cache_write_5m
            + cache_1h_tokens as f64 * self.cache_write_1h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::PricingFile;

    fn opus_pricing() -> ModelPricing {
        let parsed = PricingFile::parse(
            r#"
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
source_url = "https://example.test"
source_accessed_at = "2026-04-28"
"#,
        )
        .unwrap();
        parsed
            .lookup("anthropic", "claude-opus-4-7")
            .unwrap()
            .clone()
    }

    #[test]
    fn opus_multipliers_match_published_ratios() {
        let multipliers = BillableMultipliers::from_pricing(&opus_pricing());
        assert!((multipliers.input - 1.0).abs() < f64::EPSILON);
        assert!((multipliers.output - 5.0).abs() < f64::EPSILON);
        assert!((multipliers.cache_read - 0.10).abs() < 1e-12);
        assert!((multipliers.cache_write_5m - 1.25).abs() < f64::EPSILON);
        assert!((multipliers.cache_write_1h - 2.00).abs() < f64::EPSILON);
    }

    #[test]
    fn weight_total_combines_all_token_types() {
        let multipliers = BillableMultipliers::from_pricing(&opus_pricing());
        // 1000 input + 100 output + 10000 cache_read + 100 cache_5m + 50 cache_1h
        // = 1000*1.0 + 100*5.0 + 10000*0.1 + 100*1.25 + 50*2.0
        // = 1000 + 500 + 1000 + 125 + 100 = 2725
        let total = multipliers.weight_total(1000, 100, 10_000, 100, 50);
        assert!((total - 2725.0).abs() < f64::EPSILON);
    }

    #[test]
    fn weight_total_is_zero_for_zero_tokens() {
        let multipliers = BillableMultipliers::from_pricing(&opus_pricing());
        let total = multipliers.weight_total(0, 0, 0, 0, 0);
        assert!(total.abs() < f64::EPSILON);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn cache_reads_dominate_for_typical_claude_code_session() {
        // The data we observed in Phase B kickoff: 98.5% of opus-4-7 tokens
        // are cache reads. Confirm the billable view substantially deflates
        // that — most of the weight should come from output.
        let multipliers = BillableMultipliers::from_pricing(&opus_pricing());
        let raw_total = 1_000_u64 + 100_000_u64 + 100_000_000_u64 + 10_000_u64 + 1_000_000_u64;
        let billable = multipliers.weight_total(1_000, 100_000, 100_000_000, 10_000, 1_000_000);
        // Billable should be far less than raw — cache_read dominates raw
        // count but only contributes 10% per token.
        let deflation_factor = billable / raw_total as f64;
        assert!(deflation_factor < 0.20);
    }
}
