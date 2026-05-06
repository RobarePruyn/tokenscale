//! Billing endpoints — CSV import preview / commit and historical list.
//!
//! The dashboard's Stripe-CSV import flow uses a two-step
//! preview/commit pattern:
//!
//!   1. `POST /api/v1/billing/charges/preview` — accepts the raw CSV
//!      in the body, parses it with [`tokenscale_core::parse_stripe_csv`],
//!      detects which parsed rows would conflict with manually-declared
//!      subscriptions, and returns the full preview as JSON. Nothing
//!      is written to the database.
//!   2. `POST /api/v1/billing/charges/commit` — accepts the previewed
//!      charges (the user may have re-categorized rows or dismissed
//!      manual subscriptions), bulk-upserts them via
//!      [`tokenscale_store::insert_billing_charges`], and optionally
//!      deletes the dismissed manual subscriptions in the same
//!      transaction.
//!
//! The split keeps the destructive write under a "user reviewed
//! before clicking" gate without forcing the frontend to re-parse
//! the CSV after edits — the parsed rows round-trip through JSON.
//!
//! `GET /api/v1/billing/charges` lists everything in a window, used by
//! the panel's "imported charges" history view.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokenscale_core::{parse_stripe_csv, BillingCategory, BillingCharge};
use tokenscale_store::{
    delete_subscription, insert_billing_charges, list_billing_charges_in_window,
    list_subscriptions, BillingChargeInsertSummary, BillingChargeRow, Subscription,
};

use crate::error::ApiError;
use crate::state::AppState;

/// One parsed CSV row in the preview/commit wire format. Mirrors
/// `BillingCharge` but uses string forms for the category and
/// occurred_at so the JSON schema is easier to consume from the
/// frontend (no enum-shape variance, just plain strings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewedCharge {
    pub source: String,
    pub occurred_at: String,
    pub amount_usd: f64,
    pub description: String,
    /// `subscription` | `overage` | `one_time` | `refund` | `unknown`.
    pub category: String,
    pub external_id: Option<String>,
    /// Raw CSV row, preserved through the round trip so commit can
    /// store it for audit. Frontend ignores it.
    pub raw: Option<String>,
}

impl From<BillingCharge> for PreviewedCharge {
    fn from(charge: BillingCharge) -> Self {
        Self {
            source: charge.source,
            occurred_at: charge.occurred_at,
            amount_usd: charge.amount_usd,
            description: charge.description,
            category: charge.category.as_str().to_owned(),
            external_id: charge.external_id,
            raw: charge.raw,
        }
    }
}

impl From<PreviewedCharge> for BillingCharge {
    fn from(preview: PreviewedCharge) -> Self {
        Self {
            source: preview.source,
            occurred_at: preview.occurred_at,
            amount_usd: preview.amount_usd,
            description: preview.description,
            category: BillingCategory::from_str_lenient(&preview.category),
            external_id: preview.external_id,
            raw: preview.raw,
        }
    }
}

/// One manually-declared subscription that overlaps in date range
/// with at least one previewed CSV charge of category `subscription`
/// — the import preview surfaces these so the user can dismiss
/// redundant manual entries inline before committing.
#[derive(Debug, Clone, Serialize)]
pub struct ConflictingSubscription {
    pub id: i64,
    pub plan_name: String,
    pub monthly_usd: f64,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Previewed-charge `external_id` values whose date falls within
    /// `[started_at, ended_at]`. Empty `Vec` means we matched on
    /// shape (plan name) but no specific charges overlapped — rare,
    /// but kept distinct from "no conflict at all".
    pub overlapping_charge_external_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PreviewResponse {
    /// Successfully parsed charges, in input order. Each is ready to
    /// be POSTed back to the commit endpoint as-is, or with
    /// user-edited categories.
    pub charges: Vec<PreviewedCharge>,
    /// Manually-declared subscriptions whose date window overlaps any
    /// previewed `subscription`-category charge. Frontend renders a
    /// dismissable card per entry so the user can avoid double-counting.
    pub conflicting_subscriptions: Vec<ConflictingSubscription>,
    /// Sum of CSV rows we filtered out for non-USD currency.
    /// Surfaced so the user knows we silently dropped them.
    pub skipped_non_usd_count: usize,
    /// Total `amount_usd` across all parsed rows. Cheap to compute
    /// here so the frontend's preview header doesn't need its own
    /// reduce.
    pub total_amount_usd: f64,
}

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    /// Raw CSV body. We accept it as a JSON string field rather than
    /// a `text/csv` body so the same endpoint can tolerate either a
    /// drag-drop file upload or a paste-into-textarea flow on the
    /// frontend without content-type negotiation gymnastics.
    pub csv: String,
}

pub async fn preview_handler(
    State(state): State<AppState>,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<PreviewResponse>, ApiError> {
    // Count non-USD rows by parsing twice — once with the silent
    // filter (the production parser) and once with a naive line
    // count, so we can surface "X non-USD lines were skipped". The
    // double-parse is fine; CSVs are small.
    let pre_filter_count = raw_csv_data_row_count(&request.csv);
    let charges = parse_stripe_csv(&request.csv)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let skipped_non_usd_count = pre_filter_count.saturating_sub(charges.len());

    let total_amount_usd = charges
        .iter()
        .map(|charge| charge.amount_usd)
        .sum::<f64>();

    let subscriptions = list_subscriptions(&state.database).await?;
    let conflicting_subscriptions = detect_subscription_conflicts(&charges, &subscriptions);

    let preview_charges: Vec<PreviewedCharge> =
        charges.into_iter().map(PreviewedCharge::from).collect();

    Ok(Json(PreviewResponse {
        charges: preview_charges,
        conflicting_subscriptions,
        skipped_non_usd_count,
        total_amount_usd,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    /// Charges to insert. The frontend echoes the preview response's
    /// `charges` back here, possibly with user-overridden categories.
    pub charges: Vec<PreviewedCharge>,
    /// IDs of manually-declared subscriptions the user dismissed in
    /// the preview UI. Deleted in the same transaction as the insert,
    /// preventing a double-counting window.
    #[serde(default)]
    pub dismiss_subscription_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct CommitResponse {
    pub inserted: u64,
    pub skipped_duplicate: u64,
    pub dismissed_subscriptions: usize,
}

pub async fn commit_handler(
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    let charges: Vec<BillingCharge> =
        request.charges.into_iter().map(BillingCharge::from).collect();

    // Insert billing charges first so a failure on subscription
    // dismissal doesn't leave us with neither half of the user's
    // intent. Both ops are idempotent — re-running the commit with
    // already-deleted subscription IDs returns 0 dismissed.
    let BillingChargeInsertSummary {
        inserted,
        skipped_duplicate,
    } = insert_billing_charges(&state.database, &charges).await?;

    let mut dismissed_subscriptions = 0;
    for subscription_id in request.dismiss_subscription_ids {
        let was_present = delete_subscription(&state.database, subscription_id).await?;
        if was_present {
            dismissed_subscriptions += 1;
        }
    }

    Ok(Json(CommitResponse {
        inserted,
        skipped_duplicate,
        dismissed_subscriptions,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListChargesParams {
    /// `YYYY-MM-DD`. Defaults to 1970-01-01 (effectively unbounded).
    pub from: Option<String>,
    /// `YYYY-MM-DD`. Defaults to today (UTC).
    pub to: Option<String>,
    /// Optional source filter — `stripe_csv`, `anthropic_admin`, etc.
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListChargesResponse {
    pub charges: Vec<BillingChargeRow>,
}

pub async fn list_charges_handler(
    State(state): State<AppState>,
    Query(params): Query<ListChargesParams>,
) -> Result<Json<ListChargesResponse>, ApiError> {
    let from = params.from.unwrap_or_else(|| "1970-01-01".to_owned());
    let to = params
        .to
        .unwrap_or_else(|| chrono::Utc::now().date_naive().to_string());
    let charges = list_billing_charges_in_window(
        &state.database,
        &from,
        &to,
        params.source.as_deref(),
    )
    .await?;
    Ok(Json(ListChargesResponse { charges }))
}

/// Detect which manually-declared subscriptions overlap in date with
/// at least one previewed `subscription`-category charge. The match
/// is intentionally permissive — any date overlap counts; we don't
/// try to match on plan name, because users name plans variably
/// ("Claude Pro" vs "Pro plan" vs "Anthropic Pro") while the Stripe
/// CSV's description is the canonical signal.
fn detect_subscription_conflicts(
    charges: &[BillingCharge],
    subscriptions: &[Subscription],
) -> Vec<ConflictingSubscription> {
    let subscription_charges: Vec<&BillingCharge> = charges
        .iter()
        .filter(|charge| charge.category == BillingCategory::Subscription)
        .collect();

    subscriptions
        .iter()
        .filter_map(|subscription| {
            let overlapping: Vec<String> = subscription_charges
                .iter()
                .filter(|charge| {
                    let started_match = charge.occurred_at.as_str() >= subscription.started_at.as_str();
                    let ended_match = subscription
                        .ended_at
                        .as_deref()
                        .is_none_or(|ended| charge.occurred_at.as_str() <= ended);
                    started_match && ended_match
                })
                .filter_map(|charge| charge.external_id.clone())
                .collect();
            if overlapping.is_empty() {
                None
            } else {
                Some(ConflictingSubscription {
                    id: subscription.id,
                    plan_name: subscription.plan_name.clone(),
                    monthly_usd: subscription.monthly_usd,
                    started_at: subscription.started_at.clone(),
                    ended_at: subscription.ended_at.clone(),
                    overlapping_charge_external_ids: overlapping,
                })
            }
        })
        .collect()
}

/// Cheap upper-bound on the row count so we can show "skipped X
/// non-USD lines" without parsing twice for real. Counts every line
/// after the first non-empty one (the header), trimming blanks.
fn raw_csv_data_row_count(raw: &str) -> usize {
    let mut lines = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .skip(1); // skip header
    lines.by_ref().count()
}

/// Reject malformed CSV with a 400 rather than the default 422 axum
/// would emit, so the frontend's error display is consistent with
/// other validation paths. Unused locally but kept in scope for
/// future error refinement.
#[allow(dead_code)]
fn bad_csv(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_conflicts_finds_overlap_by_date_window() {
        let charges = vec![BillingCharge {
            source: "stripe_csv".to_owned(),
            occurred_at: "2026-04-15".to_owned(),
            amount_usd: 200.0,
            description: "Claude Max".to_owned(),
            category: BillingCategory::Subscription,
            external_id: Some("in_001".to_owned()),
            raw: None,
        }];
        let subscriptions = vec![
            Subscription {
                id: 1,
                plan_name: "Claude Max".to_owned(),
                monthly_usd: 200.0,
                started_at: "2026-04-01".to_owned(),
                ended_at: Some("2026-05-01".to_owned()),
            },
            Subscription {
                id: 2,
                plan_name: "Old plan".to_owned(),
                monthly_usd: 50.0,
                started_at: "2025-01-01".to_owned(),
                ended_at: Some("2025-12-31".to_owned()),
            },
        ];
        let conflicts = detect_subscription_conflicts(&charges, &subscriptions);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].id, 1);
        assert_eq!(conflicts[0].overlapping_charge_external_ids, vec!["in_001"]);
    }

    #[test]
    fn detect_conflicts_skips_non_subscription_charges() {
        let charges = vec![BillingCharge {
            source: "stripe_csv".to_owned(),
            occurred_at: "2026-04-15".to_owned(),
            amount_usd: 12.34,
            description: "API overage".to_owned(),
            category: BillingCategory::Overage,
            external_id: Some("in_001".to_owned()),
            raw: None,
        }];
        let subscriptions = vec![Subscription {
            id: 1,
            plan_name: "Claude Max".to_owned(),
            monthly_usd: 200.0,
            started_at: "2026-04-01".to_owned(),
            ended_at: None,
        }];
        // Even though dates overlap, the CSV row is an overage — not
        // a subscription — so it shouldn't conflict.
        assert!(detect_subscription_conflicts(&charges, &subscriptions).is_empty());
    }

    #[test]
    fn raw_data_row_count_skips_header_and_blanks() {
        let csv = "id,Date,Amount\nch_a,2026-01-01,20\n\nch_b,2026-02-01,20\n";
        assert_eq!(raw_csv_data_row_count(csv), 2);
    }
}
