//! `GET /api/v1/subscriptions`           — list all
//! `POST /api/v1/subscriptions`          — create
//! `DELETE /api/v1/subscriptions/{id}`   — delete by id
//!
//! No PATCH / PUT in Phase 1: editing a subscription = delete + recreate.
//! Adds keep the surface narrow until the dashboard's growth justifies more.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokenscale_store::{
    delete_subscription, insert_subscription, list_subscriptions, update_subscription, Subscription,
};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct SubscriptionsResponse {
    pub subscriptions: Vec<SubscriptionDto>,
}

#[derive(Serialize)]
pub struct SubscriptionDto {
    pub id: i64,
    pub plan_name: String,
    pub monthly_usd: f64,
    pub started_at: String,
    pub ended_at: Option<String>,
}

impl From<Subscription> for SubscriptionDto {
    fn from(value: Subscription) -> Self {
        Self {
            id: value.id,
            plan_name: value.plan_name,
            monthly_usd: value.monthly_usd,
            started_at: value.started_at,
            ended_at: value.ended_at,
        }
    }
}

pub async fn list_handler(
    State(state): State<AppState>,
) -> Result<Json<SubscriptionsResponse>, ApiError> {
    let subscriptions = list_subscriptions(&state.database).await?;
    Ok(Json(SubscriptionsResponse {
        subscriptions: subscriptions
            .into_iter()
            .map(SubscriptionDto::from)
            .collect(),
    }))
}

#[derive(Deserialize)]
pub struct CreateSubscriptionRequest {
    pub plan_name: String,
    pub monthly_usd: f64,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// Validate a CreateSubscriptionRequest's fields without writing. Returns
/// the canonicalized values; reused by both create and update handlers.
fn validate_request(
    request: &CreateSubscriptionRequest,
) -> Result<(String, String, Option<String>), ApiError> {
    let plan_name = request.plan_name.trim().to_owned();
    if plan_name.is_empty() {
        return Err(ApiError::BadRequest(
            "plan_name must not be empty".to_owned(),
        ));
    }
    if request.monthly_usd < 0.0 || !request.monthly_usd.is_finite() {
        return Err(ApiError::BadRequest(
            "monthly_usd must be a non-negative finite number".to_owned(),
        ));
    }
    let started_at = parse_iso_date(&request.started_at, "started_at")?;
    let ended_at = match request.ended_at.as_deref() {
        None | Some("") => None,
        Some(value) => Some(parse_iso_date(value, "ended_at")?),
    };
    if let Some(end) = &ended_at {
        if end.as_str() < started_at.as_str() {
            return Err(ApiError::BadRequest(
                "ended_at must be on or after started_at".to_owned(),
            ));
        }
    }
    Ok((plan_name, started_at, ended_at))
}

pub async fn create_handler(
    State(state): State<AppState>,
    Json(request): Json<CreateSubscriptionRequest>,
) -> Result<(StatusCode, Json<SubscriptionDto>), ApiError> {
    let (plan_name, started_at, ended_at) = validate_request(&request)?;
    let inserted = insert_subscription(
        &state.database,
        &plan_name,
        request.monthly_usd,
        &started_at,
        ended_at.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(SubscriptionDto::from(inserted))))
}

pub async fn update_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<CreateSubscriptionRequest>,
) -> Result<Json<SubscriptionDto>, ApiError> {
    let (plan_name, started_at, ended_at) = validate_request(&request)?;
    let updated = update_subscription(
        &state.database,
        id,
        &plan_name,
        request.monthly_usd,
        &started_at,
        ended_at.as_deref(),
    )
    .await?;
    match updated {
        Some(subscription) => Ok(Json(SubscriptionDto::from(subscription))),
        None => Err(ApiError::BadRequest(format!("subscription {id} not found"))),
    }
}

pub async fn delete_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let removed = delete_subscription(&state.database, id).await?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::BadRequest(format!("subscription {id} not found")))
    }
}

/// Validate a YYYY-MM-DD field and return the canonicalized string. The
/// `field_name` parameter is plumbed only for the error message.
fn parse_iso_date(raw: &str, field_name: &str) -> Result<String, ApiError> {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(|date| date.to_string())
        .map_err(|_| {
            ApiError::BadRequest(format!("{field_name}: expected YYYY-MM-DD, got {raw:?}"))
        })
}
