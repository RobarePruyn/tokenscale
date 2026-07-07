"""Tests for pricing_drift_check.

Run locally:
    python3 .github/scripts/tests/test_pricing_drift_check.py

CI runs the same script (via .github/workflows/pricing-drift-check.yml's
"unit tests" step) before invoking the live drift check. If parser
tests fail, the workflow stops — we don't trust a parser we know is
broken to make drift calls.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

# Make the script importable.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import pricing_drift_check as detector  # noqa: E402


FIXTURE_PATH = Path(__file__).parent / "fixtures" / "pricing-page-sample.md"


def load_fixture() -> str:
    return FIXTURE_PATH.read_text()


class ParseModelPricingTests(unittest.TestCase):
    """Parser pinning — the load-bearing guard against the Fast Mode
    false-positive (six-times-standard rates that would silently flag
    every row as 'drifted' if the parser locked onto the wrong table).
    """

    def setUp(self) -> None:
        self.page = load_fixture()
        self.rates = detector.parse_anthropic_page(self.page)

    def test_opus_4_7_base_rates_extracted(self):
        opus = self.rates["claude-opus-4-7"]
        self.assertEqual(opus.base_input, 5.00)
        self.assertEqual(opus.output, 25.00)
        self.assertEqual(opus.cache_read, 0.50)
        self.assertEqual(opus.cache_write_5m, 6.25)
        self.assertEqual(opus.cache_write_1h, 10.00)

    def test_sonnet_4_6_base_rates_extracted(self):
        s = self.rates["claude-sonnet-4-6"]
        self.assertEqual(s.base_input, 3.00)
        self.assertEqual(s.output, 15.00)
        self.assertEqual(s.cache_read, 0.30)

    def test_haiku_4_5_base_rates_extracted(self):
        h = self.rates["claude-haiku-4-5"]
        self.assertEqual(h.base_input, 1.00)
        self.assertEqual(h.output, 5.00)
        self.assertEqual(h.cache_read, 0.10)

    def test_fast_mode_rates_do_not_contaminate_opus(self):
        """The Fast Mode table says $30 / $150 for Opus. If the parser
        bled into that section, this assertion would catch it."""
        opus = self.rates["claude-opus-4-7"]
        self.assertNotEqual(opus.base_input, 30.00, "parser leaked into Fast Mode table")
        self.assertNotEqual(opus.output, 150.00, "parser leaked into Fast Mode table")

    def test_batch_rates_do_not_contaminate_any_row(self):
        """Batch table says $2.50 / $12.50 for Opus, $1.50 / $7.50 for
        Sonnet, $0.50 / $2.50 for Haiku. Any of these would be a tell
        that the parser drifted out of the Model pricing section."""
        for model_id, batch_input, batch_output in [
            ("claude-opus-4-7", 2.50, 12.50),
            ("claude-sonnet-4-6", 1.50, 7.50),
            ("claude-haiku-4-5", 0.50, 2.50),
        ]:
            r = self.rates[model_id]
            self.assertNotEqual(r.base_input, batch_input, f"{model_id} leaked into Batch")
            self.assertNotEqual(r.output, batch_output, f"{model_id} leaked into Batch")

    def test_only_tracked_models_returned(self):
        """Even if Anthropic adds rows for models we don't track (e.g.
        Claude Opus 4.1, Claude Haiku 3.5), the detector should not
        return them. The fixture omits these to keep the test focused,
        but the loop in parse_anthropic_page iterates TRACKED_MODELS
        only, so this is structural."""
        self.assertEqual(set(self.rates.keys()), set(detector.TRACKED_MODELS.keys()))


class ParseCacheMultipliersTests(unittest.TestCase):
    def setUp(self) -> None:
        self.page = load_fixture()
        self.multipliers = detector.parse_cache_multipliers(self.page)

    def test_cache_read_multiplier_is_0_1(self):
        self.assertEqual(self.multipliers["cache_read"], 0.1)

    def test_cache_write_5m_is_1_25(self):
        self.assertEqual(self.multipliers["cache_write_5m"], 1.25)

    def test_cache_write_1h_is_2x(self):
        self.assertEqual(self.multipliers["cache_write_1h"], 2.0)


class ParseFailureGuardTests(unittest.TestCase):
    """If Anthropic restructures the page, the detector must report a
    ParseFailure (exit code 2 = warn, don't fail the build) rather than
    silently mis-parsing or returning empty drift."""

    def test_missing_model_pricing_heading_raises(self):
        page = "## Other heading\n\nSome content."
        with self.assertRaises(detector.ParseFailure):
            detector.parse_anthropic_page(page)

    def test_missing_expected_column_raises(self):
        page = (
            "## Model pricing\n\n"
            "| Model | Standard Input | Other |\n"
            "|-------|----------------|-------|\n"
            "| Claude Opus 4.7 | $5 | $25 |\n"
        )
        with self.assertRaises(detector.ParseFailure):
            detector.parse_anthropic_page(page)

    def test_missing_tracked_model_row_raises(self):
        page = (
            "## Model pricing\n\n"
            "| Model | Base Input Tokens | 5m Cache Writes | 1h Cache Writes "
            "| Cache Hits & Refreshes | Output Tokens |\n"
            "|-------|-------------------|-----------------|-----------------"
            "|----------------------|---------------|\n"
            # Only Sonnet present — Opus 4.7/4.6 and Haiku 4.5 missing.
            "| Claude Sonnet 4.6 | $3 / MTok | $3.75 / MTok | $6 / MTok "
            "| $0.30 / MTok | $15 / MTok |\n"
        )
        with self.assertRaises(detector.ParseFailure):
            detector.parse_anthropic_page(page)

    def test_missing_cache_multiplier_prose_raises(self):
        page = "### Other section\n\nNo multipliers here."
        with self.assertRaises(detector.ParseFailure):
            detector.parse_cache_multipliers(page)


class RetiredModelGuardTests(unittest.TestCase):
    """D5 (v0.1.19): a model marked status='retired' in pricing.toml that has
    been delisted from the live page is skipped (logged, not flagged), so a
    pull/delisting cannot cry-wolf the detector. Fable 5 is the case."""

    def _page_without_fable(self) -> str:
        return "\n".join(
            line
            for line in FIXTURE_PATH.read_text().splitlines()
            if "Claude Fable 5" not in line
        )

    def test_retired_model_missing_from_page_raises_without_guard(self):
        # Sanity: Fable is a tracked model, so absent-from-page is a
        # ParseFailure when the retired set is empty (the default).
        with self.assertRaises(detector.ParseFailure):
            detector.parse_anthropic_page(self._page_without_fable())

    def test_retired_model_missing_from_page_is_skipped_with_guard(self):
        rates = detector.parse_anthropic_page(
            self._page_without_fable(), retired_ids=frozenset({"claude-fable-5"})
        )
        self.assertNotIn("claude-fable-5", rates)
        # Live (non-retired) models are still parsed normally.
        self.assertIn("claude-opus-4-8", rates)
        self.assertIn("claude-opus-4-7", rates)

    def test_load_retired_model_ids_reads_fable_from_pricing_toml(self):
        retired = detector.load_retired_model_ids()
        self.assertIn("claude-fable-5", retired)
        self.assertNotIn("claude-opus-4-8", retired)  # live model, not retired


class DriftCheckTests(unittest.TestCase):
    """End-to-end drift detection against in-memory upstream + local."""

    def _upstream(self) -> dict[str, detector.ModelRates]:
        return {
            "claude-opus-4-7": detector.ModelRates(
                display_name="Claude Opus 4.7",
                base_input=5.00, output=25.00, cache_read=0.50,
                cache_write_5m=6.25, cache_write_1h=10.00,
            ),
            "claude-sonnet-4-6": detector.ModelRates(
                display_name="Claude Sonnet 4.6",
                base_input=3.00, output=15.00, cache_read=0.30,
                cache_write_5m=3.75, cache_write_1h=6.00,
            ),
            "claude-haiku-4-5": detector.ModelRates(
                display_name="Claude Haiku 4.5",
                base_input=1.00, output=5.00, cache_read=0.10,
                cache_write_5m=1.25, cache_write_1h=2.00,
            ),
        }

    def _local_in_sync(self) -> dict[str, dict[str, float]]:
        return {
            "claude-opus-4-7": {
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

    def test_no_drift_when_in_sync(self):
        result = detector.check_drift(
            self._upstream(),
            {"cache_read": 0.1, "cache_write_5m": 1.25, "cache_write_1h": 2.0},
            self._local_in_sync(),
        )
        self.assertEqual(result, [])

    def test_drift_detected_on_input_rate(self):
        """The original v0.1.0–v0.1.10 bug: pricing.toml Opus input
        was $15 when Anthropic published $5."""
        local = self._local_in_sync()
        local["claude-opus-4-7"]["input"] = 15.00
        local["claude-opus-4-7"]["cache_read"] = 1.50  # consistent with old input
        result = detector.check_drift(
            self._upstream(),
            {"cache_read": 0.1, "cache_write_5m": 1.25, "cache_write_1h": 2.0},
            local,
        )
        self.assertTrue(
            any("claude-opus-4-7 input" in line for line in result),
            f"expected input drift to be flagged; got: {result}",
        )

    def test_internal_inconsistency_detected(self):
        """If someone updates input but forgets to recompute the
        absolute cache_read (which is stored as USD/MTok, not as a
        multiplier), this check catches it."""
        local = self._local_in_sync()
        # Input is correct, but cache_read was forgotten.
        local["claude-opus-4-7"]["cache_read"] = 1.50  # should be 0.50
        result = detector.check_drift(
            self._upstream(),
            {"cache_read": 0.1, "cache_write_5m": 1.25, "cache_write_1h": 2.0},
            local,
        )
        # Two drifts: (1) cache_read vs Anthropic, (2) internal consistency.
        self.assertTrue(any("INTERNAL" in line for line in result))
        self.assertTrue(any("cache_read" in line for line in result))

    def test_cache_multiplier_drift_flagged(self):
        """If Anthropic changes the cache-write multiplier convention
        (e.g. 5m moves from 1.25x → 1.5x), the detector flags it."""
        result = detector.check_drift(
            self._upstream(),
            # Anthropic-published multiplier differs from EXPECTED_CACHE_MULTIPLIERS.
            {"cache_read": 0.1, "cache_write_5m": 1.5, "cache_write_1h": 2.0},
            self._local_in_sync(),
        )
        self.assertTrue(any("cache_write_5m" in line for line in result))


if __name__ == "__main__":
    unittest.main(verbosity=2)
