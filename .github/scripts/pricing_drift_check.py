#!/usr/bin/env python3
"""
pricing_drift_check — detect when tokenscale's pricing.toml has drifted
away from Anthropic's published rate card.

Runs nightly under .github/workflows/pricing-drift-check.yml. See
docs/cost-methodology.md "Drift detector" for the user-facing
documentation.

Exit codes — 0 success plus three failure modes (the failure modes
are split deliberately so the wrapping workflow can react differently
to drift vs. detector-degradation):
  0 — pricing.toml matches Anthropic's published rates AND internal
      consistency checks pass. No action needed.
  1 — real drift detected. Workflow fails (red X on main). The wrapping
      workflow also opens a GH Issue with the diff inline.
  2 — parse failure. The page format may have changed. Workflow warns
      loudly but does NOT fail — false-positive risk too high. Detector
      needs an update; opens a "detector-degraded" Issue if persistent.
  3 — network failure. Workflow logs and exits non-zero-non-1 so the
      retry-tomorrow path is obvious.

Design constraints (all locked in v0.1.12 sign-off, do not change without
re-asking):
  * Source URL pinned to https://platform.claude.com/docs/en/about-claude/pricing.
    NOT claude.com/pricing (marketing). NOT models/overview (informational).
  * Parser pinned to the `## Model pricing` table specifically. Fast Mode
    ($30/$150), Batch (50% off), and US Data Residency (1.1x) live in
    separate `### …` subsections under different `##` headings and must
    not contaminate the result.
  * Cache rates: pricing.toml stores cache_read as absolute USD/MTok and
    cache_write_{5m,1h} as multipliers. Detector checks both the absolute
    cache_read value AND that input × multiplier matches Anthropic's
    absolute cache-write columns.
  * Live fetch primary; snapshot at pricing-rate-card.snapshot.json is
    the offline-fallback. Snapshot > 90d emits a "snapshot is stale"
    warning regardless of fetch success.
  * Scope: the four models actually tracked in pricing.toml. New-model
    alerting is out-of-scope for V1.
"""

from __future__ import annotations

import datetime as _dt
import json
import re
import sys
import tomllib  # 3.11+
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

# ---------------------------------------------------------------------------
# Configuration constants — change these only with the design-decision
# review described in docs/cost-methodology.md "Drift detector".
# ---------------------------------------------------------------------------

PRICING_PAGE_URL = "https://platform.claude.com/docs/en/about-claude/pricing"
USER_AGENT = "tokenscale-pricing-drift-check/0.1 (+https://github.com/RobarePruyn/tokenscale)"
FETCH_TIMEOUT_SECS = 30
SNAPSHOT_STALE_AFTER_DAYS = 90

# Tracked models — only the rows actually in pricing.toml. Adding a row
# means: (1) update pricing.toml with verified rates, (2) re-capture
# pricing-rate-card.snapshot.json, (3) add the model ID here.
TRACKED_MODELS = {
    "claude-opus-4-8": "Claude Opus 4.8",
    "claude-opus-4-7": "Claude Opus 4.7",
    "claude-opus-4-6": "Claude Opus 4.6",
    "claude-sonnet-4-6": "Claude Sonnet 4.6",
    "claude-haiku-4-5": "Claude Haiku 4.5",
    # Fable 5 is retired (pulled 2026-06-12 by a US export-control
    # directive, ~3 days after launch). It is still on the pricing page
    # today, so we verify its rates while they last. Once Anthropic
    # delists it, `status = "retired"` in pricing.toml tells the detector
    # to skip it rather than raise ParseFailure. See load_retired_model_ids
    # and the retired-skip guard in parse_anthropic_page.
    "claude-fable-5": "Claude Fable 5",
}

# Anthropic's published cache multipliers (per ### Prompt caching prose).
# The detector validates pricing.toml's stored multipliers match these AND
# that the per-row absolute cache columns equal input × multiplier.
EXPECTED_CACHE_MULTIPLIERS = {
    "cache_read": 0.1,
    "cache_write_5m": 1.25,
    "cache_write_1h": 2.0,
}

# Repo paths.
REPO_ROOT = Path(__file__).resolve().parent.parent.parent
PRICING_TOML = REPO_ROOT / "pricing.toml"
SNAPSHOT_JSON = REPO_ROOT / "pricing-rate-card.snapshot.json"


# ---------------------------------------------------------------------------
# Parsed-page model.
# ---------------------------------------------------------------------------


@dataclass
class ModelRates:
    """Absolute rates per million tokens, parsed from the `## Model pricing`
    table. All fields are USD/MTok."""
    display_name: str
    base_input: float
    output: float
    cache_read: float
    cache_write_5m: float
    cache_write_1h: float


def _normalize(text: str) -> str:
    """Strip HTML tags, decode entities, drop markdown table delimiters,
    collapse whitespace. Lets the parser run uniformly against either
    the markdown fixture (test path) or the rendered HTML the live
    page actually serves (Mintlify / Next.js — no `<h2>` semantics,
    everything's `<div>` wrappers).

    After normalization both forms reduce to plain text like:

        Model pricing The following table shows ... Claude Opus 4.7 $5 / MTok $6.25 ...
    """
    # Strip HTML tags.
    stripped = re.sub(r"<[^>]+>", " ", text)
    # Decode the small set of HTML entities that actually appear in
    # Anthropic's pricing copy. `html.unescape` from stdlib would be
    # more general but adds an import for two characters.
    stripped = stripped.replace("&amp;", "&").replace("&nbsp;", " ").replace("&#x27;", "'")
    # Drop markdown table-cell delimiters so the same regex works on
    # both fixture and live formats.
    stripped = stripped.replace("|", " ")
    # Collapse whitespace.
    return re.sub(r"\s+", " ", stripped).strip()


def parse_anthropic_page(
    markdown_or_html: str, retired_ids: "frozenset[str]" = frozenset()
) -> dict[str, ModelRates]:
    """Parse the page content and extract base-rate rows from the
    Model pricing section only.

    Pinning strategy:
      1. Normalize input through `_normalize` so markdown fixtures and
         live HTML reduce to the same plain-text shape.
      2. Slice the content between the literal phrase "Model pricing"
         (the section heading text) and "Cloud platform pricing" (the
         next major heading). Fast Mode, Batch, and Data Residency
         live AFTER "Cloud platform pricing" so they fall outside the
         slice and cannot contaminate the result.
      3. Within the slice, confirm the expected column header tokens
         ("Base Input Tokens" AND "Output Tokens") are present. Reject
         otherwise — guards against Anthropic restructuring columns.
      4. For each tracked model, find the row by display-name match
         followed by exactly five `$X / MTok` cells. Order is
         input → 5m write → 1h write → cache read → output, matching
         Anthropic's column order.

    Returns a dict keyed by Claude API model ID (e.g. `claude-opus-4-7`).
    Raises `ParseFailure` when any of the pinning steps fail — the
    detector exits with code 2 (warn, don't fail the build) so a page
    restructure doesn't generate a red X.
    """
    normalized = _normalize(markdown_or_html)

    # Step 2: find the Model pricing section.
    start_marker = "Model pricing"
    end_markers = [
        "Cloud platform pricing",
        "Feature-specific pricing",  # backup if Cloud platform section drops
    ]
    start_idx = normalized.find(start_marker)
    if start_idx < 0:
        raise ParseFailure("`Model pricing` heading not found in page")
    end_idx = len(normalized)
    for marker in end_markers:
        candidate = normalized.find(marker, start_idx + len(start_marker))
        if candidate >= 0:
            end_idx = min(end_idx, candidate)
    section = normalized[start_idx:end_idx]

    # Step 3: confirm column headers.
    if "Base Input Tokens" not in section or "Output Tokens" not in section:
        raise ParseFailure(
            "`Model pricing` table is missing expected column headers "
            "(`Base Input Tokens` and/or `Output Tokens`). "
            "Anthropic may have restructured the page; update the parser."
        )

    # Step 4: extract rows. After normalization, a row reads:
    #   Claude Opus 4.7 $5 / MTok $6.25 / MTok $10 / MTok $0.50 / MTok $25 / MTok
    # Up to a "(deprecated)" / "(retired ...)" suffix between the name and
    # the dollar cells, which we consume non-greedily.
    rates: dict[str, ModelRates] = {}
    for model_id, display_name in TRACKED_MODELS.items():
        pattern = (
            rf"{re.escape(display_name)}\b\s*(?:\(.*?\)\s*)?"
            r"\$([\d.]+)\s*/\s*MTok\s*"
            r"\$([\d.]+)\s*/\s*MTok\s*"
            r"\$([\d.]+)\s*/\s*MTok\s*"
            r"\$([\d.]+)\s*/\s*MTok\s*"
            r"\$([\d.]+)\s*/\s*MTok"
        )
        m = re.search(pattern, section)
        if not m:
            if model_id in retired_ids:
                # Retired model (e.g. a pulled model marked status="retired"
                # in pricing.toml) is no longer on the page. Expected, not
                # drift: log and skip rather than failing the build. This is
                # the D5 guard — keeps a delisting from crying wolf (the
                # v0.1.14 "cry-wolf kills trust" lesson).
                print(
                    f"::notice::{display_name} ({model_id}) is marked retired and is "
                    "no longer on the pricing page; skipping (expected, not drift).",
                    file=sys.stderr,
                )
                continue
            raise ParseFailure(
                f"could not find row for {display_name!r} in Model pricing table. "
                "Either the model was removed upstream or the parser regex needs to update."
            )
        rates[model_id] = ModelRates(
            display_name=display_name,
            base_input=float(m.group(1)),
            cache_write_5m=float(m.group(2)),
            cache_write_1h=float(m.group(3)),
            cache_read=float(m.group(4)),
            output=float(m.group(5)),
        )

    return rates


def parse_cache_multipliers(markdown_or_html: str) -> dict[str, float]:
    """Extract the cache multipliers from the Prompt caching prose
    table. Anthropic's wording (after normalization) is:

        5-minute cache write 1.25x base input price Cache valid for 5 minutes
        1-hour cache write   2x   base input price  Cache valid for 1 hour
        Cache read (hit)     0.1x base input price  Same duration ...

    Returns {"cache_read": 0.1, "cache_write_5m": 1.25, "cache_write_1h": 2.0}.
    Raises ParseFailure on missing values.
    """
    normalized = _normalize(markdown_or_html)
    patterns = {
        "cache_write_5m": r"5-minute cache write\s*([\d.]+)x",
        "cache_write_1h": r"1-hour cache write\s*([\d.]+)x",
        "cache_read": r"Cache read\s*\(hit\)\s*([\d.]+)x",
    }
    result: dict[str, float] = {}
    for key, pat in patterns.items():
        m = re.search(pat, normalized)
        if not m:
            raise ParseFailure(
                f"could not parse Prompt caching multiplier for {key!r}"
            )
        result[key] = float(m.group(1))
    return result


# ---------------------------------------------------------------------------
# Comparison checks.
# ---------------------------------------------------------------------------


class ParseFailure(Exception):
    pass


class NetworkFailure(Exception):
    pass


def fetch_live_page() -> str:
    """Fetch the live pricing page. Raises NetworkFailure on transport
    errors. Parse-failures bubble up separately as ParseFailure from the
    callers."""
    req = urllib.request.Request(
        PRICING_PAGE_URL,
        headers={"User-Agent": USER_AGENT, "Accept": "text/html,*/*"},
    )
    try:
        with urllib.request.urlopen(req, timeout=FETCH_TIMEOUT_SECS) as resp:
            return resp.read().decode("utf-8", errors="replace")
    except urllib.error.URLError as e:
        raise NetworkFailure(f"could not fetch {PRICING_PAGE_URL}: {e}") from e
    except TimeoutError as e:
        raise NetworkFailure(f"timed out fetching {PRICING_PAGE_URL}: {e}") from e


def load_pricing_toml() -> dict[str, dict[str, float]]:
    """Load pricing.toml's tracked rows into a flat {model_id: {field: value}}
    dict. Only the fields the detector compares against.

    Schema-compat note: v0.1.12 stored each model as a single TOML table
    (`[providers.anthropic.models."claude-opus-4-7"]`) so the loader saw
    a dict per model. v0.1.13 introduced multi-row format
    (`[[providers.anthropic.models."claude-opus-4-7"]]`) and tomllib
    parses that as a list of dicts. Anthropic only publishes one rate
    per model, so the detector compares against the row with the latest
    `valid_from` — the rate that's currently live. Older rows in
    pricing.toml are historical and intentionally don't match the live
    page.
    """
    with PRICING_TOML.open("rb") as f:
        data = tomllib.load(f)
    out: dict[str, dict[str, float]] = {}
    for provider_id, provider in data.get("providers", {}).items():
        for model_id, model in provider.get("models", {}).items():
            if model_id not in TRACKED_MODELS:
                continue
            # Normalize single-table-form (dict) and multi-row-form (list)
            # into the same shape, then pick the row with the latest
            # valid_from. `valid_from` is ISO YYYY-MM-DD so lexical max
            # is correct.
            if isinstance(model, list):
                latest = max(model, key=lambda row: row.get("valid_from", ""))
            else:
                latest = model
            out[model_id] = {
                "input": float(latest["input_usd_per_mtok"]),
                "output": float(latest["output_usd_per_mtok"]),
                "cache_read": float(latest["cache_read_usd_per_mtok"]),
                "cache_write_5m_multiplier": float(latest["cache_write_5m_multiplier"]),
                "cache_write_1h_multiplier": float(latest["cache_write_1h_multiplier"]),
            }
    return out


def load_retired_model_ids() -> "frozenset[str]":
    """Model IDs whose latest pricing.toml row carries `status = "retired"`.

    A retired model (a pulled model like Fable 5) may vanish from the live
    pricing page at any time. The detector treats its absence as expected
    rather than as a parse failure — see the guard in parse_anthropic_page.
    Reads the same `status` field the Rust parser ignores (TOML-only
    provenance, like launch_date_source).
    """
    with PRICING_TOML.open("rb") as f:
        data = tomllib.load(f)
    retired: set[str] = set()
    for provider in data.get("providers", {}).values():
        for model_id, model in provider.get("models", {}).items():
            rows = model if isinstance(model, list) else [model]
            latest = max(rows, key=lambda row: row.get("valid_from", ""))
            if latest.get("status") == "retired":
                retired.add(model_id)
    return frozenset(retired)


def load_snapshot() -> dict:
    with SNAPSHOT_JSON.open("r") as f:
        return json.load(f)


def check_drift(
    upstream: dict[str, ModelRates],
    upstream_multipliers: dict[str, float],
    local: dict[str, dict[str, float]],
) -> list[str]:
    """Return a list of drift descriptions. Empty list = no drift."""
    drift: list[str] = []

    # Per-model base-rate comparison.
    for model_id, up in upstream.items():
        if model_id not in local:
            drift.append(
                f"{model_id}: present on Anthropic page but missing from pricing.toml"
            )
            continue
        lo = local[model_id]
        if not _close(up.base_input, lo["input"]):
            drift.append(
                f"{model_id} input: pricing.toml=${lo['input']}, Anthropic=${up.base_input}"
            )
        if not _close(up.output, lo["output"]):
            drift.append(
                f"{model_id} output: pricing.toml=${lo['output']}, Anthropic=${up.output}"
            )
        # cache_read is absolute in pricing.toml — compare directly.
        if not _close(up.cache_read, lo["cache_read"]):
            drift.append(
                f"{model_id} cache_read: pricing.toml=${lo['cache_read']}, "
                f"Anthropic=${up.cache_read}"
            )
        # cache writes are multipliers in pricing.toml; compute the implied
        # absolute and compare against Anthropic's absolute column.
        implied_5m = lo["input"] * lo["cache_write_5m_multiplier"]
        if not _close(up.cache_write_5m, implied_5m):
            drift.append(
                f"{model_id} cache_write_5m: pricing.toml implies ${implied_5m:.2f} "
                f"(input ${lo['input']} × {lo['cache_write_5m_multiplier']}), "
                f"Anthropic publishes ${up.cache_write_5m}"
            )
        implied_1h = lo["input"] * lo["cache_write_1h_multiplier"]
        if not _close(up.cache_write_1h, implied_1h):
            drift.append(
                f"{model_id} cache_write_1h: pricing.toml implies ${implied_1h:.2f} "
                f"(input ${lo['input']} × {lo['cache_write_1h_multiplier']}), "
                f"Anthropic publishes ${up.cache_write_1h}"
            )

        # Internal consistency: cache_read should be 10% of input.
        if not _close(lo["cache_read"], lo["input"] * 0.1):
            drift.append(
                f"{model_id} INTERNAL: cache_read ${lo['cache_read']} ≠ "
                f"input × 0.1 = ${lo['input'] * 0.1:.2f}. "
                "pricing.toml is internally inconsistent."
            )

    # Cache-multiplier prose comparison.
    for key, expected in EXPECTED_CACHE_MULTIPLIERS.items():
        upstream_val = upstream_multipliers.get(key)
        if upstream_val is None:
            drift.append(f"cache multiplier {key} missing from Anthropic page")
            continue
        if not _close(upstream_val, expected):
            drift.append(
                f"cache multiplier {key}: detector expects {expected}x, "
                f"Anthropic publishes {upstream_val}x — convention may have changed"
            )

    return drift


def check_snapshot_age() -> str | None:
    """Returns a warning string if the snapshot fixture is > 90 days old,
    else None. Doesn't fail the check — fall-open is the right posture
    when the live fetch is the primary signal."""
    snapshot = load_snapshot()
    captured = _dt.date.fromisoformat(snapshot["captured_at"])
    age = (_dt.date.today() - captured).days
    if age > SNAPSHOT_STALE_AFTER_DAYS:
        return (
            f"snapshot fixture pricing-rate-card.snapshot.json is {age} days old "
            f"(> {SNAPSHOT_STALE_AFTER_DAYS}). Re-capture from {PRICING_PAGE_URL}."
        )
    return None


def _close(a: float, b: float, tol: float = 0.001) -> bool:
    return abs(a - b) < tol


# ---------------------------------------------------------------------------
# Main entry.
# ---------------------------------------------------------------------------


def main() -> int:
    snapshot_warning = check_snapshot_age()
    if snapshot_warning:
        print(f"::warning::{snapshot_warning}", file=sys.stderr)

    try:
        page = fetch_live_page()
    except NetworkFailure as e:
        print(f"::warning::Network failure: {e}", file=sys.stderr)
        print(
            "::warning::Falling back to snapshot for awareness only; "
            "real drift cannot be detected without a live fetch.",
            file=sys.stderr,
        )
        return 3

    try:
        upstream_rates = parse_anthropic_page(page, load_retired_model_ids())
        upstream_multipliers = parse_cache_multipliers(page)
    except ParseFailure as e:
        print(f"::warning::Parse failure: {e}", file=sys.stderr)
        print(
            "::warning::Anthropic's pricing page format may have changed. "
            "Detector needs an update; not failing the build to avoid false-positive drift alerts.",
            file=sys.stderr,
        )
        return 2

    local = load_pricing_toml()
    drift = check_drift(upstream_rates, upstream_multipliers, local)

    if not drift:
        print("✓ pricing.toml matches Anthropic's published rates for all tracked models.")
        for model_id, up in sorted(upstream_rates.items()):
            print(
                f"  {model_id}: ${up.base_input}/MTok input, ${up.output}/MTok output, "
                f"${up.cache_read}/MTok cache_read"
            )
        return 0

    print("::error::Pricing drift detected — pricing.toml is stale or inconsistent.")
    print()
    for line in drift:
        print(f"  · {line}")
    print()
    print(
        "Action: re-verify against the source URL, update pricing.toml, recapture "
        "pricing-rate-card.snapshot.json, and tag a new release. See "
        "docs/cost-methodology.md → Drift detector for the runbook."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
