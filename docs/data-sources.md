# tokenscale — Data Sources

> **Status:** scaffold. Fleshed out per ingest path as it lands.
>
> See [`sources.md`](sources.md) for the bibliography of the **factor model**. This document is about the **ingest surfaces** — the data tokenscale reads to produce events.

## Source registry

`tokenscale` models each ingest source as a row in the `sources` table, joined to events through `events.source REFERENCES sources(kind)`. The seed rows are:

| `kind`         | `provider` | `display_name`                | Phase | Status         |
| -------------- | ---------- | ----------------------------- | ----- | -------------- |
| `claude_code`  | anthropic  | Claude Code (local JSONL)     | 1     | In progress    |
| `admin_api`    | anthropic  | Anthropic Admin API           | 2     | Not yet wired  |

v2 will add OpenAI, Bedrock cross-vendor, etc., as new rows in `sources` and new crates under `crates/`. No schema migration is needed to add a provider.

---

## 1. Claude Code JSONL session logs

**Path.** `~/.claude/projects/<project-id>/<session-id>.jsonl`. Each line is a JSON object representing one turn (or one message within a turn — schema confirmation pending; see "Open questions" below).

**Crate.** [`tokenscale-ingest-cc`](../crates/tokenscale-ingest-cc).

**Idempotency model.**

1. Track each file's last-seen mtime in the database. On re-scan, files unchanged since the last run are stat()'d and skipped without parsing.
2. If a JSONL line carries a `request_id`, dedupe on `(source = 'claude_code', request_id)`.
3. If a line is missing a `request_id` (older Claude Code revisions), compute a SHA-256 over a deterministic projection of the row — `occurred_at || model || token counts || session_id || project_id` — and dedupe on `(source, content_hash)`.

**Schema-drift tolerance.** Unknown JSON fields are ignored. Missing optional fields default sensibly. Missing required fields skip the line, log a structured warning, and do not crash the scan.

**What we extract per line:**

- `occurred_at` — turn timestamp.
- `model` — the model identifier as written by Claude Code.
- Token counts: `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_5m_tokens`, `cache_write_1h_tokens`. Missing counts default to 0.
- `session_id`, `project_id` — recovered from the file path if not present in the JSON.
- `request_id` — if present.
- The full raw JSON line — stored in `events.raw` unless `ingest.store_raw = false`.

**Open questions** _(answered before the parser is finalized; check a real session file)_:

- Confirm the exact JSON keys for token counts in current Claude Code releases.
- Confirm whether one JSONL line corresponds to one assistant turn, one message exchange, or something else.
- Confirm whether `request_id` is consistently present in current Claude Code releases or whether it's introduced at a known version.

These get resolved by inspecting a real `~/.claude/projects/.../*.jsonl` file before the parser is locked in. The result feeds back into this document.

---

## 2. Anthropic Admin API _(Phase 2)_

**Endpoints.**

- `GET /v1/organizations/usage_report/messages` — token-level usage with `request_id`, `workspace_id`, `api_key_id`, `model`.
- `GET /v1/organizations/cost_report` — invoice-line-item-level cost.

**Auth.** Admin API key (distinct from a regular API key — this is a separate credential issued from the Anthropic Console).

**Reference.** <https://docs.anthropic.com/en/api/admin-api/usage-cost/get-messages-usage-report>

**Crate.** [`tokenscale-ingest-api`](../crates/tokenscale-ingest-api) — placeholder in Phase 1.

---

## What is **not** ingested

- Any data Anthropic does not publish. There is no public per-request region attribution, no per-request energy or water metric, no per-request datacenter assignment. Where `tokenscale` shows region- or impact-derived metrics, the underlying assumption is documented and user-configurable.
- Logs from other LLM providers in v1.
- File contents Claude Code touched on disk during a session — `tokenscale` ingests only the JSONL session logs, not the artefacts those sessions produced.

### Structurally invisible consumer surfaces

The following are **inaccessible by design** to any external tool, including a future version of `tokenscale`:

- **Claude iOS app** — chat data lives in the app's iOS sandbox container; not readable from outside the app without jailbreak. iOS encrypted backups also do not expose it readably.
- **Claude Android app** — same reasoning as iOS (per-app sandboxing).
- **Claude desktop apps (Mac / Windows)** — primarily thin clients around `claude.ai`; conversation data is server-side and not exposed to local readers.
- **`claude.ai` web** — session data lives server-side at Anthropic; the only access path is the authenticated browser session itself. Scraping is rejected on ToS / account-safety / brittleness grounds (see `decisions.md`).

These surfaces ALL contribute to a Pro / Max / Team subscriber's billed usage. **Their COST is fully captured** via the billing-CSV import path (`source = "stripe_csv"`, future `"anthropic_admin"`) — the subscription line items in `billing_charges` cover everything Anthropic charges for. What's missing is **per-conversation, per-token granularity** for those surfaces.

**This is a structural limitation of Anthropic's product surface, not a `tokenscale` gap to close.** Anthropic does not currently expose any consumer-side usage feed (web, iOS, Android, desktop app); ChatGPT, Gemini, and Mistral are the same. The dashboard surfaces this honestly in a footnote rather than fabricating data we don't have.

If Anthropic ever ships a per-user consumer usage API (analogous to the Admin API but for consumer products), it would slot into `tokenscale` as a new source — same pattern as adding any other ingest path: row in `sources`, new crate under `crates/`, ingester emits `Event` rows that flow through the existing pipeline.
