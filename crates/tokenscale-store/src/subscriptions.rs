//! `subscriptions` table CRUD.
//!
//! The subscriptions table records flat-fee plans the user pays for —
//! Claude Max, API budget commitments, etc. Each row carries a plan name,
//! a monthly USD amount, and a date window (started_at, optional ended_at).
//!
//! The dashboard pro-rates these over its current chart window to compute
//! the "subscription paid" half of the net-value-of-subscription summary.
//! Date-granular precision is enough for that — sub-day billing isn't a
//! Claude product.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: i64,
    pub plan_name: String,
    pub monthly_usd: f64,
    /// `YYYY-MM-DD`.
    pub started_at: String,
    /// `YYYY-MM-DD`. `None` means "still active."
    pub ended_at: Option<String>,
}

pub async fn list_subscriptions(database: &Database) -> Result<Vec<Subscription>> {
    let rows: Vec<(i64, String, f64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, plan_name, monthly_usd, started_at, ended_at
           FROM subscriptions
          ORDER BY started_at DESC, id DESC",
    )
    .fetch_all(database.pool())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, plan_name, monthly_usd, started_at, ended_at)| Subscription {
                id,
                plan_name,
                monthly_usd,
                started_at,
                ended_at,
            },
        )
        .collect())
}

pub async fn insert_subscription(
    database: &Database,
    plan_name: &str,
    monthly_usd: f64,
    started_at: &str,
    ended_at: Option<&str>,
) -> Result<Subscription> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO subscriptions (plan_name, monthly_usd, started_at, ended_at)
              VALUES (?, ?, ?, ?)
          RETURNING id",
    )
    .bind(plan_name)
    .bind(monthly_usd)
    .bind(started_at)
    .bind(ended_at)
    .fetch_one(database.pool())
    .await?;

    Ok(Subscription {
        id: row.0,
        plan_name: plan_name.to_owned(),
        monthly_usd,
        started_at: started_at.to_owned(),
        ended_at: ended_at.map(str::to_owned),
    })
}

/// Delete by id. Returns `true` when a row was actually removed (i.e., the
/// id existed); `false` when the id was unknown.
pub async fn delete_subscription(database: &Database, id: i64) -> Result<bool> {
    let result = sqlx::query("DELETE FROM subscriptions WHERE id = ?")
        .bind(id)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Replace the fields of an existing subscription. Returns `Some(updated)`
/// when the id was found, `None` when it wasn't. Editing semantics are
/// "replace all four fields" — the API doesn't currently support partial
/// updates because subscriptions are small and the UX is "edit the form
/// and save."
pub async fn update_subscription(
    database: &Database,
    id: i64,
    plan_name: &str,
    monthly_usd: f64,
    started_at: &str,
    ended_at: Option<&str>,
) -> Result<Option<Subscription>> {
    let result = sqlx::query(
        "UPDATE subscriptions
            SET plan_name = ?, monthly_usd = ?, started_at = ?, ended_at = ?
          WHERE id = ?",
    )
    .bind(plan_name)
    .bind(monthly_usd)
    .bind(started_at)
    .bind(ended_at)
    .bind(id)
    .execute(database.pool())
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    Ok(Some(Subscription {
        id,
        plan_name: plan_name.to_owned(),
        monthly_usd,
        started_at: started_at.to_owned(),
        ended_at: ended_at.map(str::to_owned),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_database_lists_no_subscriptions() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let subscriptions = list_subscriptions(&database).await.unwrap();
        assert!(subscriptions.is_empty());
    }

    #[tokio::test]
    async fn insert_then_list_roundtrips() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let inserted = insert_subscription(&database, "Claude Max", 200.0, "2025-01-01", None)
            .await
            .unwrap();
        assert_eq!(inserted.plan_name, "Claude Max");
        assert!((inserted.monthly_usd - 200.0).abs() < f64::EPSILON);

        let subs = list_subscriptions(&database).await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].id, inserted.id);
    }

    #[tokio::test]
    async fn list_orders_by_started_at_desc() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        insert_subscription(
            &database,
            "Old plan",
            50.0,
            "2024-01-01",
            Some("2024-12-31"),
        )
        .await
        .unwrap();
        insert_subscription(&database, "Current plan", 200.0, "2025-01-01", None)
            .await
            .unwrap();
        let subs = list_subscriptions(&database).await.unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].plan_name, "Current plan"); // newer first
        assert_eq!(subs[1].plan_name, "Old plan");
    }

    #[tokio::test]
    async fn update_returns_some_on_hit_none_on_miss() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let inserted = insert_subscription(&database, "Old", 100.0, "2025-01-01", None)
            .await
            .unwrap();

        let updated = update_subscription(
            &database,
            inserted.id,
            "New",
            150.0,
            "2025-02-01",
            Some("2025-12-31"),
        )
        .await
        .unwrap()
        .expect("update should hit");
        assert_eq!(updated.plan_name, "New");
        assert!((updated.monthly_usd - 150.0).abs() < f64::EPSILON);
        assert_eq!(updated.started_at, "2025-02-01");
        assert_eq!(updated.ended_at.as_deref(), Some("2025-12-31"));

        // Confirm the change persisted.
        let listed = list_subscriptions(&database).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].plan_name, "New");

        // Unknown id → None.
        let miss = update_subscription(&database, 99_999, "Y", 1.0, "2025-01-01", None)
            .await
            .unwrap();
        assert!(miss.is_none());
    }

    #[tokio::test]
    async fn delete_returns_true_on_hit_false_on_miss() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let inserted = insert_subscription(&database, "Plan", 100.0, "2025-01-01", None)
            .await
            .unwrap();

        let deleted = delete_subscription(&database, inserted.id).await.unwrap();
        assert!(deleted);

        let deleted_again = delete_subscription(&database, inserted.id).await.unwrap();
        assert!(!deleted_again);

        let unknown = delete_subscription(&database, 99_999).await.unwrap();
        assert!(!unknown);
    }
}
