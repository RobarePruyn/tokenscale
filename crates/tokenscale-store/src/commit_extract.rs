//! Phase 2 commit-attribution extraction (v0.1.18).
//!
//! Pure functions that turn a Bash `ToolUse` plus its optional
//! `ToolResult` into a `SessionCommit` (when the tool_use is actually
//! a `git commit` invocation). No DB, no async, no IO beyond the
//! shellout in `cwd::resolve_to_git_toplevel` which the caller
//! invokes separately.
//!
//! Three jobs:
//!
//! 1. `is_real_commit_command(command)` answers "does this Bash
//!    invocation contain a real `git commit` subcommand?" using the
//!    shlex token-walk filter (D-decision §1.1 in the scoping doc).
//!    The naive `LIKE '%git commit%'` overcounts by 4.7x because
//!    probe scripts, sqlite queries, and `grep "git commit"` lines
//!    all match.
//!
//! 2. `extract_sha(content)` applies the primary regex
//!    `[<branch>( \(root-commit\))? <sha>]` to a tool_result content
//!    string. On miss, applies the push-refspec fallback regex
//!    `<old>..<new>  <local> -> <remote>`. Returns the SHA plus the
//!    `RecoverySource` enum.
//!
//! 3. `extract_session_commit(...)` wires the above together with
//!    flag detection (`--amend`, `output_head_truncated_by_command`)
//!    and cd-target extraction.

use regex::Regex;
use std::sync::OnceLock;
use tokenscale_core::{RecoverySource, SessionCommit, ToolResult, ToolUse};

/// The canonical filter from § 1.1 of the scoping doc. Returns true
/// when the command's structure (after shlex tokenisation, split on
/// `&&` / `;` / `||`) contains an actual `git commit` subcommand.
///
/// Catches meta-mentions: the command's first non-empty line must not
/// start with `sqlite3`, `python3`, `grep `, `awk `, `echo `, `cat `
/// (a probe-script heuristic that captures the common false positives
/// observed in the maintainer corpus).
#[must_use]
pub fn is_real_commit_command(command: &str) -> bool {
    const BAD_PREFIXES: &[&str] = &[
        "sqlite3", "python3", "grep ", "awk ", "echo ", "cat ", "=== ", "--- ",
    ];
    if command.is_empty() {
        return false;
    }
    let first_line = command.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let first_trimmed = first_line.trim_start();
    for pfx in BAD_PREFIXES {
        if first_trimmed.starts_with(pfx) {
            return false;
        }
    }
    // Split on top-level subcommand separators. We do this BEFORE
    // shlex so multiline HEREDOC bodies don't break tokenisation.
    for sub in split_subcommands(command) {
        let sub_trimmed = sub.trim();
        if sub_trimmed.is_empty() {
            continue;
        }
        // shlex returns None on unbalanced quotes (which happens with
        // HEREDOCs that include the marker but not the body in the
        // sub-slice). Skip those; the real `git commit` token won't
        // be inside an unbalanced subslice.
        let Some(tokens) = shlex::split(sub_trimmed) else {
            continue;
        };
        if tokens.len() >= 2 && tokens[0] == "git" && tokens[1] == "commit" {
            return true;
        }
    }
    false
}

/// Split a command string on top-level `&&`, `;`, `||` separators.
///
/// **§9 release-gate fix (v0.1.18 smoke)**: heredoc bodies are
/// stripped before splitting. The original implementation traversed
/// heredoc body content as if it were shell, which caused probe
/// scripts (Python heredocs containing `git commit ...` fragments)
/// to slip past the filter when the heredoc body happened to contain
/// `&&` or `;` separators. The fix recognises `<<MARKER`,
/// `<<'MARKER'`, `<<"MARKER"`, `<<-MARKER` etc. and skips lines
/// until a line equal to MARKER is found.
fn split_subcommands(command: &str) -> Vec<String> {
    let cleaned = strip_heredoc_bodies(command);
    let bytes = cleaned.as_bytes();
    let mut result = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let two = if i + 1 < bytes.len() {
            &bytes[i..=i + 1]
        } else {
            &[][..]
        };
        if two == b"&&" || two == b"||" {
            result.push(cleaned[start..i].to_owned());
            i += 2;
            start = i;
            continue;
        }
        if bytes[i] == b';' {
            result.push(cleaned[start..i].to_owned());
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    if start < bytes.len() {
        result.push(cleaned[start..].to_owned());
    }
    result
}

/// Strip heredoc bodies from a command string. Keeps the line that
/// opens the heredoc (the line containing `<<MARKER`) so the
/// surrounding command structure is preserved; drops every body line
/// and the closing-marker line until the next heredoc or end of
/// input.
///
/// Recognises:
/// - `<<MARKER`            (unquoted, parameter-expanded body)
/// - `<<'MARKER'`          (single-quoted, no expansion)
/// - `<<"MARKER"`          (double-quoted, expanded; rare)
/// - `<<-MARKER` variants  (leading-tab strip; same body semantics)
///
/// The closing marker must be alone on its line (per POSIX; leading
/// tabs allowed if `<<-` was used). Body lines that happen to contain
/// the marker word as a substring are NOT treated as the closer.
fn strip_heredoc_bodies(command: &str) -> String {
    static OPEN_RE: OnceLock<Regex> = OnceLock::new();
    let open_re = OPEN_RE.get_or_init(|| {
        // Capture the marker word. Allow optional leading '-' on the
        // `<<` (the `<<-` form) and optional quoting around the marker.
        Regex::new(r#"<<-?\s*(?:'([A-Za-z_][A-Za-z0-9_]*)'|"([A-Za-z_][A-Za-z0-9_]*)"|([A-Za-z_][A-Za-z0-9_]*))"#)
            .expect("heredoc-open regex is valid")
    });

    let mut out = String::with_capacity(command.len());
    let mut iter = command.lines().peekable();
    while let Some(line) = iter.next() {
        // Always emit the current line (the heredoc opener stays).
        out.push_str(line);
        out.push('\n');
        // Check whether this line opens a heredoc.
        let Some(caps) = open_re.captures(line) else {
            continue;
        };
        let marker = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().to_owned());
        let Some(marker) = marker else { continue };
        // Skip body lines until we hit the closer alone on its line.
        // POSIX allows leading whitespace stripping with `<<-`; accept
        // either zero or arbitrary leading whitespace to be lenient.
        for body in iter.by_ref() {
            if body.trim() == marker {
                break;
            }
            // Drop body lines (do not append to out).
        }
    }
    // If the input didn't end with a newline, our re-join added one;
    // trim it to keep round-trip behavior reasonable on edge inputs.
    if !command.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Primary regex: `[<branch>( \(root-commit\))? <sha>]` followed by
/// a space and the commit subject. Branch can be any non-bracket
/// non-whitespace characters.
fn primary_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\[(?P<branch>[^\]]+?)(?: \(root-commit\))? +(?P<sha>[0-9a-f]{7,40})\] ")
            .expect("primary commit-line regex is valid")
    })
}

/// Push-refspec fallback regex: `<old>..<new>  <local> -> <remote>`
/// pattern that `git push` writes to stderr+stdout. The `new` capture
/// is the post-commit HEAD by git's definition.
fn push_refspec_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?P<old>[0-9a-f]{7,40})\.\.(?P<new>[0-9a-f]{7,40})\b\s+\S+\s+->\s+\S+")
            .expect("push-refspec regex is valid")
    })
}

/// Output of `extract_sha`: the captured SHA (if any) plus which
/// regex captured it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSha {
    pub sha: Option<String>,
    pub recovery_source: RecoverySource,
}

/// Apply the two-regex capture strategy to a tool_result content
/// string. Primary regex first; push-refspec fallback on miss.
#[must_use]
pub fn extract_sha(content: &str) -> ExtractedSha {
    if let Some(caps) = primary_re().captures(content) {
        return ExtractedSha {
            sha: Some(caps.name("sha").map(|m| m.as_str().to_owned()).unwrap_or_default()),
            recovery_source: RecoverySource::Primary,
        };
    }
    // Take the FIRST push-refspec match in the content (load-bearing
    // rule per D2 failure mode 2). For most cases there is only one.
    if let Some(caps) = push_refspec_re().captures(content) {
        return ExtractedSha {
            sha: Some(caps.name("new").map(|m| m.as_str().to_owned()).unwrap_or_default()),
            recovery_source: RecoverySource::PushRefspec,
        };
    }
    ExtractedSha {
        sha: None,
        recovery_source: RecoverySource::None,
    }
}

/// Extract the `cd "<path>"` target from the first cd-subcommand in
/// the command string. Returns the verbatim path (after shlex
/// dequoting). `None` when no cd prefix appears.
///
/// Looks at every subcommand, not just the first, so that a sequence
/// like `git status && cd "/path" && git commit ...` returns the cd
/// target correctly.
#[must_use]
pub fn extract_cd_target(command: &str) -> Option<String> {
    for sub in split_subcommands(command) {
        let sub_trimmed = sub.trim();
        if sub_trimmed.is_empty() {
            continue;
        }
        let Some(tokens) = shlex::split(sub_trimmed) else {
            continue;
        };
        if tokens.first().is_some_and(|t| t == "cd") && tokens.len() >= 2 {
            return Some(tokens[1].clone());
        }
    }
    None
}

/// Detect output-compressing pipes that drop git commit's output
/// head. Currently matches `| tail` anywhere in the command.
///
/// **Deliberate scope (per D5 Part 2 sub-decision)**: matches `| tail`
/// inside HEREDOC bodies as well, which is a theoretical false
/// positive (a commit message body could literally say "| tail" in
/// prose). The empirical false-positive rate in the corpus is zero;
/// YAGNI applies until evidence surfaces.
#[must_use]
pub fn is_output_head_truncated_by_command(command: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\|\s*tail\b").expect("tail-pipe regex is valid"));
    re.is_match(command)
}

/// Detect `--amend` anywhere in the command. False positives are
/// possible if `--amend` appears in a HEREDOC body; empirical rate
/// in the corpus is zero. See D6.
#[must_use]
pub fn is_amend_command(command: &str) -> bool {
    command.contains("--amend")
}

/// Top-level extraction: turn a Bash ToolUse plus its optional
/// ToolResult into a SessionCommit. Returns `None` when the tool_use
/// is not a real `git commit` invocation, or when the tool_use has
/// no session_id (defensive; the schema requires NOT NULL).
///
/// The `project_resolved` arg must be precomputed by the caller via
/// `tokenscale_core::resolve_to_git_toplevel`. The caller batches
/// cwd resolution across many tool_uses to avoid per-row shell spawns.
pub fn extract_session_commit(
    tool_use: &ToolUse,
    tool_result: Option<&ToolResult>,
    project_resolved: String,
) -> Option<SessionCommit> {
    if tool_use.tool_name != "Bash" {
        return None;
    }
    let session_id = tool_use.session_id.clone()?;
    // Extract the command from the input_json. Stored as JSON text;
    // the field shape is `{ "command": "...", "description": "..." }`.
    let command = serde_json::from_str::<serde_json::Value>(&tool_use.input_json)
        .ok()
        .and_then(|v| {
            v.get("command")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })?;

    if !is_real_commit_command(&command) {
        return None;
    }

    let extracted = match tool_result {
        Some(tr) => extract_sha(&tr.content),
        None => ExtractedSha {
            sha: None,
            recovery_source: RecoverySource::None,
        },
    };

    Some(SessionCommit {
        source: tool_use.source.clone(),
        tool_use_id: tool_use.tool_use_id.clone(),
        session_id,
        sha: extracted.sha,
        cd_target_raw: extract_cd_target(&command),
        project_resolved,
        recovery_source: extracted.recovery_source,
        output_head_truncated_by_command: is_output_head_truncated_by_command(&command),
        is_amend: is_amend_command(&command),
        occurred_at: tool_use.occurred_at,
    })
}

/// Detect whether a project_resolved path is a /tmp testing exercise.
/// Used by the query layer to default-filter testing commits per
/// D4a; opt-in via `?include_testing=true`.
#[must_use]
pub fn is_testing_project(project_resolved: &str) -> bool {
    project_resolved == "/tmp"
        || project_resolved == "/private/tmp"
        || project_resolved.starts_with("/tmp/")
        || project_resolved.starts_with("/private/tmp/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn sample_tool_use(command: &str, tool_use_id: &str) -> ToolUse {
        ToolUse {
            tool_use_id: tool_use_id.to_owned(),
            parent_event_uuid: "parent-uuid".to_owned(),
            source: "claude_code".to_owned(),
            tool_name: "Bash".to_owned(),
            input_json: serde_json::json!({ "command": command }).to_string(),
            occurred_at: Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap(),
            session_id: Some("sess-1".to_owned()),
            project_id: Some("/tmp/sample".to_owned()),
        }
    }

    fn sample_tool_result(tool_use_id: &str, content: &str) -> ToolResult {
        ToolResult {
            tool_use_id: tool_use_id.to_owned(),
            parent_event_uuid: "user-uuid".to_owned(),
            source: "claude_code".to_owned(),
            content: content.to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 1).unwrap(),
            session_id: Some("sess-1".to_owned()),
        }
    }

    // ---------- is_real_commit_command ----------

    #[test]
    fn real_commit_canonical_heredoc_form_passes_filter() {
        let cmd = "cd \"/path\" && git add . && git commit -m \"$(cat <<'EOF'\nsubject\nEOF\n)\" && git status";
        assert!(is_real_commit_command(cmd));
    }

    #[test]
    fn standalone_git_commit_passes_filter() {
        assert!(is_real_commit_command("git commit -m msg"));
    }

    #[test]
    fn grep_for_git_commit_does_not_pass_filter() {
        assert!(!is_real_commit_command("grep 'git commit' file.txt"));
    }

    #[test]
    fn sqlite_query_mentioning_git_commit_does_not_pass_filter() {
        let q = "sqlite3 db \"SELECT * FROM tool_uses WHERE input_json LIKE '%git commit%'\"";
        assert!(!is_real_commit_command(q));
    }

    #[test]
    fn python_script_mentioning_git_commit_does_not_pass_filter() {
        assert!(!is_real_commit_command("python3 -c 'print(\"git commit\")'"));
    }

    #[test]
    fn empty_command_does_not_pass_filter() {
        assert!(!is_real_commit_command(""));
    }

    #[test]
    fn chained_status_then_commit_passes_filter() {
        let cmd = "git status && cd /repo && git commit -m msg";
        assert!(is_real_commit_command(cmd));
    }

    // ----------------------------------------------------------------
    // §9 release-gate smoke regression (v0.1.18). Probe scripts with
    // heredoc'd Python that contains `git commit` substring fragments
    // must NOT pass the filter. The pre-fix split_subcommands
    // traversed heredoc body content as if it were shell, so a
    // Python comment line like
    //     # Also handle '... && cd "..." && git commit ...'
    // got split on the `&&` inside the comment string and the
    // resulting fragment after the last `&&` started with
    // `git commit ...` whose shlex tokens were ['git', 'commit', ...].
    // The fix strips heredoc bodies before subcommand splitting.
    //
    // The first test below pins the broad pattern (probe-style
    // command). The second test is the DISCRIMINATING one: it FAILS
    // when strip_heredoc_bodies is bypassed (verified by patching
    // the helper to a no-op during the §9 sanity check). The first
    // test alone does not discriminate because its Python body never
    // produces a fragment whose first two shlex tokens are git+commit.
    // ----------------------------------------------------------------
    #[test]
    fn probe_script_with_heredoc_python_body_does_not_pass_filter() {
        let cmd = "DB=\"$HOME/Library/Application Support/tokenscale/tokenscale.db\"\npython3 <<'PY'\nimport sqlite3, re\ncon = sqlite3.connect(DB)\nrows = con.execute(\"SELECT input_json FROM tool_uses WHERE input_json LIKE '%git commit%'\").fetchall()\nfor sub in re.split(r'&&|;|\\|\\|', cmd):\n    if toks[0] == 'git' and toks[1] == 'commit':\n        return True\nPY";
        assert!(
            !is_real_commit_command(cmd),
            "probe scripts that heredoc Python with `git commit` substring must NOT pass the shlex filter; heredoc bodies are stripped before subcommand splitting"
        );
    }

    #[test]
    fn discriminating_test_heredoc_body_with_amp_amp_git_commit_fragment_fails_filter() {
        // Reproduces the actual smoke-found bug shape: a Python
        // heredoc body containing `... && git commit FOO'bar baz'`
        // produces, after splitting on `&&`, a balanced-quote
        // fragment whose first two shlex tokens are `git` and
        // `commit`. The pre-fix filter accepted; the post-fix
        // filter (with strip_heredoc_bodies removing the body) does
        // not split on the body content at all and correctly rejects.
        //
        // Why the balanced quotes matter: an unbalanced single quote
        // in the fragment makes shlex.split return None (which the
        // filter treats as "skip"), so the bug only manifests when
        // the fragment's quotes happen to balance. The smoke-found
        // case had matching quotes; this test reproduces that exact
        // shape.
        //
        // Verified to FAIL without strip_heredoc_bodies via the §9
        // sanity-check bypass-patch. Removing this test would
        // re-open the slip pattern to regression.
        let cmd = "DB=x\npython3 <<'PY'\n# example: ... && git commit foo'bar baz'qux\nPY";
        assert!(
            !is_real_commit_command(cmd),
            "the heredoc body fragment `&& git commit foo'bar baz'qux` (balanced single-quotes) must NOT cause the filter to accept; strip_heredoc_bodies removes the body before subcommand splitting, so no `git commit`-starting fragment exists"
        );
    }

    #[test]
    fn strip_heredoc_bodies_handles_single_and_double_quoted_markers() {
        let cmd = "echo first; cat <<'EOF'\nbody line 1 with && and ;\nEOF\necho middle; cat <<\"END\"\nbody line with || and ;\nEND\necho last";
        let stripped = strip_heredoc_bodies(cmd);
        assert!(stripped.contains("echo first"));
        assert!(stripped.contains("echo middle"));
        assert!(stripped.contains("echo last"));
        assert!(stripped.contains("cat <<'EOF'"));
        assert!(stripped.contains("cat <<\"END\""));
        assert!(!stripped.contains("body line 1 with"));
        assert!(!stripped.contains("body line with"));
    }

    #[test]
    fn strip_heredoc_bodies_handles_unquoted_marker() {
        let cmd = "cat <<MARK\nbody with junk && hello\nMARK\necho after";
        let stripped = strip_heredoc_bodies(cmd);
        assert!(stripped.contains("cat <<MARK"));
        assert!(stripped.contains("echo after"));
        assert!(!stripped.contains("body with junk"));
    }

    #[test]
    fn split_subcommands_does_not_split_on_separators_inside_heredoc_body() {
        // The heredoc body contains `&&`, `;`, and `||` but those are
        // inside a Python source body, not shell separators. Splitting
        // must see only the outer subcommands.
        let cmd = "cd /repo && cat <<'PY'\nif a && b: pass; foo || bar\nPY\ngit status";
        let subs = split_subcommands(cmd);
        // Outer subcommands: "cd /repo", then "cat <<'PY' ... PY\ngit status"
        assert_eq!(
            subs.len(),
            2,
            "should see 2 subcommands (cd, then cat-plus-git-status); got {subs:?}"
        );
    }

    // ---------- extract_sha ----------

    #[test]
    fn primary_regex_captures_canonical_branch_sha_line() {
        let content = "[main aa7f38a] Add docs\n 1 file changed, 5 insertions(+)";
        let got = extract_sha(content);
        assert_eq!(got.sha.as_deref(), Some("aa7f38a"));
        assert_eq!(got.recovery_source, RecoverySource::Primary);
    }

    #[test]
    fn primary_regex_captures_root_commit_variant() {
        let content = "[main (root-commit) 5a39710] First commit\n 6 files changed";
        let got = extract_sha(content);
        assert_eq!(got.sha.as_deref(), Some("5a39710"));
        assert_eq!(got.recovery_source, RecoverySource::Primary);
    }

    #[test]
    fn push_refspec_fallback_captures_when_primary_misses() {
        let content = " 3 files changed, 329 insertions(+)\nTo https://github.com/example/repo.git\n   8386b26..d9bd400  main -> main";
        let got = extract_sha(content);
        assert_eq!(got.sha.as_deref(), Some("d9bd400"));
        assert_eq!(got.recovery_source, RecoverySource::PushRefspec);
    }

    #[test]
    fn no_sha_recoverable_when_both_regexes_miss() {
        let content = "Exit code 1\nThe following paths are ignored by one of your .gitignore files:";
        let got = extract_sha(content);
        assert_eq!(got.sha, None);
        assert_eq!(got.recovery_source, RecoverySource::None);
    }

    #[test]
    fn primary_wins_over_push_refspec_when_both_present() {
        // The canonical chained case: commit line + push line in same result.
        let content = "[main abc1234] subject\n 1 file changed\nTo https://example.com/r.git\n   0000111..2222333  main -> main";
        let got = extract_sha(content);
        assert_eq!(got.sha.as_deref(), Some("abc1234"));
        assert_eq!(got.recovery_source, RecoverySource::Primary);
    }

    // ---------- extract_cd_target ----------

    #[test]
    fn cd_target_extracted_from_quoted_path() {
        let cmd = "cd \"/path/with space\" && git commit -m msg";
        assert_eq!(extract_cd_target(cmd), Some("/path/with space".to_owned()));
    }

    #[test]
    fn cd_target_extracted_from_unquoted_path() {
        assert_eq!(
            extract_cd_target("cd /tmp/foo && git commit -m msg"),
            Some("/tmp/foo".to_owned()),
        );
    }

    #[test]
    fn cd_target_extracted_from_backslash_escaped_path() {
        // iCloud-style path with backslash-escaped spaces.
        let cmd = r"cd /Users/Robare/Library/Mobile\ Documents/test && git commit -m msg";
        assert_eq!(
            extract_cd_target(cmd),
            Some("/Users/Robare/Library/Mobile Documents/test".to_owned()),
        );
    }

    #[test]
    fn cd_target_none_when_no_cd_prefix() {
        assert_eq!(extract_cd_target("git commit -m msg"), None);
    }

    #[test]
    fn cd_target_found_when_cd_appears_mid_chain() {
        let cmd = "git status && cd /repo && git commit -m msg";
        assert_eq!(extract_cd_target(cmd), Some("/repo".to_owned()));
    }

    // ---------- flag detection ----------

    #[test]
    fn output_head_truncated_detects_tail_pipe() {
        assert!(is_output_head_truncated_by_command(
            "git commit -m msg 2>&1 | tail -5"
        ));
        assert!(is_output_head_truncated_by_command(
            "git commit -m msg | tail -3 && git push | tail -3"
        ));
    }

    #[test]
    fn output_head_truncated_false_without_tail() {
        assert!(!is_output_head_truncated_by_command(
            "git commit -m \"$(cat <<'EOF'\nsubject\nEOF\n)\" && git status"
        ));
    }

    #[test]
    fn amend_detection_finds_flag() {
        assert!(is_amend_command("git commit --amend -m msg"));
        assert!(is_amend_command("git commit --amend --no-edit"));
        assert!(!is_amend_command("git commit -m msg"));
    }

    // ---------- /tmp filter ----------

    #[test]
    fn testing_project_filter_catches_tmp_paths() {
        assert!(is_testing_project("/tmp"));
        assert!(is_testing_project("/tmp/main-repo"));
        assert!(is_testing_project("/private/tmp"));
        assert!(is_testing_project("/private/tmp/wt-test-2"));
        assert!(!is_testing_project("/Users/foo/dev/repo"));
        assert!(!is_testing_project("/home/foo/tmp-not-prefix"));
    }

    // ---------- extract_session_commit (top-level) ----------

    #[test]
    fn extract_session_commit_returns_some_for_real_commit_with_sha() {
        let tu = sample_tool_use(
            "cd \"/repo\" && git commit -m msg",
            "toolu_aaaa",
        );
        let tr = sample_tool_result("toolu_aaaa", "[main abc1234] subject");
        let sc = extract_session_commit(&tu, Some(&tr), "/repo".to_owned())
            .expect("real commit should extract");
        assert_eq!(sc.tool_use_id, "toolu_aaaa");
        assert_eq!(sc.sha.as_deref(), Some("abc1234"));
        assert_eq!(sc.recovery_source, RecoverySource::Primary);
        assert_eq!(sc.cd_target_raw.as_deref(), Some("/repo"));
        assert_eq!(sc.project_resolved, "/repo");
        assert!(!sc.is_amend);
        assert!(!sc.output_head_truncated_by_command);
    }

    #[test]
    fn extract_session_commit_returns_none_for_non_commit_bash() {
        let tu = sample_tool_use("git status && ls -la", "toolu_bbbb");
        let tr = sample_tool_result("toolu_bbbb", "stuff");
        assert!(extract_session_commit(&tu, Some(&tr), "/x".to_owned()).is_none());
    }

    #[test]
    fn extract_session_commit_returns_none_for_meta_mention() {
        let tu = sample_tool_use("grep 'git commit' file.txt", "toolu_cccc");
        let tr = sample_tool_result("toolu_cccc", "matches");
        assert!(extract_session_commit(&tu, Some(&tr), "/x".to_owned()).is_none());
    }

    #[test]
    fn extract_session_commit_handles_orphan_with_none_sha() {
        let tu = sample_tool_use("git commit -m msg", "toolu_dddd");
        // No tool_result -> orphan
        let sc = extract_session_commit(&tu, None, "/repo".to_owned())
            .expect("orphan should still extract");
        assert_eq!(sc.sha, None);
        assert_eq!(sc.recovery_source, RecoverySource::None);
    }

    #[test]
    fn extract_session_commit_returns_none_when_tool_name_is_not_bash() {
        let mut tu = sample_tool_use("git commit -m msg", "toolu_eeee");
        tu.tool_name = "Edit".to_owned();
        let tr = sample_tool_result("toolu_eeee", "[main abc1234]");
        assert!(extract_session_commit(&tu, Some(&tr), "/x".to_owned()).is_none());
    }

    #[test]
    fn extract_session_commit_returns_none_without_session_id() {
        let mut tu = sample_tool_use("git commit -m msg", "toolu_ffff");
        tu.session_id = None;
        let tr = sample_tool_result("toolu_ffff", "[main abc1234] subject");
        assert!(
            extract_session_commit(&tu, Some(&tr), "/x".to_owned()).is_none(),
            "session_id is NOT NULL on the schema; no session_id means we skip"
        );
    }

    #[test]
    fn extract_session_commit_detects_amend_and_truncation_flags() {
        let tu = sample_tool_use(
            "git commit --amend --no-edit 2>&1 | tail -5",
            "toolu_gggg",
        );
        let tr = sample_tool_result("toolu_gggg", "[main abc1234] subject");
        let sc = extract_session_commit(&tu, Some(&tr), "/x".to_owned()).unwrap();
        assert!(sc.is_amend);
        assert!(sc.output_head_truncated_by_command);
    }
}
