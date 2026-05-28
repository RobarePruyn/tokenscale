//! Session-commit type for Phase 2 (v0.1.18) Tier 1 commit attribution.
//!
//! One `SessionCommit` per real `git commit` invocation captured from
//! a CC Bash tool_use. See `docs/roadmap-2-commit-attribution.md` D1
//! for the schema design and rationale; see § 1.6 for the empirical
//! sha-nullability rate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Records which regex captured the SHA. The `None` bucket covers
/// both real commit failures and parser misses; downstream code
/// distinguishes via the `output_head_truncated_by_command` flag and
/// the SHA-resolution check (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySource {
    /// Captured via `[<branch>( \(root-commit\))? <sha>]` regex.
    /// Empirical 93.7% of real commit invocations in the maintainer
    /// corpus.
    Primary,
    /// Captured via push-refspec fallback regex
    /// `<old>..<new>  <local> -> <remote>` when `git push` is chained
    /// in the same Bash command. Empirical 3.5% additional recovery.
    PushRefspec,
    /// No SHA recoverable from the tool_result. Real failures
    /// (gitignore reject, nothing-to-commit) and maintainer-pipe-cut-
    /// without-chained-push both land here.
    None,
}

impl RecoverySource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::PushRefspec => "push_refspec",
            Self::None => "none",
        }
    }

    /// Parse the DB-side TEXT representation back into the enum.
    /// Used by query primitives. Named `from_db_str` rather than
    /// `from_str` to avoid colliding with the standard
    /// `std::str::FromStr` trait method, which has a different
    /// signature (returns Result, not Self).
    #[must_use]
    pub fn from_db_str(value: &str) -> Self {
        match value {
            "primary" => Self::Primary,
            "push_refspec" => Self::PushRefspec,
            _ => Self::None,
        }
    }
}

/// One captured commit, derived from a Bash tool_use that invoked
/// `git commit` (per the shlex token-walk filter).
///
/// This is the pure domain type. The `tokenscale-store::commit_data`
/// module inserts these into the `session_commits` SQLite table; the
/// `tokenscale-server::routes::commits` handler reads them back and
/// adds the query-time SHA resolution check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCommit {
    pub source: String,
    pub tool_use_id: String,
    pub session_id: String,
    /// The captured commit SHA. `None` for the ~2.1% of invocations
    /// where neither the primary regex nor the push-refspec fallback
    /// captured anything (real failures plus head-piped-without-push).
    pub sha: Option<String>,
    /// Verbatim `cd "<path>"` target if present; `None` when the
    /// command has no cd prefix (14% of commits).
    pub cd_target_raw: Option<String>,
    /// Resolved git toplevel at insert time, via
    /// `tokenscale_core::cwd::resolve_to_git_toplevel`. Forward-only:
    /// stored value is what was true at scan time, never updated.
    pub project_resolved: String,
    pub recovery_source: RecoverySource,
    /// True when the Bash command contains `| tail` anywhere,
    /// signalling the operator deliberately truncated git commit's
    /// output head. See D5 Part 2.
    pub output_head_truncated_by_command: bool,
    /// True when `--amend` appears in the command. See D6.
    pub is_amend: bool,
    pub occurred_at: DateTime<Utc>,
}
