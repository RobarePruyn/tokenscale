# tokenscale — Research Cadence

How the environmental-factor model stays current after v0.1.

The mechanics for evolving factors are already in place — `schema_version`, per-row `valid_from`, time-anchored resolution at query time, file-replacement-on-startup sync. This document is about the *process*: when to refresh, what triggers a sweep, who runs it, and how an update lands in users' dashboards.

---

## Default cadence: quarterly

A research sweep runs **every three months** to capture:

- New vendor sustainability disclosures (typically annual or quarterly cycles).
- New eGRID releases (annual, late September — historically the biggest single data update of the year).
- Newly-launched models that need factor rows.
- Methodology improvements from peer-reviewed work in the LLM-impact space.
- Tokenizer changes that affect per-task energy attribution (e.g. Opus 4.7's reported 1.0–1.35× token-count inflation versus 4.6).

The first quarterly sweep after a release lands as a minor version bump (e.g. `v0.2.0`). Sweeps that produce no material change land as a `request-for-research.md` update noting "nothing material to update this cycle."

---

## Ad-hoc triggers (don't wait for the quarter)

A sweep happens **immediately**, not on the quarterly cycle, when:

| Trigger | Why |
|---|---|
| A major vendor publishes a first-time sustainability disclosure | Largest possible factor-quality improvement; defer for months would leave users with stale data when better data exists. |
| EPA publishes new eGRID year | All US grid factors get refreshed; downstream CO₂e numbers shift across the board. |
| A frontier model launches (Opus, Sonnet, Haiku N+1; GPT-N; Gemini N) | Users need a row to attribute impact against. Until it's added, the model surfaces in `modelsWithoutFactors`. |
| A peer-reviewed paper substantially changes the methodology | E.g., if a Ren-et-al-class paper publishes a new water methodology that materially changes our numbers, we should adopt rather than wait. |
| The maintainer notices a factor that's clearly wrong | Errors-in-the-file fix-forward, not next-quarter. |

---

## What a sweep produces

Each sweep generates:

1. **A new entry in `docs/research-log.md`** capturing:
   - What was investigated (specific question or trigger).
   - Sources consulted (with URLs and access dates).
   - What was found vs. the previous version.
   - What changed in `environmental-factors.toml` (rows added, values revised, `valid_from` dates set).
2. **A bump to `file_version`** in `environmental-factors.toml`. `v0.1` → `v0.2` for material updates; `v0.1.1` for typo / clarification fixes.
3. **New rows or updated rows** in the factor file. New rows for new models. Updated rows get a new `valid_from` date so the time-anchored resolution picks them up for events on or after that date — older events continue to resolve against the previous row.
4. **An updated `docs/sources.md`** when new bibliography entries land (e.g. a new vendor disclosure URL gets cited).
5. **An updated `docs/request-for-research.md`** marking which open questions are now answered, and adding new ones surfaced during the sweep.

---

## Review and merge

The factor file is the maintainer's canonical artifact — changes go through a PR for visibility, but the maintainer has final say. Review checklist:

- [ ] Every new numeric value cites a source (URL or `docs/sources.md` anchor).
- [ ] Estimated values (vs anchored) are marked with `# ESTIMATE:` and show the derivation.
- [ ] `valid_from` dates make sense (model release dates for new rows; today's date for new estimates of existing models when superseding an older row).
- [ ] `uncertainty_range_pct` reflects the quality of the source — anchors ±30%, derivations ±35–60%, deep extrapolations 60%+.
- [ ] `confidence` tag set: `primary` for vendor disclosure or peer-reviewed; `secondary` for derivation, benchmark, or blog; `superseded` for replaced rows.
- [ ] `docs/research-log.md` entry written.
- [ ] `docs/sources.md` updated if new bibliography landed.
- [ ] `docs/request-for-research.md` updated.

---

## Distribution to users

Users get factor updates one of three ways:

| Path | Mechanism | When it applies |
|---|---|---|
| **Bundled in a new release** | Updated `environmental-factors.toml` is `include_str!`'d into the binary at compile time. Users get it when they upgrade (`brew upgrade tokenscale-cli` or equivalent). | Default — works for everyone, requires nothing from the user. |
| **Local override** | User sets `[factors].file = "/path/to/their/own.toml"` and edits the file directly. The application loads from that path instead of the embedded copy. | "Local research mode" per the [CHARTER](../CHARTER.md) — power-users who maintain their own factor model. |
| **`tokenscale factors update` (Phase 3)** | Pulls the latest maintainer-blessed factor file from an upstream repo without requiring a full binary upgrade. | Reserved for future. Lets users get factor updates between binary releases. |

For v1, every sweep ships as a new binary release. The maintainer cuts a version bump (`v0.1.x → v0.2.0`), the existing release pipeline produces fresh installers and the Homebrew formula, and users on auto-update channels get it on next upgrade.

---

## What the database does when factors change

The dashboard's day-to-day behavior across factor updates:

1. **At server startup**, `tokenscale serve` reads the (possibly-new) factor file and runs `sync_environmental_factors` against the DB. This **replaces** the contents of `env_factors` and `grid_factors` with what the file says. The current sync is full-replacement; v0.2 will switch to history-preserving upsert that adds new `valid_from` rows without dropping older ones.
2. **On every request**, the daily endpoint resolves each event's `(provider, model)` against the latest `env_factors` row whose `valid_from <= event.occurred_at`. So historical events get the factor row that was authoritative *for their date*, not whatever the latest file says. This is the whole reason we use time-anchored resolution: factor refinements propagate forward without rewriting history.
3. **No data migration is required** on a factor update. Users just upgrade and the dashboard reflects the new numbers immediately for events going forward — and retroactively for any time periods where the new file's `valid_from` extends backward.

---

## When the maintainer is not the one doing the research

The CHARTER's longer-term model is that the **Cowork research agent** (a separate system) generates factor-file revisions and the maintainer reviews/merges them. That's a Phase 3+ workflow not yet built. Until it lands, the maintainer runs research sweeps manually using whichever tools they prefer — but the *output shape* (research-log entry, file_version bump, sources update) stays the same regardless of how the sweep got produced.

If you're not the maintainer and you want to propose a factor update: open a PR with the changes plus a `docs/research-log.md` entry. The maintainer is the reviewer.

---

## Why this is documented now, not later

The factor data carries the project's credibility. Anyone looking at the environmental view should be able to trace any number back to a source and a date. Without a documented cadence, that audit trail decays — values land, no one remembers where they came from, and the project drifts toward "trust me." With the cadence written down, the workflow stays disciplined even when the maintainer changes or the project goes a quarter without attention.
