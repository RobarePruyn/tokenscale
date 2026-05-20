"""End-to-end exit-code coverage for pricing_drift_check.main().

Builds on test_pricing_drift_check.py (which tests individual helper
functions) by driving `main()` through each of its three non-clean
exit paths with monkeypatched fetch / load helpers, plus structural
assertions on the workflow YAML's issue-creation step that runs only
on exit 1.

Why this file is separate from test_pricing_drift_check.py:
  The function-level suite proves the parser pins to the right table
  and check_drift() produces the right output. This suite proves the
  alarm actually rings — every non-clean exit path is verified to
  (a) return the documented exit code, (b) print the expected
  diagnostic, and (c) NOT print anything that would trigger the
  exit-1 issue-creation path. The two failure modes (parse failure
  vs. real drift) must stay disjoint at runtime.

Negative-path fixtures used here:
  Fixtures under `tests/fixtures/negative_path/` are deliberately
  wrong and named to make that role unambiguous (e.g.
  `parse_failure_restructured.md`). The live detector path
  (`fetch_live_page` over the real Anthropic URL + `load_snapshot`
  from `pricing-rate-card.snapshot.json` at the repo root) cannot
  resolve to them — the tests monkeypatch `fetch_live_page` and
  `load_pricing_toml` directly. No production code path reads from
  `negative_path/`.

How the GitHub-Issue path is tested:
  The issue is created in a bash heredoc inside the workflow YAML.
  Two layers of coverage:
    * Structural (in this file): parse the workflow YAML as text and
      assert the issue-creation step is gated on `rc == '1'`, calls
      `gh issue create --label pricing-divergence`, interpolates
      `$(cat drift-output.txt)` into the body, and includes the
      runbook elements documented in cost-methodology.md.
    * End-to-end (one-shot, manual, captured separately from CI):
      `workflow_dispatch` against a throwaway branch with a wrong
      `pricing.toml`, observe an actual `pricing-divergence` issue
      open, capture the body, clean up. See the v0.1.14 CHANGELOG +
      this commit's PR description for the captured body.

  Limitation the structural test cannot catch: a heredoc with wrong
  bash quoting that YAML parses fine but bash interpolates
  differently. The one-shot end-to-end firing closes that gap once;
  ongoing regression armor lives in the structural test.
"""

from __future__ import annotations

import contextlib
import io
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

# Make the detector script importable.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import pricing_drift_check as detector  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent.parent.parent
FIXTURES_DIR = Path(__file__).parent / "fixtures"
VALID_FIXTURE = FIXTURES_DIR / "pricing-page-sample.md"
PARSE_FAILURE_FIXTURE = FIXTURES_DIR / "negative_path" / "parse_failure_restructured.md"
WORKFLOW_YAML = REPO_ROOT / ".github" / "workflows" / "pricing-drift-check.yml"


def _toml_in_sync() -> dict[str, dict[str, float]]:
    """A `pricing.toml`-shape dict that matches the valid fixture's
    rates. Mutate one entry to simulate drift."""
    return {
        "claude-opus-4-7": {
            "input": 5.00, "output": 25.00, "cache_read": 0.50,
            "cache_write_5m_multiplier": 1.25, "cache_write_1h_multiplier": 2.0,
        },
        "claude-opus-4-6": {
            "input": 5.00, "output": 25.00, "cache_read": 0.50,
            "cache_write_5m_multiplier": 1.25, "cache_write_1h_multiplier": 2.0,
        },
        "claude-sonnet-4-6": {
            "input": 3.00, "output": 15.00, "cache_read": 0.30,
            "cache_write_5m_multiplier": 1.25, "cache_write_1h_multiplier": 2.0,
        },
        "claude-haiku-4-5": {
            "input": 1.00, "output": 5.00, "cache_read": 0.10,
            "cache_write_5m_multiplier": 1.25, "cache_write_1h_multiplier": 2.0,
        },
    }


@contextlib.contextmanager
def _capture_main_io():
    """Capture stdout + stderr around a `detector.main()` call. Yields
    a (stdout_buf, stderr_buf) pair; both are io.StringIO."""
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        yield out, err


class MainReturnsOneOnRealDrift(unittest.TestCase):
    """Exit code 1 — real Anthropic price change. The core case the
    detector exists for. Drives main() with the valid fixture (so the
    parser succeeds) but a pricing.toml whose Opus 4.7 input has been
    silently bumped to $7. Asserts main() returns 1 and the divergence
    output identifies the specific model + field that diverged.
    """

    def test_input_rate_drift_returns_1_and_names_model_and_field(self):
        local = _toml_in_sync()
        local["claude-opus-4-7"]["input"] = 7.00
        # Internal-consistency requires cache_read = input * 0.1; bump
        # it alongside so the test isolates the upstream-vs-local input
        # drift (the case under test) rather than also tripping the
        # INTERNAL check (a different test below).
        local["claude-opus-4-7"]["cache_read"] = 0.70

        with patch.object(detector, "fetch_live_page", return_value=VALID_FIXTURE.read_text()), \
             patch.object(detector, "load_pricing_toml", return_value=local), \
             patch.object(detector, "check_snapshot_age", return_value=None):
            with _capture_main_io() as (out, err):
                rc = detector.main()

        self.assertEqual(rc, 1, f"expected exit 1, got {rc}; stderr={err.getvalue()}")
        stdout = out.getvalue()
        self.assertIn("Pricing drift detected", stdout)
        # The specific divergence line names the model AND the field
        # AND both numbers, so a reviewer reading the issue body knows
        # exactly what to change.
        self.assertIn("claude-opus-4-7 input", stdout)
        self.assertIn("pricing.toml=$7.0", stdout)
        self.assertIn("Anthropic=$5.0", stdout)


class MainReturnsOneOnInternalInconsistency(unittest.TestCase):
    """Exit code 1 — pricing.toml is internally inconsistent regardless
    of what Anthropic publishes. Specifically: `cache_read` should equal
    `input × 0.1`, and any row that drifts off that ratio gets flagged.
    This is the seed-error class: someone updated `input` but forgot to
    update the absolute `cache_read`.
    """

    def test_cache_read_not_equal_input_times_0_1_returns_1_and_flags_INTERNAL(self):
        local = _toml_in_sync()
        # Input correct, cache_read wrong. Should trip the INTERNAL check.
        local["claude-opus-4-7"]["cache_read"] = 1.50  # should be 0.50

        with patch.object(detector, "fetch_live_page", return_value=VALID_FIXTURE.read_text()), \
             patch.object(detector, "load_pricing_toml", return_value=local), \
             patch.object(detector, "check_snapshot_age", return_value=None):
            with _capture_main_io() as (out, err):
                rc = detector.main()

        self.assertEqual(rc, 1, f"expected exit 1, got {rc}; stderr={err.getvalue()}")
        stdout = out.getvalue()
        self.assertIn("Pricing drift detected", stdout)
        # The INTERNAL marker is the parser-side signal that this is
        # the "internally inconsistent" failure mode, distinct from the
        # "drifted from Anthropic" failure mode.
        self.assertIn("INTERNAL", stdout)
        self.assertIn("claude-opus-4-7", stdout)


class MainReturnsTwoOnParseFailure(unittest.TestCase):
    """Exit code 2 — Anthropic page restructured. The HIGHEST-VALUE
    test in this suite: a false-positive drift alert is worse than a
    missed detection because once the alarm cries wolf, it stops being
    trusted. This test proves the parse-failure path is FULLY DISJOINT
    from the exit-1 issue path — different exit code, no drift-detected
    output, no marker strings the workflow's issue body would
    interpolate.
    """

    def test_restructured_page_returns_2_does_not_fire_drift_alert(self):
        broken_page = PARSE_FAILURE_FIXTURE.read_text()
        # Sanity-check the fixture really is broken — if someone fixes
        # the fixture by accident, the rest of the assertions become
        # meaningless and we'd be testing the wrong code path.
        self.assertNotIn(
            "Base Input Tokens", broken_page,
            "negative-path fixture must NOT contain the expected column header; "
            "if it does, the parser will succeed and this test exercises the wrong path",
        )

        with patch.object(detector, "fetch_live_page", return_value=broken_page), \
             patch.object(detector, "load_pricing_toml", return_value=_toml_in_sync()), \
             patch.object(detector, "check_snapshot_age", return_value=None):
            with _capture_main_io() as (out, err):
                rc = detector.main()

        # Exit-code disjointness: 2 is warn-only and the workflow's
        # issue step is gated on `rc == '1'`. Exit 2 must NEVER be 1.
        self.assertEqual(rc, 2, f"expected exit 2, got {rc}; stdout={out.getvalue()}")
        self.assertNotEqual(rc, 1, "parse failure must not leak into the exit-1 issue path")

        stdout = out.getvalue()
        stderr = err.getvalue()

        # Parse-failure warning lives on stderr (GitHub Actions
        # `::warning::` prefix). Confirm it's there.
        self.assertIn("::warning::Parse failure:", stderr)

        # Addition 1: prove the parse-failure run cannot leak into the
        # exit-1 issue path. The bash heredoc that builds the issue
        # body interpolates `$(cat drift-output.txt)`, which captures
        # stdout+stderr. Even if (counterfactually) the workflow ran
        # the issue step on rc=2, none of the strings the runbook
        # expects to see in a drift alert would be present.
        for drift_only_marker in [
            "Pricing drift detected",      # exit-1 stdout line
            "INTERNAL",                     # internal-inconsistency marker
            "pricing.toml=$",               # divergence detail format
            "Anthropic=$",                  # divergence detail format
            "Action: re-verify against the source",  # exit-1 trailing line
        ]:
            self.assertNotIn(
                drift_only_marker, stdout,
                f"parse-failure stdout must not emit drift-alert marker {drift_only_marker!r}",
            )
            self.assertNotIn(
                drift_only_marker, stderr,
                f"parse-failure stderr must not emit drift-alert marker {drift_only_marker!r}",
            )


class WorkflowIssueStepStructure(unittest.TestCase):
    """Structural assertions on the workflow YAML's issue-creation
    step. Read as raw text — PyYAML is not in the default 3.11 image
    and the substring assertions are sufficient to catch the
    regressions that matter (step deleted, label changed, runbook
    weakened, interpolation removed).

    Limitations: this layer cannot catch a heredoc that bash
    interpolates differently than text-search reads it. That gap is
    closed by the one-shot manual `workflow_dispatch` firing recorded
    in the v0.1.14 release notes.
    """

    def setUp(self) -> None:
        self.workflow = WORKFLOW_YAML.read_text()

    def test_issue_creation_step_is_gated_on_rc_eq_1(self):
        # The exact `if:` condition the issue step runs under.
        self.assertIn(
            "if: ${{ steps.drift.outputs.rc == '1' }}",
            self.workflow,
            "issue-creation step must be gated on exit code 1; "
            "any other gate would couple it to the parse-failure / network-failure paths",
        )

    def test_issue_step_calls_gh_with_pricing_divergence_label(self):
        self.assertIn("gh issue create", self.workflow)
        self.assertIn("--label pricing-divergence", self.workflow)
        # The existing-issue comment-append path uses the same label
        # for the dedup lookup; assert it's the same string.
        self.assertGreaterEqual(
            self.workflow.count("--label pricing-divergence"), 1,
            "the `pricing-divergence` label is the dedup key; if it changes "
            "or disappears, the workflow would either fail to dedup or "
            "fail to open new issues",
        )

    def test_issue_body_interpolates_drift_output_txt(self):
        # The detector writes its diagnostic to drift-output.txt; the
        # workflow embeds `$(cat drift-output.txt)` inside the issue
        # body. If that interpolation disappears, the issue opens but
        # has no detail — the alarm rings without a message.
        self.assertIn("$(cat drift-output.txt)", self.workflow)

    def test_issue_body_includes_documented_runbook_steps(self):
        # The runbook in the issue body must list these steps (the
        # ones documented in docs/cost-methodology.md → Drift
        # detector). If any disappears, the receiver of the issue
        # loses guidance on how to resolve it. Backticks are escaped
        # inside the bash heredoc (`\`pricing.toml\``) so the marker
        # substrings drop them to stay agnostic to that detail.
        for runbook_marker in [
            "Re-verify against the source",
            "Update",                              # "Update `pricing.toml`"
            "pricing.toml",                        # same line, name appears
            "Re-capture",                          # "Re-capture `pricing-rate-card.snapshot.json`"
            "pricing-rate-card.snapshot.json",
            "Corrections log",
            "Tag a new release",
        ]:
            self.assertIn(
                runbook_marker, self.workflow,
                f"runbook step missing from issue body: {runbook_marker!r}",
            )


class LoadPricingTomlSchemaCompat(unittest.TestCase):
    """End-to-end: `load_pricing_toml()` must successfully read the
    actual repo `pricing.toml` and return the expected shape. The other
    test classes monkeypatch this function, so a schema break (like
    v0.1.13's switch from single-table to multi-row format) wouldn't
    show up there. This test reads the live file.

    Caught a real bug on first manual `workflow_dispatch` firing: the
    v0.1.13 multi-row schema (`[[providers.…models."<id>"]]`) makes
    `models[<id>]` a list, not a dict. v0.1.12's loader did
    `model["input_usd_per_mtok"]` directly and crashed with a TypeError
    when v0.1.13 shipped. This test pins the loader to the schema in
    use by the repo's actual pricing.toml.
    """

    def test_loader_returns_dict_for_every_tracked_model(self):
        loaded = detector.load_pricing_toml()
        self.assertEqual(
            set(loaded.keys()), set(detector.TRACKED_MODELS.keys()),
            "load_pricing_toml must return one entry per TRACKED_MODELS key",
        )

    def test_loader_returns_floats_for_every_compared_field(self):
        loaded = detector.load_pricing_toml()
        for model_id, fields in loaded.items():
            for field in [
                "input", "output", "cache_read",
                "cache_write_5m_multiplier", "cache_write_1h_multiplier",
            ]:
                self.assertIn(field, fields, f"{model_id} missing field {field!r}")
                self.assertIsInstance(
                    fields[field], float,
                    f"{model_id}.{field} should be float, got {type(fields[field])}",
                )

    def test_loader_picks_latest_valid_from_when_multiple_rows(self):
        """For a multi-row entry, the detector must compare against the
        latest valid_from row (the rate currently live). Synthesize a
        two-row TOML in-memory and verify the loader picks the row with
        the later valid_from."""
        import tomllib
        toml_text = b"""
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[[providers.anthropic.models."claude-opus-4-7"]]
display_name = "Claude Opus 4.7"
valid_from = "2025-01-01"
input_usd_per_mtok = 99.00
output_usd_per_mtok = 999.00
cache_read_usd_per_mtok = 9.90
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.0
source_url = "x"
source_accessed_at = "2025-01-01"

[[providers.anthropic.models."claude-opus-4-7"]]
display_name = "Claude Opus 4.7"
valid_from = "2026-04-16"
input_usd_per_mtok = 5.00
output_usd_per_mtok = 25.00
cache_read_usd_per_mtok = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.0
source_url = "x"
source_accessed_at = "2026-05-18"
"""
        # Sanity-check tomllib parses the multi-row form to a list, so
        # this test exercises the schema the bug was in.
        parsed = tomllib.loads(toml_text.decode())
        self.assertIsInstance(
            parsed["providers"]["anthropic"]["models"]["claude-opus-4-7"], list,
            "multi-row [[…]] form must parse to a list",
        )

        # Patch the loader's file path to a temp file with our two-row
        # content. Cleanest mock: just override PRICING_TOML for the call.
        import tempfile
        with tempfile.NamedTemporaryFile(suffix=".toml", delete=False) as tmp:
            tmp.write(toml_text)
            tmp_path = Path(tmp.name)
        try:
            with patch.object(detector, "PRICING_TOML", tmp_path):
                loaded = detector.load_pricing_toml()
            self.assertEqual(loaded["claude-opus-4-7"]["input"], 5.00,
                "loader must pick the row with the latest valid_from (2026-04-16, $5), "
                "not the older 2025-01-01 row ($99)")
        finally:
            tmp_path.unlink()


class ExistingFixtureRetainsNegativeAssertions(unittest.TestCase):
    """Confirms that the v0.1.12 negative assertions still hold against
    the current `pricing-page-sample.md` fixture. If anyone trims the
    fixture down (e.g., deletes the Fast Mode or Batch sections to
    "shorten" it), this test fails — the parser pinning is only proven
    against false positives if those sections are present in the
    sample to be ignored.
    """

    def setUp(self) -> None:
        self.page = VALID_FIXTURE.read_text()
        self.rates = detector.parse_anthropic_page(self.page)

    def test_fixture_still_contains_fast_mode_section(self):
        self.assertIn("### Fast mode pricing", self.page)
        self.assertIn("$30 / MTok", self.page)
        self.assertIn("$150 / MTok", self.page)

    def test_fixture_still_contains_batch_section(self):
        self.assertIn("### Batch processing", self.page)
        # The Batch table's Opus row — the source of one of the
        # false-positive cases the parser must ignore.
        self.assertIn("$2.50 / MTok", self.page)
        self.assertIn("$12.50 / MTok", self.page)

    def test_fixture_still_contains_data_residency_section(self):
        self.assertIn("### Data residency pricing", self.page)
        self.assertIn("1.1x", self.page)

    def test_parser_still_pins_to_base_rates_despite_fast_mode(self):
        # Re-runs the v0.1.12 negative assertion: even with Fast Mode
        # ($30/$150) and Batch ($2.50/$12.50) sections present in the
        # sample, the parser must return $5/$25 for Opus 4.7.
        opus = self.rates["claude-opus-4-7"]
        self.assertEqual(opus.base_input, 5.00)
        self.assertEqual(opus.output, 25.00)
        # Explicit non-equal against Fast Mode + Batch numbers — if
        # the parser ever drifts into those tables, these catch it.
        self.assertNotEqual(opus.base_input, 30.00)
        self.assertNotEqual(opus.output, 150.00)
        self.assertNotEqual(opus.base_input, 2.50)
        self.assertNotEqual(opus.output, 12.50)


if __name__ == "__main__":
    unittest.main(verbosity=2)
