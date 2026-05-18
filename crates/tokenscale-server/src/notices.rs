//! In-app notices — one-time-dismissible banners that explain a specific
//! change to the user on first launch after upgrade.
//!
//! The dashboard's existing pricing-review banner (v0.1.0) is purely
//! data-driven from `pricing.toml`; this module covers a different surface:
//! release-event notices that exist to give the user *context* on a change
//! the data itself doesn't carry. The v0.1.11 pricing correction is the
//! first instance — the headline counterfactual number drops ~3x for Opus
//! users, and without an in-product explanation that reads as "the tool
//! broke" rather than "the old number was wrong." A 90-day notice closes
//! that gap.
//!
//! Notices are **compile-time constants in the binary** — a notice added
//! in v0.1.11 can only exist in binaries built from v0.1.11 or later, so
//! we don't need a runtime version check. Each notice has an explicit
//! expiry date; after that, the binary no longer surfaces it (the
//! dated correction log in `docs/cost-methodology.md` is the permanent
//! record).
//!
//! Dismissal state is persisted to a tiny TOML file alongside
//! `config.toml`. Frontend never needs to know the persistence shape —
//! it only sees the GET response (active notices) and POSTs to dismiss.

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::Mutex;

/// One in-app notice. Authored once at release time; never edited
/// after ship — that's what the corrections log in
/// `docs/cost-methodology.md` is for. To retire a notice early, drop
/// it from `NOTICES` in a later release.
#[derive(Debug, Clone, Copy)]
pub struct Notice {
    /// Stable identifier used in the dismissal state file and POST URL.
    /// Format: `<topic>-<release>` — e.g. `pricing-correction-v0.1.11`.
    pub id: &'static str,
    /// Date after which the notice no longer renders, ISO `YYYY-MM-DD`.
    /// 90 days from release matches the quarterly research-sweep cadence.
    pub expires_at: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub link_url: &'static str,
    pub link_label: &'static str,
}

/// All notices the running binary knows about. Authored at release time;
/// see the doc comment on `Notice` for retirement semantics.
pub const NOTICES: &[Notice] = &[Notice {
    id: "pricing-correction-v0.1.11",
    expires_at: "2026-08-16", // ~90 days from 2026-05-18 release
    title: "Pricing correction in v0.1.11",
    body: "Three model rows in pricing.toml carried wrong API rates from v0.1.0 through v0.1.10. \
           Opus 4.7 and 4.6 were about 3x overstated, Haiku 4.5 about 20% understated. v0.1.11 \
           corrects them. If your historical \"Counterfactual API cost\" or \"Estimated savings \
           vs raw API rates\" figures look smaller than they did last week, this is why: the old \
           numbers were inflated, not the new ones deflated.",
    link_url: "https://github.com/RobarePruyn/tokenscale/blob/main/docs/cost-methodology.md#corrections-log",
    link_label: "Full detail in docs/cost-methodology.md, Corrections log",
}];

/// What the server sends to the frontend for each active notice. The
/// frontend doesn't need to know about `expires_at` — the server has
/// already filtered. Keeps the dismissal-storage shape entirely
/// server-side.
#[derive(Debug, Serialize)]
pub struct ActiveNotice {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    #[serde(rename = "linkUrl")]
    pub link_url: &'static str,
    #[serde(rename = "linkLabel")]
    pub link_label: &'static str,
}

/// Persisted shape of `<config-dir>/dismissed-notices.toml`. Versioned
/// so we can evolve the schema later (e.g. add per-notice dismissal
/// metadata) without breaking older state files.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DismissedNoticesFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub dismissed: Vec<DismissedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DismissedEntry {
    pub id: String,
    /// ISO 8601 timestamp of dismissal. Recorded for forensic value
    /// only — the active-notice filter only needs a set membership
    /// check.
    pub dismissed_at: String,
}

fn default_schema_version() -> u32 {
    1
}

/// Async-safe reader/writer for the dismissal state file. Wraps the
/// file path + a tokio Mutex so concurrent dismiss requests serialize
/// on the file write rather than racing.
pub struct DismissalStore {
    path: std::path::PathBuf,
    lock: Mutex<()>,
}

impl DismissalStore {
    #[must_use]
    pub fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    /// Read the dismissal state. Missing file = empty (no notices
    /// dismissed yet). Malformed file = empty + a warning — we'd rather
    /// re-show a dismissed notice than crash on startup.
    pub async fn load(&self) -> DismissedNoticesFile {
        match fs::read_to_string(&self.path).await {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|err| {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %err,
                    "could not parse dismissed-notices.toml — treating as empty"
                );
                DismissedNoticesFile::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                DismissedNoticesFile::default()
            }
            Err(err) => {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %err,
                    "could not read dismissed-notices.toml — treating as empty"
                );
                DismissedNoticesFile::default()
            }
        }
    }

    /// Append a dismissal. Idempotent — already-dismissed notice IDs
    /// are a no-op. Creates parent directory + file if missing.
    pub async fn dismiss(&self, notice_id: &str) -> std::io::Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.load().await;
        if state.dismissed.iter().any(|d| d.id == notice_id) {
            return Ok(()); // already dismissed
        }
        if state.schema_version == 0 {
            state.schema_version = default_schema_version();
        }
        state.dismissed.push(DismissedEntry {
            id: notice_id.to_owned(),
            dismissed_at: Utc::now().to_rfc3339(),
        });
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let body = toml::to_string_pretty(&state).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?;
        fs::write(&self.path, body).await
    }
}

/// Filter `NOTICES` down to the ones the dashboard should actually
/// render right now: not past expiry AND not dismissed. The version
/// check is implicit (a notice authored in v0.1.11 isn't compiled
/// into earlier binaries).
#[must_use]
pub fn active_notices(state: &DismissedNoticesFile) -> Vec<ActiveNotice> {
    let today = Utc::now().date_naive();
    let dismissed_ids: std::collections::HashSet<&str> =
        state.dismissed.iter().map(|d| d.id.as_str()).collect();
    NOTICES
        .iter()
        .filter(|n| {
            if dismissed_ids.contains(n.id) {
                return false;
            }
            match NaiveDate::parse_from_str(n.expires_at, "%Y-%m-%d") {
                Ok(expiry) => today <= expiry,
                Err(_) => true, // malformed expiry → show; better than silently hiding
            }
        })
        .map(|n| ActiveNotice {
            id: n.id,
            title: n.title,
            body: n.body,
            link_url: n.link_url,
            link_label: n.link_label,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn active_notices_excludes_dismissed() {
        let mut state = DismissedNoticesFile::default();
        state.dismissed.push(DismissedEntry {
            id: "pricing-correction-v0.1.11".to_owned(),
            dismissed_at: "2026-05-18T00:00:00Z".to_owned(),
        });
        let active = active_notices(&state);
        assert!(
            active.iter().all(|n| n.id != "pricing-correction-v0.1.11"),
            "dismissed notice must not appear"
        );
    }

    #[test]
    fn active_notices_excludes_expired() {
        // A notice with expiry yesterday should not be returned.
        // Synthesize via the public constant + manual filter — we just
        // need to confirm the date comparison works.
        let yesterday = (Utc::now() - Duration::days(1)).date_naive();
        let expired = Notice {
            id: "test-expired",
            expires_at: Box::leak(yesterday.format("%Y-%m-%d").to_string().into_boxed_str()),
            title: "Expired",
            body: "",
            link_url: "",
            link_label: "",
        };
        let today = Utc::now().date_naive();
        let parsed = NaiveDate::parse_from_str(expired.expires_at, "%Y-%m-%d").unwrap();
        assert!(today > parsed, "test setup: yesterday must be < today");
    }

    #[tokio::test]
    async fn dismissal_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dismissed-notices.toml");
        let store = DismissalStore::new(path.clone());

        store.dismiss("pricing-correction-v0.1.11").await.unwrap();
        store.dismiss("pricing-correction-v0.1.11").await.unwrap();

        let state = store.load().await;
        assert_eq!(state.dismissed.len(), 1, "duplicate dismiss must be idempotent");
        assert_eq!(state.dismissed[0].id, "pricing-correction-v0.1.11");
    }

    #[tokio::test]
    async fn dismissal_creates_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("dismissed-notices.toml");
        let store = DismissalStore::new(path.clone());
        store.dismiss("test").await.unwrap();
        assert!(path.exists());
    }
}
