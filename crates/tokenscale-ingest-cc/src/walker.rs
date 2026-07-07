//! Filesystem walker: find Claude Code session JSONL files under a root.
//!
//! Layout we walk (recursively, within each project directory):
//!
//! ```text
//! <root>/
//!   -Users-r-Dev-QTrial/
//!     455218e7-....jsonl                 (top-level session transcript)
//!     455218e7-...../
//!       subagents/
//!         agent-abc.jsonl                (subagent transcript, v0.1.19+)
//!   -Users-r-Dev-Other/
//!     ...
//! ```
//!
//! The slug-encoded directory name is preserved as the file path; we do not
//! try to reconstruct the original cwd from it. The actual cwd lands in each
//! event via the `cwd` field on the JSONL line itself.

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::warn;

use crate::error::{IngestError, Result};

/// One JSONL file ready for ingest.
#[derive(Debug, Clone)]
pub struct JsonlFile {
    pub path: PathBuf,
    /// Modification time as nanoseconds since the unix epoch. Compared
    /// alongside `len` to skip re-parsing unchanged files.
    pub mtime_ns: i64,
    /// File size in bytes. Tracked alongside `mtime_ns` so cloud-FS
    /// drift (synced bytes with preserved mtime, or vice versa) still
    /// triggers a re-parse.
    pub len: i64,
}

/// Walk every configured Claude Code root and return the union of
/// JSONL files found. A missing root logs a warning and is skipped
/// (multi-machine setups regularly list roots that are mirrored from
/// elsewhere — the mirror's first sync may not have landed yet, but
/// that shouldn't block scanning of the other roots). An empty input
/// is an error to surface misconfiguration loudly.
///
/// `_ingest_file_state` keys by full path, so identically-named files
/// under different roots don't collide even when filenames repeat —
/// e.g., two machines each have a `session-abc.jsonl` in their own
/// `~/.claude/projects/myproject/`.
pub async fn walk_claude_code_roots(claude_code_roots: &[std::path::PathBuf]) -> Result<Vec<JsonlFile>> {
    if claude_code_roots.is_empty() {
        return Err(IngestError::RootNotFound(std::path::PathBuf::new()));
    }
    let mut all_files = Vec::new();
    for root in claude_code_roots {
        match walk_claude_code_root(root).await {
            Ok(mut files) => all_files.append(&mut files),
            Err(IngestError::RootNotFound(missing)) => {
                warn!(path = %missing.display(), "claude_code_roots entry does not exist; skipping");
            }
            Err(other) => return Err(other),
        }
    }
    all_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(all_files)
}

/// Return every `*.jsonl` file under `claude_code_root`, recursing through
/// each project directory's full subtree. Returns an error if the root
/// itself doesn't exist; an individual unreadable subdirectory only logs a
/// warning and is skipped.
///
/// v0.1.19: the inner walk recurses (was one level deep) so subagent
/// transcripts at `<project>/<session-id>/subagents/agent-*.jsonl` are
/// collected alongside top-level session files. Subagent token spend is
/// real account usage that prior versions silently dropped. Stray `*.jsonl`
/// sitting directly at the root level are still skipped: only direct child
/// directories of the root (the project directories) are descended.
pub async fn walk_claude_code_root(claude_code_root: &Path) -> Result<Vec<JsonlFile>> {
    if !claude_code_root.exists() {
        return Err(IngestError::RootNotFound(claude_code_root.to_path_buf()));
    }

    let mut found_files = Vec::new();
    let mut project_directories = match tokio::fs::read_dir(claude_code_root).await {
        Ok(directory_iterator) => directory_iterator,
        Err(io_error) => return Err(IngestError::Io(io_error)),
    };

    while let Some(entry) = project_directories.next_entry().await? {
        let project_path = entry.path();
        if !project_path.is_dir() {
            // Files directly at the root level are Claude Code metadata,
            // not session transcripts — skip them (matches pre-v0.1.19
            // behavior). Only project directories are descended.
            continue;
        }
        collect_jsonl_recursive(&project_path, &mut found_files).await?;
    }

    found_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found_files)
}

/// Iterative depth-first walk of a project directory's subtree, pushing
/// every `*.jsonl` file into `found_files`. Iterative (explicit stack)
/// rather than recursive-async to avoid boxing the recursive future.
/// `DirEntry::file_type` does not follow symlinks, so symlinked
/// subdirectories are not descended — there are none in the Claude Code
/// transcript layout, and this rules out symlink cycles by construction.
async fn collect_jsonl_recursive(
    project_dir: &Path,
    found_files: &mut Vec<JsonlFile>,
) -> Result<()> {
    let mut stack: Vec<PathBuf> = vec![project_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(directory_iterator) => directory_iterator,
            Err(io_error) => {
                warn!(path = %dir.display(), error = %io_error, "skipping unreadable directory");
                continue;
            }
        };

        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(io_error) => {
                    warn!(path = %entry_path.display(), error = %io_error, "skipping unreadable entry");
                    continue;
                }
            };

            if file_type.is_dir() {
                stack.push(entry_path);
                continue;
            }
            if entry_path.extension().is_none_or(|ext| ext != "jsonl") {
                continue;
            }
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(io_error) => {
                    warn!(path = %entry_path.display(), error = %io_error, "skipping unreadable session file");
                    continue;
                }
            };
            let mtime_ns = system_time_to_unix_nanos(metadata.modified()?);
            let len = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
            found_files.push(JsonlFile {
                path: entry_path,
                mtime_ns,
                len,
            });
        }
    }

    Ok(())
}

/// Convert a `SystemTime` to nanoseconds since the unix epoch, saturating
/// at i64 bounds. Negative result for pre-epoch times (which we never expect
/// for Claude Code logs but handle defensively).
fn system_time_to_unix_nanos(system_time: SystemTime) -> i64 {
    match system_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX),
        Err(error) => {
            // Pre-epoch — duration is negative.
            -i64::try_from(error.duration().as_nanos()).unwrap_or(i64::MAX)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn walker_finds_jsonl_files_one_level_deep() -> Result<()> {
        let temp_root = TempDir::new()?;
        let project_a = temp_root.path().join("project-a");
        let project_b = temp_root.path().join("project-b");
        fs::create_dir(&project_a)?;
        fs::create_dir(&project_b)?;
        fs::write(project_a.join("session-1.jsonl"), b"{}\n")?;
        fs::write(project_a.join("session-2.jsonl"), b"{}\n")?;
        fs::write(project_a.join("not-jsonl.txt"), b"ignore me")?;
        fs::write(project_b.join("session-3.jsonl"), b"{}\n")?;

        let files = walk_claude_code_root(temp_root.path()).await?;
        let names: Vec<&str> = files
            .iter()
            .map(|file| file.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["session-1.jsonl", "session-2.jsonl", "session-3.jsonl"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn walker_returns_root_not_found_when_missing() {
        let result = walk_claude_code_root(Path::new("/this/path/does/not/exist/zzz")).await;
        assert!(matches!(result, Err(IngestError::RootNotFound(_))));
    }

    #[tokio::test]
    async fn walker_handles_empty_root() -> Result<()> {
        let temp_root = TempDir::new()?;
        let files = walk_claude_code_root(temp_root.path()).await?;
        assert!(files.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn walker_skips_files_at_root_level() -> Result<()> {
        // Files directly in `~/.claude/projects/` (not in a subdirectory) are
        // not session files — they're claude code metadata. Walker should
        // ignore them silently.
        let temp_root = TempDir::new()?;
        fs::write(temp_root.path().join("stray.jsonl"), b"{}\n")?;
        let files = walk_claude_code_root(temp_root.path()).await?;
        assert!(files.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn walker_recurses_into_subagent_subdirectories() -> Result<()> {
        // v0.1.19 (D4): subagent transcripts live at
        // <project>/<session-id>/subagents/agent-*.jsonl and must be ingested
        // alongside the top-level session file. Pre-v0.1.19 the walker read
        // only direct children of the project dir and silently dropped them.
        // This test fails against the old one-level-deep walker and passes
        // against the recursive one.
        let temp_root = TempDir::new()?;
        let project = temp_root.path().join("-Users-r-Dev-Mediacast");
        let session_subagents = project.join("session-abc").join("subagents");
        fs::create_dir_all(&session_subagents)?;
        fs::write(project.join("session-abc.jsonl"), b"{}\n")?; // top-level
        fs::write(session_subagents.join("agent-1.jsonl"), b"{}\n")?; // subagent
        fs::write(session_subagents.join("agent-2.jsonl"), b"{}\n")?; // subagent
        // A non-jsonl artifact deeper in the tree must still be ignored.
        fs::write(session_subagents.join("notes.md"), b"ignore me")?;

        let files = walk_claude_code_root(temp_root.path()).await?;
        let names: Vec<String> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            files.len(),
            3,
            "top-level session + 2 subagent transcripts; got {names:?}"
        );
        assert!(names.contains(&"session-abc.jsonl".to_owned()));
        assert!(names.contains(&"agent-1.jsonl".to_owned()));
        assert!(names.contains(&"agent-2.jsonl".to_owned()));
        Ok(())
    }
}
