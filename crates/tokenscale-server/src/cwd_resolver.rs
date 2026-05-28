//! cwd → git toplevel resolution (granular-attribution Phase 1B-i, v0.1.15).
//!
//! Built at server startup. Enumerates the distinct `events.project_id`
//! values currently in the DB (each one a raw `cwd` string captured by
//! the CC JSONL parser), shells out `git -C <cwd> rev-parse
//! --show-toplevel` against each, and builds an in-memory bidirectional
//! map between raw cwds and their resolved git toplevels.
//!
//! Why a map and not a DB column: see `docs/roadmap-1b-cwd-resolution.md`
//! § D1. The recommendation that landed is query-time resolution — labels
//! are a presentation concern, not a data correctness concern, and a
//! filesystem-state-following map is the right shape for that. Zero
//! migration footprint, trivial downgrade.
//!
//! Failure modes are graceful: when `git rev-parse` fails (cwd no
//! longer exists on disk, cwd isn't in a git repo, git binary missing),
//! the raw cwd maps to itself. The dashboard renders the raw path —
//! same behavior as pre-v0.1.15.
//!
//! Worktree handling per D4: native `--show-toplevel` returns the
//! worktree path, so each worktree counts as a distinct project. The
//! A1 → A2 promotion (compose `--git-common-dir` to collapse worktrees
//! to the main worktree path) is a one-line future addition.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use tokenscale_core::resolve_to_git_toplevel;
use tokenscale_store::{list_projects_with_totals, Database, Result as StoreResult, ALL_PROVIDERS};
use tracing::info;

/// Bidirectional mapping between raw `cwd` strings and resolved git
/// toplevels.
///
/// Wrapped in `Arc` and stored on `AppState`. The forward map answers
/// "what's the resolved name for this raw cwd?" and the reverse map
/// answers "which raw cwds back this resolved project?" — both needed
/// for the project-filter expansion (user clicks a resolved-name chip;
/// server expands to the set of raw cwds for the SQL `IN` clause).
#[derive(Debug, Clone, Default)]
pub struct CwdResolver {
    /// Raw cwd → resolved project name. Resolved name is the git
    /// toplevel for git cwds, or the raw cwd itself for non-git
    /// directories / cwds that no longer exist on disk.
    forward: HashMap<String, String>,
    /// Resolved name → sorted list of raw cwds that map to it.
    /// `BTreeMap` for deterministic iteration in the projects list
    /// response; `BTreeSet` for the inner collection so duplicates
    /// can't sneak in.
    reverse: BTreeMap<String, BTreeSet<String>>,
}

impl CwdResolver {
    /// An empty resolver — every call to `resolve` returns the raw cwd.
    /// Used in tests and as a safe default before the real resolver
    /// has been built.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Resolve a raw cwd to its project name. Returns the resolved git
    /// toplevel when one was found at build time, otherwise the raw
    /// cwd unchanged. Both are valid `events.project_id` shapes — the
    /// dashboard accepts either.
    #[must_use]
    pub fn resolve<'a>(&'a self, raw_cwd: &'a str) -> &'a str {
        self.forward.get(raw_cwd).map_or(raw_cwd, String::as_str)
    }

    /// Reverse-lookup: given a resolved project name (as the frontend
    /// sends in a `?project=` filter), return every raw cwd that maps
    /// to it. Used to expand the user's filter selection into the SQL
    /// `WHERE project_id IN (...)` clause.
    ///
    /// When the resolved name has no entries in the reverse map
    /// (e.g. the user is filtering on a cwd the resolver hasn't seen
    /// — possible if a new cwd appeared in events between the
    /// resolver's startup build and the current query), falls back to
    /// returning the resolved name itself, matching the forward
    /// fallback shape. The SQL `IN` then exact-matches the raw cwd,
    /// which is correct for the unresolved-fallback case.
    #[must_use]
    pub fn raw_paths_for_resolved(&self, resolved: &str) -> Vec<String> {
        match self.reverse.get(resolved) {
            Some(set) => set.iter().cloned().collect(),
            None => vec![resolved.to_owned()],
        }
    }

    /// Number of distinct raw cwds known to the resolver. Used by
    /// telemetry and tests; not part of the request path.
    #[must_use]
    pub fn raw_cwd_count(&self) -> usize {
        self.forward.len()
    }

    /// Number of distinct resolved project names. After resolution
    /// collapses fragments, this is typically smaller than the raw
    /// count — the value Phase 1B's "67 projects fragmentation" was
    /// meant to compress.
    #[must_use]
    pub fn resolved_project_count(&self) -> usize {
        self.reverse.len()
    }
}

/// Build the resolver by enumerating distinct cwds from the DB and
/// shelling out `git rev-parse --show-toplevel` against each. Runs at
/// server startup, before the HTTP listener binds.
///
/// On any per-cwd resolution failure, falls back to raw — the
/// dashboard's pre-v0.1.15 behavior. Resolution failures are logged at
/// `debug` (not `warn`) because the most common cause is "this cwd
/// path isn't currently a git repo" which is expected for one-off
/// scratch sessions.
pub async fn build_resolver_from_db(database: &Database) -> StoreResult<CwdResolver> {
    // Enumerate distinct project_ids in the events table. Using an
    // ALL_PROVIDERS filter on a wide date window picks up everything
    // ever ingested; the resolver covers the full historical surface,
    // not just the current dashboard window. Cwds for events outside
    // the dashboard's selected window still need resolution if a user
    // widens the window later.
    let projects = list_projects_with_totals(
        database,
        // ISO date min — earliest plausible event date. Anything before
        // this would be pre-Anthropic-API-existence and isn't real
        // user data.
        "2022-01-01",
        // Far-future cap — picks up everything currently in the DB.
        "9999-12-31",
        ALL_PROVIDERS,
    )
    .await?;

    let raw_cwds: Vec<String> = projects.into_iter().map(|p| p.project_id).collect();
    Ok(build_resolver_from_raw_cwds(&raw_cwds))
}

/// Pure-function form of the resolver build, factored out so tests
/// can drive it with a synthetic cwd list without touching the DB.
#[must_use]
pub fn build_resolver_from_raw_cwds(raw_cwds: &[String]) -> CwdResolver {
    let mut forward: HashMap<String, String> = HashMap::with_capacity(raw_cwds.len());
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for raw in raw_cwds {
        let resolved = resolve_one(raw);
        forward.insert(raw.clone(), resolved.clone());
        reverse.entry(resolved).or_default().insert(raw.clone());
    }

    let resolver = CwdResolver { forward, reverse };
    info!(
        raw_cwd_count = resolver.raw_cwd_count(),
        resolved_project_count = resolver.resolved_project_count(),
        "cwd resolver built",
    );
    resolver
}

/// Single-cwd resolution. Delegates to
/// `tokenscale_core::resolve_to_git_toplevel`. The shellout logic was
/// promoted to `tokenscale-core` in v0.1.18 so both the server-side
/// resolver and the ingest-side `session_commits` insert path share
/// the same semantic.
fn resolve_one(raw_cwd: &str) -> String {
    resolve_to_git_toplevel(raw_cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command as StdCommand;

    /// Helper: create a throwaway git repo under a fresh temp dir,
    /// initialise it, and return the absolute path. The path is
    /// canonicalised so it matches what `git rev-parse --show-toplevel`
    /// returns (which is always the canonical, symlink-resolved path).
    fn make_temp_git_repo(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "tokenscale-cwd-test-{}-{}",
            std::process::id(),
            name,
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create temp dir");
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(&base)
            .arg("init")
            .arg("-q")
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        base.canonicalize().expect("canonicalize temp dir")
    }

    #[test]
    fn empty_resolver_returns_raw_for_everything() {
        let r = CwdResolver::empty();
        assert_eq!(r.resolve("/anything/at/all"), "/anything/at/all");
        assert_eq!(
            r.raw_paths_for_resolved("/queried/resolved"),
            vec!["/queried/resolved".to_owned()],
            "missing reverse entry falls back to the queried name itself",
        );
    }

    #[test]
    fn resolves_subdirectory_to_repo_toplevel() {
        let repo = make_temp_git_repo("subdir");
        let subdir = repo.join("nested").join("deeper");
        std::fs::create_dir_all(&subdir).expect("nested dir");

        let resolver = build_resolver_from_raw_cwds(&[
            repo.to_string_lossy().into_owned(),
            subdir.to_string_lossy().into_owned(),
        ]);

        // Both raw cwds (the repo root AND its nested subdir) should
        // resolve to the same toplevel — the fragmentation fix.
        let expected = repo.to_string_lossy().into_owned();
        assert_eq!(resolver.resolve(&repo.to_string_lossy()), expected.as_str());
        assert_eq!(resolver.resolve(&subdir.to_string_lossy()), expected.as_str());

        // resolved_project_count == 1: the two raw cwds collapsed.
        assert_eq!(resolver.resolved_project_count(), 1);
        assert_eq!(resolver.raw_cwd_count(), 2);

        // Reverse lookup returns both raw cwds for the SQL `IN` clause.
        let raws = resolver.raw_paths_for_resolved(&expected);
        assert_eq!(raws.len(), 2);
        assert!(raws.iter().any(|r| r == repo.to_string_lossy().as_ref()));
        assert!(raws.iter().any(|r| r == subdir.to_string_lossy().as_ref()));

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn non_git_directory_falls_back_to_raw() {
        let base = std::env::temp_dir().join(format!(
            "tokenscale-cwd-test-nongit-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create temp dir");

        let raw = base.canonicalize().unwrap().to_string_lossy().into_owned();
        let resolver = build_resolver_from_raw_cwds(&[raw.clone()]);

        // Non-git dir → raw cwd is its own resolved name (D3).
        assert_eq!(resolver.resolve(&raw), raw.as_str());
        assert_eq!(resolver.raw_paths_for_resolved(&raw), vec![raw.clone()]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_directory_falls_back_to_raw() {
        let raw = "/nonexistent/path/that/definitely/does/not/exist/anywhere";
        let resolver = build_resolver_from_raw_cwds(&[raw.to_owned()]);
        // Same fallback behavior as non-git: no resolution, raw is
        // its own resolved name. This is the "user moved the repo
        // since ingest" / "user deleted the repo" case (D1's
        // disk-state behavior).
        assert_eq!(resolver.resolve(raw), raw);
    }

    #[test]
    fn worktree_resolves_to_worktree_path_not_main_repo() {
        // D4: native `--show-toplevel` treats each worktree as a
        // distinct project. This test pins that decision against
        // the resolver — if a future change tried to compose
        // `--git-common-dir` for unified-repo grouping (A1 → A2),
        // this test would catch the regression.
        let main = make_temp_git_repo("worktree-main");
        // Need at least one commit before `git worktree add` works.
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(&main)
            .arg("commit")
            .arg("--allow-empty")
            .arg("-q")
            .arg("-m")
            .arg("init")
            .status()
            .expect("git commit");
        assert!(status.success());

        let worktree = std::env::temp_dir().join(format!(
            "tokenscale-cwd-test-wt-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&worktree);
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(&main)
            .arg("worktree")
            .arg("add")
            .arg("-q")
            .arg("-b")
            .arg("test-feat")
            .arg(&worktree)
            .arg("HEAD")
            .status()
            .expect("git worktree add");
        assert!(status.success(), "worktree add failed");
        let worktree = worktree.canonicalize().expect("canonicalize wt");

        let resolver = build_resolver_from_raw_cwds(&[
            main.to_string_lossy().into_owned(),
            worktree.to_string_lossy().into_owned(),
        ]);

        // Main and worktree resolve to DIFFERENT paths — worktree-as-project.
        let main_resolved = resolver.resolve(&main.to_string_lossy()).to_owned();
        let wt_resolved = resolver.resolve(&worktree.to_string_lossy()).to_owned();
        assert_ne!(
            main_resolved, wt_resolved,
            "D4: worktrees must NOT collapse to main repo path",
        );
        assert_eq!(resolver.resolved_project_count(), 2);

        // Cleanup.
        let _ = StdCommand::new("git")
            .arg("-C")
            .arg(&main)
            .arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(&worktree)
            .status();
        let _ = std::fs::remove_dir_all(&main);
        let _ = std::fs::remove_dir_all(&worktree);
    }

    #[test]
    fn reverse_lookup_groups_multiple_raws_under_one_resolved() {
        let repo = make_temp_git_repo("reverse");
        let sub_a = repo.join("a");
        let sub_b = repo.join("b").join("nested");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();

        let resolver = build_resolver_from_raw_cwds(&[
            repo.to_string_lossy().into_owned(),
            sub_a.to_string_lossy().into_owned(),
            sub_b.to_string_lossy().into_owned(),
        ]);

        let resolved = repo.to_string_lossy().into_owned();
        let raws = resolver.raw_paths_for_resolved(&resolved);
        assert_eq!(raws.len(), 3, "all three raw cwds must reverse-map");

        let _ = std::fs::remove_dir_all(&repo);
    }
}
