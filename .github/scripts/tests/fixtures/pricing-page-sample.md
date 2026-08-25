# Pricing

Learn about Anthropic's pricing structure for models and features

---

This page provides detailed pricing information for Anthropic's models and features. All prices are in USD.

For the most current pricing information, visit [claude.com/pricing](https://claude.com/pricing).

## Model pricing

The following table shows pricing for all Claude models:

| Model             | Base Input Tokens | 5m Cache Writes | 1h Cache Writes | Cache Hits & Refreshes | Output Tokens |
|-------------------|-------------------|-----------------|-----------------|----------------------|---------------|
| Claude Opus 5       | $5 / MTok         | $6.25 / MTok    | $10 / MTok      | $0.50 / MTok | $25 / MTok    |
| Claude Fable 5      | $10 / MTok        | $12.50 / MTok   | $20 / MTok      | $1 / MTok | $50 / MTok    |
| Claude Opus 4.8     | $5 / MTok         | $6.25 / MTok    | $10 / MTok      | $0.50 / MTok | $25 / MTok    |
| Claude Opus 4.7     | $5 / MTok         | $6.25 / MTok    | $10 / MTok      | $0.50 / MTok | $25 / MTok    |
| Claude Opus 4.6     | $5 / MTok         | $6.25 / MTok    | $10 / MTok      | $0.50 / MTok | $25 / MTok    |
| Claude Sonnet 4.6   | $3 / MTok         | $3.75 / MTok    | $6 / MTok       | $0.30 / MTok | $15 / MTok    |
| Claude Haiku 4.5  | $1 / MTok         | $1.25 / MTok    | $2 / MTok       | $0.10 / MTok | $5 / MTok     |

<Note>
MTok = Million tokens.
</Note>

## Cloud platform pricing

(content omitted from fixture)

## Feature-specific pricing

### Prompt caching

Prompt caching uses the following pricing multipliers relative to base input token rates:

| Cache operation | Multiplier | Duration |
|:----------------|:-----------|:---------|
| 5-minute cache write | 1.25x base input price | Cache valid for 5 minutes |
| 1-hour cache write | 2x base input price | Cache valid for 1 hour |
| Cache read (hit) | 0.1x base input price | Same duration as the preceding write |

### Data residency pricing

For Claude Opus 4.6, Claude Sonnet 4.6, and later models, specifying US-only inference through the `inference_geo` parameter incurs a 1.1x multiplier on all token pricing categories.

### Fast mode pricing

[Fast mode](/docs/en/build-with-claude/fast-mode), in beta, provides significantly faster output for Claude Opus 4.6 and Claude Opus 4.7 at premium pricing (6x standard rates).

| Input | Output |
|:------|:-------|
| $30 / MTok | $150 / MTok |

This Fast Mode table is the false-positive source the parser MUST ignore.

### Batch processing

The Batch API allows asynchronous processing of large volumes of requests with a 50% discount on both input and output tokens.

| Model             | Batch input      | Batch output    |
|-------------------|------------------|-----------------|
| Claude Opus 4.7       | $2.50 / MTok     | $12.50 / MTok   |
| Claude Sonnet 4.6   | $1.50 / MTok     | $7.50 / MTok    |
| Claude Haiku 4.5  | $0.50 / MTok     | $2.50 / MTok    |

This Batch table is another false-positive source. Parser pinned to `## Model pricing` must not see it.
