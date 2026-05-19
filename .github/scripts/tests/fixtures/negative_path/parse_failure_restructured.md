# Pricing

THIS FIXTURE IS DELIBERATELY MALFORMED. DO NOT USE AS A PRODUCTION REFERENCE.

Lives under `tests/fixtures/negative_path/` so the live detector path
(`fetch_live_page` over the real Anthropic URL + `load_snapshot` from
`pricing-rate-card.snapshot.json` at the repo root) cannot resolve to it.
Consumed only by `test_drift_detector_exit_codes.py`.

The page intentionally restructures the Model pricing column header
text so the parser's step-3 column-header-validation check fails and
raises `ParseFailure`. The detector's `main()` must catch it, print
a `::warning::Parse failure:` line, and exit 2 (warn-only) — NOT
exit 1 (would open a false drift alert).

The exact header string the parser looks for is intentionally NOT
named in this prose, because step 3's `in section` check would then
find it here and the failure mode would not trigger. The fixture
contains "Standard Input" in the table itself; the detector expects
something else.

---

## Model pricing

The following table shows pricing for all Claude models:

| Model             | Standard Input | 5m Cache Writes | 1h Cache Writes | Cache Hits & Refreshes | Output Tokens |
|-------------------|----------------|-----------------|-----------------|----------------------|---------------|
| Claude Opus 4.7   | $5 / MTok      | $6.25 / MTok    | $10 / MTok      | $0.50 / MTok | $25 / MTok    |
| Claude Opus 4.6   | $5 / MTok      | $6.25 / MTok    | $10 / MTok      | $0.50 / MTok | $25 / MTok    |
| Claude Sonnet 4.6 | $3 / MTok      | $3.75 / MTok    | $6 / MTok       | $0.30 / MTok | $15 / MTok    |
| Claude Haiku 4.5  | $1 / MTok      | $1.25 / MTok    | $2 / MTok       | $0.10 / MTok | $5 / MTok     |

The column heading above is intentionally restructured. Step 3 of the
parser rejects the section on this exact basis: if Anthropic restructures
the table, the detector must NOT silently mis-parse and must NOT fire a
false drift alert.

## Cloud platform pricing

(content omitted from fixture)

## Feature-specific pricing

### Prompt caching

| Cache operation | Multiplier | Duration |
|:----------------|:-----------|:---------|
| 5-minute cache write | 1.25x base input price | Cache valid for 5 minutes |
| 1-hour cache write | 2x base input price | Cache valid for 1 hour |
| Cache read (hit) | 0.1x base input price | Same duration as the preceding write |
