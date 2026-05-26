# Kickoff prompt — first thread after the post-v0.1.17 dev-path migration

**Context**: The maintainer migrated the dev tree off iCloud to a stable
local path between v0.1.17 (2026-05-24, commit `e5780ef`) and the first
new-path session. The prior Claude session's context doesn't travel —
auto-memory was keyed by the old encoded cwd
(`-Users-Robare-Library-Mobile-Documents-com-apple-CloudDocs-Dev-tokenscale`)
and is stranded. The new-path session needs to bootstrap from the docs
the prior thread wrote on the way out.

**Paste the block below into the first message of the new thread**, after
`cd`-ing into the new dev path. The handoff doc it references
(`docs/handoff-post-v0.1.17.md`) is the durable knowledge transfer; this
prompt is just the orientation pointer.

---

```
You're picking up tokenscale, a self-hostable Anthropic usage and
environmental-impact dashboard I maintain. I just migrated the dev
tree off iCloud to a stable local path; the previous Claude session
won't have its context here.

Start by reading docs/handoff-post-v0.1.17.md — it captures where
we are (v0.1.17 just shipped, Phase 1.5 ingest-expansion is closed),
the small follow-ups owed, the open queue for Phase 2 / Phase 3,
and the six project conventions the prior thread asked me to make
explicit so you'd inherit them rather than rediscover them. Then
skim CHANGELOG.md's v0.1.13–v0.1.17 entries for the behavioral arc.

Specifically: the §7 release-gate framing from
docs/roadmap-1.5-tool-use-ingest.md must carry forward verbatim into
Phase 2 and Phase 3 scoping. The historical pattern across the last
five schema-touching releases is that smoke against my real DB
surfaces a real bug the test suite didn't catch; that bug-find is
the release gate, not a failure mode. Don't tag until you find it.

Don't start Phase 2 implementation yet. The next workstream is its
scoping pass — empirical grounding first (sample tool_use rows for
git commit patterns), then design decisions, then sign-off.

Note for context migration: the auto-memory dir under
~/.claude/projects/ for the new path will be empty (the old one
was at -Users-Robare-Library-Mobile-Documents-com-apple-CloudDocs-Dev-tokenscale
and is stranded). The handoff doc + repo docs are the durable knowledge.
```

---

## Why this lives in the repo, not in chat

The auto-memory system (`~/.claude/projects/<encoded-cwd>/memory/`) is
keyed by the absolute working directory. A migration that changes the
working directory invalidates the encoded key; the new path's memory
dir starts empty. Anything written to the old memory dir is stranded.

The repo, by contrast, travels. `docs/handoff-post-v0.1.17.md` and this
file both ship with the cut-over. Any future thread that lands in the
new tree finds them via `ls docs/` or a `git log --oneline | head`.

## What the next thread needs to do during its first turn

1. Read `docs/handoff-post-v0.1.17.md`.
2. Skim `CHANGELOG.md`'s most recent five entries.
3. Acknowledge to the maintainer that the post-v0.1.17 in-flight items
   are noted (the FileHistorySnapshotPayload tighten, the
   trackedFileBackups frequency doc fix, issue #4, issue #1) and ready
   to be picked up when the maintainer is.
4. Stand by for the next workstream. The natural next move is Phase 2
   scoping per the handoff doc's "Open queue."
