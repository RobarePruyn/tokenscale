//! cwd resolution helper, shared between server and ingest paths.
//!
//! `tokenscale-server::cwd_resolver` uses this at startup to build the
//! bidirectional raw-to-resolved map for the dashboard project filter
//! (v0.1.15). `tokenscale-store::commit_data` calls it at ingest time
//! to populate `session_commits.project_resolved` (v0.1.18 / Phase 2).
//!
//! Both call sites want the same shellout semantic, so the function
//! lives here in `tokenscale-core` where both can depend on it without
//! creating a cycle.

use std::process::Command;
use tracing::{debug, warn};

/// Resolve a raw cwd string to its git toplevel by shelling out
/// `git -C <cwd> rev-parse --show-toplevel`. Returns the toplevel on
/// success, the raw cwd unchanged on any failure.
///
/// Failure modes that fall through to raw:
/// - cwd no longer exists on disk
/// - cwd is not in a git repository
/// - git binary is not on PATH
/// - empty stdout (treated same as failure)
///
/// `git rev-parse` writes the toplevel to stdout on success and exits
/// 0; on failure it writes to stderr and exits non-zero. Only stdout
/// is read here; both non-zero exit and empty stdout fall through.
#[must_use]
pub fn resolve_to_git_toplevel(raw_cwd: &str) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(raw_cwd)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let trimmed = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if trimmed.is_empty() {
                debug!(cwd = raw_cwd, "git rev-parse returned empty stdout; falling back to raw");
                raw_cwd.to_owned()
            } else {
                trimmed
            }
        }
        Ok(_) => {
            // Non-zero exit: typically "not a git repository" or
            // "no such file or directory." Common enough that we log
            // at debug, not warn.
            debug!(cwd = raw_cwd, "git rev-parse returned non-zero; falling back to raw");
            raw_cwd.to_owned()
        }
        Err(e) => {
            // git binary not on PATH, permission denied on the exec,
            // or some other system-level error. Less common; warn
            // because it points at a broken environment.
            warn!(cwd = raw_cwd, error = %e, "git rev-parse failed to execute; falling back to raw");
            raw_cwd.to_owned()
        }
    }
}
