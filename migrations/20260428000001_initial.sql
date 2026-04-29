-- tokenscale — initial schema (migration 0001).
--
-- Design notes:
--
--   * `events.source` is a foreign-key reference to `sources.kind`, not a
--     CHECK enum, so v2 can add ingest sources by row insertion alone.
--
--   * `sources` carries a `provider` column so the dashboard can filter
--     by provider from day one. v1 has only `anthropic`.
--
--   * Pricing and environmental factors are versioned by `valid_from` /
--     `valid_to`, so historical events resolve against the values that were
--     authoritative at *their* timestamp. `valid_to IS NULL` means "current".
--
--   * `events` has TWO uniqueness pathways for idempotency:
--       (source, request_id)        when request_id is present
--       (source, content_hash)      when request_id is null but a stable
--                                   projection of the row can be hashed.
--     Both are partial unique indexes — SQLite supports `WHERE` clauses on
--     unique indexes.
--
--   * `_ingest_file_state` is an internal bookkeeping table (leading
--     underscore) tracking last-seen file mtimes so re-scans can skip
--     unchanged files cheaply.

PRAGMA foreign_keys = ON;

-- ----------------------------------------------------------------------------
-- sources — registry of ingest-source kinds.
-- ----------------------------------------------------------------------------
CREATE TABLE sources (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    kind          TEXT    NOT NULL UNIQUE,
    display_name  TEXT    NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    provider      TEXT    NOT NULL
);

INSERT OR IGNORE INTO sources (kind, display_name, enabled, provider) VALUES
    ('claude_code', 'Claude Code (local JSONL)', 1, 'anthropic'),
    ('admin_api',   'Anthropic Admin API',       1, 'anthropic');

-- ----------------------------------------------------------------------------
-- events — one row per usage event from any source.
-- ----------------------------------------------------------------------------
CREATE TABLE events (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    source                   TEXT    NOT NULL REFERENCES sources(kind),
    occurred_at              TEXT    NOT NULL,   -- ISO-8601 UTC
    model                    TEXT    NOT NULL,

    input_tokens             INTEGER NOT NULL DEFAULT 0,
    output_tokens            INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens        INTEGER NOT NULL DEFAULT 0,
    cache_write_5m_tokens    INTEGER NOT NULL DEFAULT 0,
    cache_write_1h_tokens    INTEGER NOT NULL DEFAULT 0,

    request_id               TEXT,
    content_hash             TEXT,

    session_id               TEXT,
    project_id               TEXT,
    workspace_id             TEXT,
    api_key_id               TEXT,

    raw                      TEXT
);

-- Idempotency: dedupe on request_id when present.
CREATE UNIQUE INDEX events_source_request_id_unique
    ON events (source, request_id)
    WHERE request_id IS NOT NULL;

-- Idempotency fallback: dedupe on content_hash when request_id is missing.
CREATE UNIQUE INDEX events_source_content_hash_unique
    ON events (source, content_hash)
    WHERE request_id IS NULL AND content_hash IS NOT NULL;

-- Query indexes (kept minimal; add when a query plan calls for it).
CREATE INDEX events_occurred_at_idx        ON events (occurred_at);
CREATE INDEX events_source_occurred_at_idx ON events (source, occurred_at);
CREATE INDEX events_model_occurred_at_idx  ON events (model, occurred_at);

-- ----------------------------------------------------------------------------
-- pricing — versioned per-provider model pricing.
-- ----------------------------------------------------------------------------
CREATE TABLE pricing (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    provider                  TEXT NOT NULL,
    model                     TEXT NOT NULL,
    valid_from                TEXT NOT NULL,
    valid_to                  TEXT,

    input_usd_per_mtok        REAL NOT NULL,
    output_usd_per_mtok       REAL NOT NULL,
    cache_read_usd_per_mtok   REAL NOT NULL,
    cache_write_5m_multiplier REAL NOT NULL,
    cache_write_1h_multiplier REAL NOT NULL,

    source_url                TEXT NOT NULL,
    source_accessed_at        TEXT NOT NULL
);

CREATE INDEX pricing_provider_model_idx ON pricing (provider, model, valid_from);

-- ----------------------------------------------------------------------------
-- env_factors — versioned environmental factors per (provider, model).
-- ----------------------------------------------------------------------------
CREATE TABLE env_factors (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider                    TEXT NOT NULL,
    model                       TEXT NOT NULL,
    valid_from                  TEXT NOT NULL,
    valid_to                    TEXT,

    -- All Wh-per-million-token values are nullable.  null means "not
    -- disclosed / not estimated"; tokenscale-core handles the gap explicitly.
    wh_per_mtok_input           REAL,
    wh_per_mtok_output          REAL,
    wh_per_mtok_cache_read      REAL,
    wh_per_mtok_cache_write_5m  REAL,
    wh_per_mtok_cache_write_1h  REAL,

    source_doc                  TEXT NOT NULL,
    notes                       TEXT
);

CREATE INDEX env_factors_provider_model_idx ON env_factors (provider, model, valid_from);

-- ----------------------------------------------------------------------------
-- grid_factors — versioned per-region grid factors.
-- ----------------------------------------------------------------------------
CREATE TABLE grid_factors (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    region              TEXT NOT NULL,
    valid_from          TEXT NOT NULL,
    valid_to            TEXT,

    co2e_kg_per_kwh     REAL NOT NULL,
    water_l_per_kwh     REAL,
    pue                 REAL NOT NULL,

    source_url          TEXT NOT NULL,
    source_accessed_at  TEXT NOT NULL
);

CREATE INDEX grid_factors_region_idx ON grid_factors (region, valid_from);

-- ----------------------------------------------------------------------------
-- subscriptions — flat-fee subscription windows for the "real cost" view.
-- ----------------------------------------------------------------------------
CREATE TABLE subscriptions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at   TEXT NOT NULL,
    ended_at     TEXT,
    plan_name    TEXT NOT NULL,
    monthly_usd  REAL NOT NULL
);

-- ----------------------------------------------------------------------------
-- research_runs — backs the in-browser research-runs review surface (Phase 3).
-- ----------------------------------------------------------------------------
CREATE TABLE research_runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_at      TEXT NOT NULL,
    kind        TEXT NOT NULL,   -- 'factor_proposal' | 'pricing_check' | ...
    summary     TEXT NOT NULL,
    result_url  TEXT
);

-- ----------------------------------------------------------------------------
-- _ingest_file_state — internal bookkeeping for incremental scans.
-- ----------------------------------------------------------------------------
-- Tracks the last-seen mtime of each scanned file so re-scans skip files that
-- haven't changed.  Naming convention: leading underscore for tables that are
-- implementation detail of the ingest layer, not part of the user-facing data
-- model.
CREATE TABLE _ingest_file_state (
    source            TEXT NOT NULL,
    file_path         TEXT NOT NULL,
    mtime_ns          INTEGER NOT NULL,   -- nanoseconds since unix epoch
    last_scanned_at   TEXT NOT NULL,      -- ISO-8601 UTC
    PRIMARY KEY (source, file_path)
);
