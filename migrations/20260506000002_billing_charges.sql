-- tokenscale — Phase 2 billing import (migration 0004).
--
-- New `billing_charges` table for raw billing line items imported from
-- external sources (Stripe Customer Portal CSV today; Anthropic Admin
-- API cost_report next). Sits alongside the user-declared
-- `subscriptions` table — both feed the dashboard's "what you actually
-- paid" calculation, but billing_charges is per-line-item ground truth
-- (one row per Stripe invoice line / one row per cost-report bucket),
-- whereas subscriptions is the user's recurring declared baseline.
--
-- The two coexist deliberately:
--   * Subscriptions remain the no-import default for users who don't
--     have or want to upload a billing CSV.
--   * When billing_charges covers a date range, it's the higher-fidelity
--     source of truth (Stripe knows the exact amount, including
--     overages, refunds, and proration).
--   * The dashboard handler resolves overlap by preferring
--     billing_charges for any month that has at least one row, falling
--     back to manually-declared subscriptions for uncovered dates. The
--     import preview surfaces overlaps explicitly so the user can
--     dismiss now-redundant manual entries inline.

PRAGMA foreign_keys = ON;

CREATE TABLE billing_charges (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Where this row came from. Mirrors `events.source` style — opaque
    -- string the ingest layer assigns. Phase 2 source values:
    --   * 'stripe_csv'      — user-uploaded Stripe Customer Portal CSV
    --   * 'anthropic_admin' — pulled from /v1/organizations/cost_report
    source        TEXT    NOT NULL,

    -- Charge / invoice date in ISO-8601 (date precision is sufficient;
    -- Stripe doesn't expose sub-day timing for billing line items, and
    -- the dashboard buckets by day anyway).
    occurred_at   TEXT    NOT NULL,

    -- Billed amount. Positive for charges, negative for refunds /
    -- credits. We store USD only — multi-currency support is a future
    -- concern (would require a currency column + FX conversion).
    amount_usd    REAL    NOT NULL,

    -- Free-form line-item description, e.g.
    -- "Claude Pro - Subscription" or "API usage - April 2026".
    -- Used for the categorization heuristic and for display in the
    -- import preview.
    description   TEXT,

    -- Computed at import time, user-overridable in the preview UI:
    --   * 'subscription' — recurring same-amount monthly charge
    --   * 'overage'      — usage-based one-off, "overage" / "usage" /
    --                      "API" tokens in the description
    --   * 'one_time'     — single non-recurring charge (e.g. setup fee)
    --   * 'refund'       — negative-amount line
    --   * 'unknown'      — heuristic abstained
    -- Stored as plain text (not enum) so we can introduce new categories
    -- without a migration; the application layer treats unknown values
    -- as 'unknown'.
    category      TEXT    NOT NULL,

    -- The provider's stable ID for this line item — Stripe charge_id,
    -- invoice ID, line item ID, etc. Combined with `source` to dedupe
    -- re-imports of the same CSV. Nullable because some sources
    -- (early Stripe CSVs without IDs) may lack one; in that case the
    -- import path synthesizes a hash of (date, amount, description).
    external_id   TEXT,

    -- Raw CSV row (or API response chunk) as JSON, for audit. Lets us
    -- show the user "this is exactly what your Stripe export said" if
    -- they question a categorization.
    raw           TEXT,

    -- When tokenscale ingested this row. Distinct from `occurred_at`
    -- (the charge's own timestamp). ISO-8601 UTC.
    created_at    TEXT    NOT NULL,

    -- Re-imports of the same source must dedupe — UNIQUE here means an
    -- import is idempotent. Conflict resolution is ON CONFLICT IGNORE
    -- at the application layer.
    UNIQUE (source, external_id)
);

-- Range scans by occurred_at drive the dashboard "billed in window"
-- query and the stat-row total.
CREATE INDEX billing_charges_occurred_at_idx
    ON billing_charges (occurred_at);

-- Source-scoped lookups (e.g., "show me everything from stripe_csv").
CREATE INDEX billing_charges_source_idx
    ON billing_charges (source);
