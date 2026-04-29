//! Filesystem walker — find Claude Code session JSONL files under a root.
//!
//! Layout we walk:
//!
//! ```text
//! <root>/
//!   -Users-r-Dev-QTrial/
//!     455218e7-....jsonl
//!     ...
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
    /// Modification time as nanoseconds since the unix epoch. Used to skip
    /// re-parsing unchanged files.
    pub mtime_ns: i64,
}

/// Walk one level deep under `claude_code_root` and return every `*.jsonl`
/// file encountered. Returns an error if the root itself doesn't exist; an
/// individual unreadable subdirectory only logs a warning and is skipped.
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
            continue;
        }

        let mut session_files = match tokio::fs::read_dir(&project_path).await {
            Ok(directory_iterator) => directory_iterator,
            Err(io_error) => {
                warn!(path = %project_path.display(), error = %io_error, "skipping unreadable project directory");
                continue;
            }
        };

        while let Some(session_entry) = session_files.next_entry().await? {
            let session_path = session_entry.path();
            if session_path.extension().is_none_or(|ext| ext != "jsonl") {
                continue;
            }
            let metadata = match session_entry.metadata().await {
                Ok(metadata) => metadata,
                Err(io_error) => {
                    warn!(path = %session_path.display(), error = %io_error, "skipping unreadable session file");
                    continue;
                }
            };
            let mtime_ns = system_time_to_unix_nanos(metadata.modified()?);
            found_files.push(JsonlFile {
                path: session_path,
                mtime_ns,
            });
        }
    }

    found_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found_files)
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
}
