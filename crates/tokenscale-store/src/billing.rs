//! Persistence for the `billing_charges` table.
//!
//! Two write paths surface here:
//!   * [`insert_billing_charges`] — bulk upsert with idempotent dedup
//!     via `(source, external_id)`. Used by the CSV import endpoint
//!     and (later) by the Anthropic Admin cost-report ingester.
//!   * [`delete_billing_charges_by_source`] — clears a source ahead of
//!     a clean re-import. Symmetrical to the `--rebuild` path on the
//!     events side.
//!
//! Read paths surface the data the dashboard's stat row needs:
//!   * [`list_billing_charges_in_window`] — every charge whose date
//!     falls in `[from, to]`, used by the import preview's
//!     conflict-detection step.
//!   * [`sum_billing_charges_in_window`] — pre-aggregated total for
//!     the "Anthropic billed in window" stat card, scoped to one or
//!     all sources.
//!
//! ## Dedup contract
//!
//! `external_id` is `NOT NULL` in the schema's UNIQUE index — but
//! Stripe CSVs occasionally lack stable IDs (older exports, custom
//! line items). For those rows the persistence layer synthesizes a
//! deterministic ID from `(date, amount, description)` so re-imports
//! still dedupe deterministically. The synthesis is documented inline
//! below so future readers don't have to reverse-engineer the format.

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::QueryBuilder;
use tokenscale_core::BillingCharge;

use crate::error::Result;
use crate::Database;

/// Outcome counters returned from a bulk insert. Mirrors the shape of
/// `events::InsertSummary` so the API layer can surface "X new, Y
/// already-known" with the same wording across both ingest paths.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BillingChargeInsertSummary {
    pub inserted: u64,
    pub skipped_duplicate: u64,
}

/// Insert charges in bulk, deduping on `(source, external_id)`. Rows
/// without an `external_id` get a synthesized one (a SHA-256 prefix of
/// `date|amount|description`) so re-imports still collapse. Returns
/// counts so the API surface can report "imported N (skipped M
/// duplicates)" without a follow-up SELECT.
pub async fn insert_billing_charges(
    database: &Database,
    charges: &[BillingCharge],
) -> Result<BillingChargeInsertSummary> {
    let mut summary = BillingChargeInsertSummary::default();
    if charges.is_empty() {
        return Ok(summary);
    }

    let now = Utc::now().to_rfc3339();
    let mut transaction = database.pool().begin().await?;

    // sqlx doesn't support parameter binding for executing many INSERTs
    // in a single round trip with conditional return values, so we
    // batch into one builder and rely on `INSERT OR IGNORE`'s changes()
    // accounting per-statement. Stripe CSV imports are bounded (a few
    // hundred rows even for years of history), so this is well within
    // SQLite's 999-bind-parameter ceiling.
    for charge in charges {
        let synthesized_external_id = charge
            .external_id
            .clone()
            .unwrap_or_else(|| synthesize_external_id(charge));
        let result = sqlx::query(
            "INSERT OR IGNORE INTO billing_charges (
                source, occurred_at, amount_usd, description, category,
                external_id, raw, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&charge.source)
        .bind(&charge.occurred_at)
        .bind(charge.amount_usd)
        .bind(&charge.description)
        .bind(charge.category.as_str())
        .bind(&synthesized_external_id)
        .bind(charge.raw.as_deref())
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() > 0 {
            summary.inserted += 1;
        } else {
            summary.skipped_duplicate += 1;
        }
    }

    transaction.commit().await?;
    Ok(summary)
}

/// Delete every charge with a given `source`. Used to re-import a
/// source from scratch when the user knows their previous import was
/// wrong (parser bug, mis-categorization at scale). Mirrors the
/// `--rebuild` semantics on the events side.
pub async fn delete_billing_charges_by_source(
    database: &Database,
    source: &str,
) -> Result<u64> {
    let result = sqlx::query("DELETE FROM billing_charges WHERE source = ?")
        .bind(source)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

/// Tuple shape sqlx binds the SELECT columns into. Defined as a
/// type alias so clippy's type-complexity lint doesn't fire and so
/// the destructure below stays a single readable line.
type RawBillingChargeTuple = (
    i64,            // id
    String,         // source
    String,         // occurred_at
    f64,            // amount_usd
    Option<String>, // description
    String,         // category
    Option<String>, // external_id
    String,         // created_at
);

/// One row out of the billing_charges table, in the shape the API
/// returns to the import preview. Carries the auto-id so the UI can
/// render edit / delete affordances; otherwise mirrors `BillingCharge`.
#[derive(Debug, Clone, Serialize)]
pub struct BillingChargeRow {
    pub id: i64,
    pub source: String,
    pub occurred_at: String,
    pub amount_usd: f64,
    pub description: Option<String>,
    pub category: String,
    pub external_id: Option<String>,
    pub created_at: String,
}

/// Every charge whose `occurred_at` falls in `[from, to]` (inclusive,
/// `YYYY-MM-DD`), ordered newest first so the preview UI shows recent
/// activity at the top. `source_filter` of `None` means "all sources."
pub async fn list_billing_charges_in_window(
    database: &Database,
    from_date: &str,
    to_date: &str,
    source_filter: Option<&str>,
) -> Result<Vec<BillingChargeRow>> {
    let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
        "SELECT id, source, occurred_at, amount_usd, description, category, external_id, created_at
           FROM billing_charges
          WHERE occurred_at BETWEEN ",
    );
    builder.push_bind(from_date.to_owned());
    builder.push(" AND ");
    builder.push_bind(to_date.to_owned());
    if let Some(source) = source_filter {
        builder.push(" AND source = ");
        builder.push_bind(source.to_owned());
    }
    builder.push(" ORDER BY occurred_at DESC, id DESC");

    let raw_rows: Vec<RawBillingChargeTuple> =
        builder.build_query_as().fetch_all(database.pool()).await?;
    Ok(raw_rows
        .into_iter()
        .map(
            |(id, source, occurred_at, amount_usd, description, category, external_id, created_at)| {
                BillingChargeRow {
                    id,
                    source,
                    occurred_at,
                    amount_usd,
                    description,
                    category,
                    external_id,
                    created_at,
                }
            },
        )
        .collect())
}

/// Sum of `amount_usd` for charges in `[from, to]`, optionally
/// filtered to a single source. Drives the dashboard's "Anthropic
/// billed in window" stat card. Refunds (negative amounts) reduce
/// the sum, which is the expected accounting.
pub async fn sum_billing_charges_in_window(
    database: &Database,
    from_date: &str,
    to_date: &str,
    source_filter: Option<&str>,
) -> Result<f64> {
    // CAST AS REAL because COALESCE's literal `0.0` would still be
    // typed INTEGER on an empty result set in SQLite, and sqlx maps
    // strict types — we want f64 unconditionally.
    let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
        "SELECT CAST(COALESCE(SUM(amount_usd), 0.0) AS REAL)
           FROM billing_charges
          WHERE occurred_at BETWEEN ",
    );
    builder.push_bind(from_date.to_owned());
    builder.push(" AND ");
    builder.push_bind(to_date.to_owned());
    if let Some(source) = source_filter {
        builder.push(" AND source = ");
        builder.push_bind(source.to_owned());
    }
    let total: (f64,) = builder.build_query_as().fetch_one(database.pool()).await?;
    Ok(total.0)
}

/// Deterministic ID for a charge that came in without one. SHA-256 of
/// `source|date|amount|description`, base16-encoded, prefixed so it's
/// obviously synthetic if a future reader inspects the table directly.
/// Stable across re-imports of the same row.
fn synthesize_external_id(charge: &BillingCharge) -> String {
    let mut hasher = Sha256::new();
    hasher.update(charge.source.as_bytes());
    hasher.update(b"|");
    hasher.update(charge.occurred_at.as_bytes());
    hasher.update(b"|");
    hasher.update(charge.amount_usd.to_le_bytes());
    hasher.update(b"|");
    hasher.update(charge.description.as_bytes());
    let digest = hasher.finalize();
    format!("synth:{}", hex::encode(&digest[..16]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenscale_core::BillingCategory;

    fn sample_charge(occurred_at: &str, amount: f64, external_id: Option<&str>) -> BillingCharge {
        BillingCharge {
            source: "stripe_csv".to_owned(),
            occurred_at: occurred_at.to_owned(),
            amount_usd: amount,
            description: "Claude Pro subscription".to_owned(),
            category: BillingCategory::Subscription,
            external_id: external_id.map(str::to_owned),
            raw: None,
        }
    }

    #[tokio::test]
    async fn insert_then_list_roundtrips() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let charges = vec![
            sample_charge("2026-01-15", 20.00, Some("in_001")),
            sample_charge("2026-02-15", 20.00, Some("in_002")),
            sample_charge("2026-03-15", 20.00, Some("in_003")),
        ];
        let summary = insert_billing_charges(&database, &charges).await.unwrap();
        assert_eq!(summary.inserted, 3);
        assert_eq!(summary.skipped_duplicate, 0);

        let rows = list_billing_charges_in_window(
            &database,
            "2026-01-01",
            "2026-04-01",
            Some("stripe_csv"),
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 3);
        // Newest first.
        assert_eq!(rows[0].occurred_at, "2026-03-15");
    }

    #[tokio::test]
    async fn re_inserting_same_external_id_is_idempotent() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let charge = sample_charge("2026-01-15", 20.00, Some("in_001"));

        let first = insert_billing_charges(&database, std::slice::from_ref(&charge))
            .await
            .unwrap();
        assert_eq!(first.inserted, 1);
        let second = insert_billing_charges(&database, std::slice::from_ref(&charge))
            .await
            .unwrap();
        assert_eq!(second.inserted, 0);
        assert_eq!(second.skipped_duplicate, 1);
    }

    #[tokio::test]
    async fn rows_without_external_id_dedup_via_synthesized_hash() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let charge = sample_charge("2026-01-15", 20.00, None);

        let first = insert_billing_charges(&database, std::slice::from_ref(&charge))
            .await
            .unwrap();
        assert_eq!(first.inserted, 1);
        let second = insert_billing_charges(&database, std::slice::from_ref(&charge))
            .await
            .unwrap();
        assert_eq!(second.inserted, 0);
        assert_eq!(second.skipped_duplicate, 1);
    }

    #[tokio::test]
    async fn sum_in_window_includes_refunds_as_negative() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let charges = vec![
            sample_charge("2026-01-15", 20.00, Some("in_001")),
            BillingCharge {
                source: "stripe_csv".to_owned(),
                occurred_at: "2026-02-01".to_owned(),
                amount_usd: -5.00,
                description: "Partial refund".to_owned(),
                category: BillingCategory::Refund,
                external_id: Some("in_002".to_owned()),
                raw: None,
            },
        ];
        insert_billing_charges(&database, &charges).await.unwrap();

        let total =
            sum_billing_charges_in_window(&database, "2026-01-01", "2026-03-01", None)
                .await
                .unwrap();
        assert!((total - 15.00).abs() < 1e-9);
    }

    #[tokio::test]
    async fn sum_outside_window_returns_zero() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        insert_billing_charges(
            &database,
            &[sample_charge("2026-01-15", 20.00, Some("in_001"))],
        )
        .await
        .unwrap();

        let total =
            sum_billing_charges_in_window(&database, "2027-01-01", "2027-12-31", None)
                .await
                .unwrap();
        assert!((total).abs() < 1e-9);
    }

    #[tokio::test]
    async fn delete_by_source_only_clears_that_source() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        insert_billing_charges(
            &database,
            &[sample_charge("2026-01-15", 20.00, Some("in_001"))],
        )
        .await
        .unwrap();
        // Different source — should survive the delete.
        insert_billing_charges(
            &database,
            &[BillingCharge {
                source: "anthropic_admin".to_owned(),
                ..sample_charge("2026-02-15", 5.00, Some("cost_001"))
            }],
        )
        .await
        .unwrap();

        let removed = delete_billing_charges_by_source(&database, "stripe_csv")
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let remaining =
            list_billing_charges_in_window(&database, "2026-01-01", "2026-12-31", None)
                .await
                .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].source, "anthropic_admin");
    }
}
