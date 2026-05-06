//! Billing-charge domain types and Stripe Customer Portal CSV parsing.
//!
//! This module is intentionally storage-agnostic: it exposes a pure
//! [`parse_stripe_csv`] function that turns raw CSV text into a `Vec<
//! BillingCharge>`, and it ships with a categorization heuristic that
//! tags each row as subscription / overage / one-time / refund / unknown
//! based on the line item's description and amount sign.
//!
//! The persistence layer (`tokenscale-store`) and HTTP surface
//! (`tokenscale-server`) consume the result without knowing anything
//! about CSV formats — so when we add the Anthropic Admin
//! `cost_report` ingester next, it produces the same `BillingCharge`
//! shape and reuses the same persistence path.
//!
//! ## Stripe CSV format flexibility
//!
//! The Stripe Customer Portal exports a CSV that *roughly* follows a
//! standard but varies by export type (Invoices vs. Subscriptions vs.
//! Charges) and by which fields the user enabled. Rather than locking
//! to one schema, we map a small set of synonymous header names to the
//! fields we need:
//!
//! | Field         | Header candidates (case-insensitive)                          |
//! |---------------|---------------------------------------------------------------|
//! | external_id   | `id`, `Invoice ID`, `Charge ID`, `Number`, `Invoice number`   |
//! | occurred_at   | `Date`, `Created`, `Created (UTC)`, `Period start`, `Date paid` |
//! | amount_usd    | `Amount`, `Amount Paid`, `Total`, `Amount Due`                  |
//! | currency      | `Currency`                                                       |
//! | description   | `Description`, `Memo`, `Line items`                              |
//!
//! Missing-but-recoverable fields fall back to defaults
//! (currency = "USD", description = ""); a missing **date** or
//! **amount** is a hard error since we can't synthesize either.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One billed line item, normalized across import sources. Mirrors the
/// `billing_charges` table shape minus the auto-id / `created_at`
/// columns the persistence layer fills in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingCharge {
    /// Opaque identifier set by the ingest layer — `"stripe_csv"` for
    /// CSV imports, `"anthropic_admin"` for Phase 2's cost-report ingest.
    pub source: String,
    /// ISO-8601 date (`YYYY-MM-DD`) of the charge.
    pub occurred_at: String,
    /// Amount in USD. Negative values represent refunds / credits.
    pub amount_usd: f64,
    /// Free-form description from the source, used for display and as
    /// the primary signal for the categorization heuristic.
    pub description: String,
    /// Tag computed at parse time; the import preview UI lets the user
    /// override it before committing.
    pub category: BillingCategory,
    /// Stable provider-side ID (Stripe charge id, invoice number, etc.)
    /// — combined with `source` for upsert dedup. `None` means the row
    /// had no usable ID; the persistence layer synthesizes one from
    /// (date, amount, description) so re-imports still dedup.
    pub external_id: Option<String>,
    /// Verbatim CSV row content (or API response chunk) JSON-encoded
    /// for audit. Optional because non-CSV sources may not preserve a
    /// raw form.
    pub raw: Option<String>,
}

/// Coarse classification used to route the charge in the dashboard's
/// stat-row math. The heuristic is deliberately conservative —
/// `Unknown` is preferred over a confidently wrong tag, and the import
/// preview surfaces all five so the user can re-tag before committing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingCategory {
    /// Recurring same-amount line. Plan fees (Pro / Max / Team).
    Subscription,
    /// Usage-based one-off. Token overage on a metered subscription,
    /// API usage charge on a Workspace.
    Overage,
    /// Single non-recurring charge. Setup fee, one-shot purchase.
    OneTime,
    /// Negative amount or "refund" / "credit" in the description.
    Refund,
    /// Heuristic abstained — user picks a category in the preview UI.
    Unknown,
}

impl BillingCategory {
    /// Stable string form for SQL persistence and JSON wire payloads.
    /// Mirrors the `#[serde(rename_all = "snake_case")]` form so the
    /// DB and the API agree on the same canonical strings.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::Overage => "overage",
            Self::OneTime => "one_time",
            Self::Refund => "refund",
            Self::Unknown => "unknown",
        }
    }

    /// Inverse of [`Self::as_str`] — accepts any value the application
    /// itself ever produces, plus an `Unknown` fallback for forward
    /// compatibility (a future row written with a category we don't
    /// recognize won't crash; it surfaces as Unknown).
    #[must_use]
    pub fn from_str_lenient(raw: &str) -> Self {
        match raw {
            "subscription" => Self::Subscription,
            "overage" => Self::Overage,
            "one_time" => Self::OneTime,
            "refund" => Self::Refund,
            _ => Self::Unknown,
        }
    }
}

/// Errors surfaced from the CSV parsing path. Each variant carries
/// enough context for the API layer to render a useful message in the
/// import preview ("row 3: missing amount column") rather than a
/// generic "parse failed."
#[derive(Debug, Error)]
pub enum BillingParseError {
    /// CSV reader couldn't tokenize the input — usually malformed
    /// quoting or an inconsistent column count.
    #[error("malformed CSV: {0}")]
    MalformedCsv(#[from] csv::Error),

    /// We couldn't find a date column under any recognized header
    /// name. Listed candidates so the user can rename a column or
    /// re-export.
    #[error(
        "CSV is missing a date column (looked for any of: Date, Created, Created (UTC), Period start, Date paid)"
    )]
    MissingDateColumn,

    /// Same as above for the amount column.
    #[error(
        "CSV is missing an amount column (looked for any of: Amount, Amount Paid, Total, Amount Due)"
    )]
    MissingAmountColumn,

    /// A specific row's date couldn't be parsed as `YYYY-MM-DD` (we
    /// also try a couple of other common Stripe formats — this fires
    /// when none match).
    #[error("row {row_number}: unparseable date {raw_value:?}")]
    UnparseableDate { row_number: usize, raw_value: String },

    /// Same for the amount field.
    #[error("row {row_number}: unparseable amount {raw_value:?}")]
    UnparseableAmount { row_number: usize, raw_value: String },
}

/// Parse a Stripe Customer Portal CSV export into a `Vec<BillingCharge>`.
///
/// The function is intentionally non-strict about which columns the
/// CSV carries beyond the bare minimum (date + amount) — extra columns
/// are ignored, missing-but-recoverable columns fall back to defaults.
/// `source` is set to `"stripe_csv"` on every returned row.
pub fn parse_stripe_csv(raw: &str) -> Result<Vec<BillingCharge>, BillingParseError> {
    // Strip a UTF-8 BOM if present — Stripe's CSV exports occasionally
    // include one and the csv crate doesn't auto-strip.
    let cleaned = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(cleaned.as_bytes());

    let column_map = ColumnMap::from_headers(reader.headers()?)?;

    let mut charges = Vec::new();
    for (index, record_result) in reader.records().enumerate() {
        // Header is row 0 in user-speak; data rows start at 1.
        let row_number = index + 2;
        let record = record_result?;

        let raw_date = column_map.get(&record, ColumnRole::Date).unwrap_or("");
        let raw_amount = column_map.get(&record, ColumnRole::Amount).unwrap_or("");
        let raw_currency = column_map.get(&record, ColumnRole::Currency).unwrap_or("");
        let raw_description = column_map
            .get(&record, ColumnRole::Description)
            .unwrap_or("")
            .trim();
        let raw_external_id = column_map.get(&record, ColumnRole::ExternalId);

        let occurred_at = parse_date(raw_date).ok_or_else(|| BillingParseError::UnparseableDate {
            row_number,
            raw_value: raw_date.to_owned(),
        })?;
        let amount_usd =
            parse_amount(raw_amount).ok_or_else(|| BillingParseError::UnparseableAmount {
                row_number,
                raw_value: raw_amount.to_owned(),
            })?;

        // Skip non-USD lines silently — multi-currency support is a
        // future concern. Tracked in the import summary on the API
        // boundary, not here.
        if !raw_currency.is_empty() && !raw_currency.eq_ignore_ascii_case("USD") {
            continue;
        }

        let category = categorize(raw_description, amount_usd);

        charges.push(BillingCharge {
            source: "stripe_csv".to_owned(),
            occurred_at,
            amount_usd,
            description: raw_description.to_owned(),
            category,
            external_id: raw_external_id.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned),
            raw: Some(record.iter().collect::<Vec<_>>().join("\t")),
        });
    }
    Ok(charges)
}

/// Heuristic categorization. The rules in priority order:
///
/// 1. Negative amount → Refund (Stripe issues refunds as negative
///    line items; this catches them before the description heuristic
///    might mis-tag, e.g., "Refund for Claude Pro Subscription").
/// 2. Description contains `refund` / `credit` / `chargeback` → Refund.
/// 3. Description contains `overage` / `usage` / `metered` / `api` →
///    Overage.
/// 4. Description contains `subscription` / `monthly` / `recurring` /
///    `pro` / `max` / `team` / `plan` → Subscription.
/// 5. Description contains `setup` / `onboarding` / `one-time` →
///    OneTime.
/// 6. Otherwise Unknown — user retags in preview UI.
fn categorize(description: &str, amount_usd: f64) -> BillingCategory {
    if amount_usd < 0.0 {
        return BillingCategory::Refund;
    }
    let lowered = description.to_lowercase();
    if matches_any(&lowered, &["refund", "credit memo", "chargeback"]) {
        return BillingCategory::Refund;
    }
    if matches_any(&lowered, &["overage", "usage", "metered", "api usage"]) {
        return BillingCategory::Overage;
    }
    if matches_any(
        &lowered,
        &[
            "subscription",
            "monthly",
            "recurring",
            "pro plan",
            "max plan",
            "team plan",
            "claude pro",
            "claude max",
            "claude team",
        ],
    ) {
        return BillingCategory::Subscription;
    }
    if matches_any(&lowered, &["setup", "onboarding", "one-time", "one time"]) {
        return BillingCategory::OneTime;
    }
    BillingCategory::Unknown
}

fn matches_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Try `YYYY-MM-DD` first (Stripe's canonical export format), then a
/// couple of common alternatives we've seen in the wild. Returns the
/// canonical `YYYY-MM-DD` form on success regardless of the input
/// format, so the persistence layer doesn't need to think about
/// format.
fn parse_date(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Try the full trimmed string against every known format first —
    // covers "Apr 15 2026" / "Apr 15, 2026" / "April 15, 2026" where
    // the whole string IS the date.
    let candidates = [
        "%Y-%m-%d",
        "%m/%d/%Y",
        "%d/%m/%Y",
        "%b %-d, %Y",
        "%B %-d, %Y",
        "%b %-d %Y",
        "%B %-d %Y",
    ];
    for format in &candidates {
        if let Ok(parsed) = NaiveDate::parse_from_str(trimmed, format) {
            return Some(parsed.format("%Y-%m-%d").to_string());
        }
    }
    // Fallback: the value is likely a datetime like
    // "2026-04-15 12:34:56" or "2026-04-15T12:34:56Z" — strip the
    // time portion and retry the date-only formats.
    let date_only = trimmed
        .split_once([' ', 'T'])
        .map_or(trimmed, |(date, _)| date);
    for format in &candidates {
        if let Ok(parsed) = NaiveDate::parse_from_str(date_only, format) {
            return Some(parsed.format("%Y-%m-%d").to_string());
        }
    }
    None
}

/// Strip currency symbols and thousands separators, then parse as f64.
/// Accepts forms like `"20.00"`, `"$20.00"`, `"$1,234.56"`, `"-5.00"`,
/// `"(5.00)"` (accounting parenthesis-as-negative).
fn parse_amount(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negate, rest) = if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        (true, inner)
    } else {
        (false, trimmed)
    };
    let sanitized: String = rest
        .chars()
        .filter(|c| !matches!(c, '$' | '€' | '£' | '¥' | ',' | ' '))
        .collect();
    let value: f64 = sanitized.parse().ok()?;
    Some(if negate { -value } else { value })
}

/// Roles we map header strings to. Each role is a logical field; the
/// `ColumnMap` resolves it to a concrete column index per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnRole {
    Date,
    Amount,
    Currency,
    Description,
    ExternalId,
}

// Each field is the column position for the same-named role; the
// `_index` suffix is intentional and uniform — clippy's struct-field-
// names lint flags the parallelism but it's load-bearing here.
#[allow(clippy::struct_field_names)]
struct ColumnMap {
    date_index: usize,
    amount_index: usize,
    currency_index: Option<usize>,
    description_index: Option<usize>,
    external_id_index: Option<usize>,
}

impl ColumnMap {
    fn from_headers(headers: &csv::StringRecord) -> Result<Self, BillingParseError> {
        let lowered: Vec<String> =
            headers.iter().map(|raw| raw.trim().to_lowercase()).collect();

        let date_index = first_match(
            &lowered,
            &["date", "created", "created (utc)", "period start", "date paid", "invoice date"],
        )
        .ok_or(BillingParseError::MissingDateColumn)?;

        let amount_index = first_match(
            &lowered,
            &["amount", "amount paid", "total", "amount due", "subtotal"],
        )
        .ok_or(BillingParseError::MissingAmountColumn)?;

        let currency_index = first_match(&lowered, &["currency"]);
        let description_index = first_match(&lowered, &["description", "memo", "line items"]);
        let external_id_index =
            first_match(&lowered, &["id", "invoice id", "charge id", "number", "invoice number"]);

        Ok(Self {
            date_index,
            amount_index,
            currency_index,
            description_index,
            external_id_index,
        })
    }

    fn get<'r>(&self, record: &'r csv::StringRecord, role: ColumnRole) -> Option<&'r str> {
        let index = match role {
            ColumnRole::Date => Some(self.date_index),
            ColumnRole::Amount => Some(self.amount_index),
            ColumnRole::Currency => self.currency_index,
            ColumnRole::Description => self.description_index,
            ColumnRole::ExternalId => self.external_id_index,
        };
        index.and_then(|index| record.get(index))
    }
}

/// Find the first lowered-header that exactly matches one of the
/// candidate names, returning its column index. Exact match instead
/// of substring match — substring matches are too easy to fool (e.g.,
/// a column named "Date Description" would match both "date" and
/// "description").
fn first_match(lowered_headers: &[String], candidates: &[&str]) -> Option<usize> {
    for candidate in candidates {
        if let Some(index) = lowered_headers.iter().position(|header| header == candidate) {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Realistic Stripe Customer Portal export with mixed line types
    /// — recurring subscription, overage, refund, one-time setup,
    /// non-USD line (skipped).
    const SAMPLE_CSV: &str = "\
id,Date,Description,Amount,Currency,Status
in_001,2026-01-15,Claude Pro Subscription - Monthly,20.00,USD,paid
in_002,2026-02-15,Claude Pro Subscription - Monthly,20.00,USD,paid
in_003,2026-03-15,Claude Max Subscription - Monthly,100.00,USD,paid
in_004,2026-03-22,API usage overage - March,12.34,USD,paid
in_005,2026-04-01,Refund for Claude Pro,-20.00,USD,paid
in_006,2026-02-01,Onboarding setup fee,50.00,USD,paid
in_007,2026-03-15,Claude Equipe Mensuel,15.00,EUR,paid
";

    #[test]
    fn parses_a_realistic_stripe_export_into_categorized_rows() {
        let charges = parse_stripe_csv(SAMPLE_CSV).expect("parses cleanly");
        // Six USD lines (the EUR row is dropped by the currency filter).
        assert_eq!(charges.len(), 6);

        let by_id: std::collections::HashMap<&str, &BillingCharge> = charges
            .iter()
            .map(|charge| (charge.external_id.as_deref().unwrap(), charge))
            .collect();

        assert_eq!(by_id["in_001"].category, BillingCategory::Subscription);
        assert_eq!(by_id["in_004"].category, BillingCategory::Overage);
        assert_eq!(by_id["in_005"].category, BillingCategory::Refund);
        assert!((by_id["in_005"].amount_usd - -20.0).abs() < f64::EPSILON);
        assert_eq!(by_id["in_006"].category, BillingCategory::OneTime);
        assert!(!by_id.contains_key("in_007"), "EUR row should be filtered out");
    }

    #[test]
    fn missing_date_column_yields_clear_error() {
        let csv = "\
id,Description,Amount
in_001,Subscription,20.00
";
        let error = parse_stripe_csv(csv).unwrap_err();
        assert!(matches!(error, BillingParseError::MissingDateColumn));
    }

    #[test]
    fn missing_amount_column_yields_clear_error() {
        let csv = "\
id,Date,Description
in_001,2026-01-15,Subscription
";
        let error = parse_stripe_csv(csv).unwrap_err();
        assert!(matches!(error, BillingParseError::MissingAmountColumn));
    }

    #[test]
    fn handles_dollar_signs_thousands_separators_and_parens_negatives() {
        let csv = "\
id,Date,Description,Amount,Currency
ch_a,2026-01-15,Big plan,\"$1,234.56\",USD
ch_b,2026-01-15,Refund,(50.00),USD
ch_c,2026-01-15,Cents,0.99,USD
";
        let charges = parse_stripe_csv(csv).expect("parses cleanly");
        assert!((charges[0].amount_usd - 1234.56).abs() < 1e-9);
        assert!((charges[1].amount_usd - -50.0).abs() < 1e-9);
        assert_eq!(charges[1].category, BillingCategory::Refund);
        assert!((charges[2].amount_usd - 0.99).abs() < 1e-9);
    }

    #[test]
    fn normalizes_alternative_date_formats_to_yyyy_mm_dd() {
        let csv = "\
id,Date,Description,Amount,Currency
ch_a,01/15/2026,Sub,20.00,USD
ch_b,Apr 15 2026,Sub,20.00,USD
ch_c,2026-01-15 14:30:00,Sub,20.00,USD
";
        let charges = parse_stripe_csv(csv).expect("parses cleanly");
        assert_eq!(charges[0].occurred_at, "2026-01-15");
        assert_eq!(charges[1].occurred_at, "2026-04-15");
        assert_eq!(charges[2].occurred_at, "2026-01-15");
    }

    #[test]
    fn unparseable_date_yields_per_row_error_with_row_number() {
        let csv = "\
id,Date,Description,Amount,Currency
ch_a,not-a-date,Sub,20.00,USD
";
        let error = parse_stripe_csv(csv).unwrap_err();
        match error {
            BillingParseError::UnparseableDate { row_number, raw_value } => {
                assert_eq!(row_number, 2);
                assert_eq!(raw_value, "not-a-date");
            }
            _ => panic!("expected UnparseableDate, got {error:?}"),
        }
    }

    #[test]
    fn empty_csv_with_just_headers_returns_empty_vec() {
        let csv = "id,Date,Description,Amount,Currency\n";
        let charges = parse_stripe_csv(csv).expect("parses cleanly");
        assert!(charges.is_empty());
    }

    #[test]
    fn category_str_round_trip() {
        for category in [
            BillingCategory::Subscription,
            BillingCategory::Overage,
            BillingCategory::OneTime,
            BillingCategory::Refund,
            BillingCategory::Unknown,
        ] {
            let round_tripped = BillingCategory::from_str_lenient(category.as_str());
            assert_eq!(round_tripped, category);
        }
    }

    #[test]
    fn unrecognized_category_string_decodes_as_unknown() {
        assert_eq!(
            BillingCategory::from_str_lenient("future_category_we_havent_invented"),
            BillingCategory::Unknown
        );
    }
}
