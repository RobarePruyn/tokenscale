-- v0.1.22: _tokenscale_meta key/value table.
--
-- Why: when an older binary opens a database that a newer build migrated,
-- sqlx can only report the unknown migration number. Recording which
-- tokenscale version last ran migrations lets the older binary say
-- "migrated by tokenscale X.Y; upgrade" instead of a bare id, and exit
-- with a dedicated code (3) so a service manager does not treat the
-- refusal as a crash and hot-loop (incident 2026-10-07: launchd restarted
-- a stale 0.1.10 binary 41,095 times against a 0.1.19-migrated database).
--
-- Written after every successful migration run (key
-- `last_migrated_by_version`). Forward-only and additive: creates one
-- table, touches no existing rows. Stated in CHANGELOG v0.1.22 and
-- docs/packaging-and-service.md.
CREATE TABLE _tokenscale_meta (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
