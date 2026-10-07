# Packaging, upgrades, and the background service

How tokenscale reaches a machine, how it upgrades, and why it does not run at login by default. Companion to `RELEASING.md` (how a release is cut). Written after the 2026-10-07 incident described at the end.

## The release chain

1. A tag matching `v*.*.*` is pushed to `RobarePruyn/tokenscale`.
2. `.github/workflows/release.yml` (cargo-dist 0.31) builds the binaries, signs and notarizes the macOS ones, creates the GitHub Release, and publishes `Formula/tokenscale-cli.rb` to the tap repo `RobarePruyn/homebrew-tokenscale`.
3. The tap repo's `amend-formula.yml` splices in the `caveats` and `service` blocks cargo-dist cannot express, and commits the amended formula.
4. `brew update && brew upgrade tokenscale-cli` on a machine picks up the new formula.

Every link is automatic once the tag exists. The chain has two external dependencies that can silently break it, both covered below: Apple's developer agreement (step 2) and Homebrew's tap trust gate (step 4). A failure anywhere in step 2 or 3 now opens a `release-failure` issue on the main repo (the `notify-failure` job in `release.yml`), so a broken chain is visible without anyone reading Actions logs.

## Homebrew on a machine

Homebrew 7 refuses to load formulae from third-party taps it has not been told to trust. A machine that tapped `robarepruyn/tokenscale` before that gate existed keeps its installed Cellar but can no longer evaluate the formula, so `brew upgrade` neither upgrades nor complains. One-time fix per machine:

```
brew trust robarepruyn/tokenscale
brew update
brew upgrade tokenscale-cli
```

Caveats printed after install describe on-demand use first and the optional service second.

## Why serve is on-demand, not a login service

The dashboard is useful when someone is looking at it. The scan it drives is incremental and runs at `serve` startup, so nothing is lost by not running between sessions: the next `tokenscale serve` catches up on everything written to `~/.claude/projects` since the last one. There is no alerting, no remote access, and no time-critical ingest that would justify a resident process. The maintainer's machine policy ("only actively needed things run at login") is the general case, not an exception, so the default is:

```
tokenscale serve      # when you want the dashboard; Ctrl-C when done
```

The formula still carries a `service do` block so people who want `brew services start tokenscale-cli` can opt in; it is not started by install. Its settings are deliberately conservative:

| Setting | Value | launchd effect | Why |
|---|---|---|---|
| `keep_alive crashed: true` | `KeepAlive = {Crashed = true}` | restart only after a signal exit | a deliberate `exit 3` (schema refusal) is not a crash and must not loop |
| `throttle_interval 300` | `ThrottleInterval = 300` | at most one launch per 5 minutes | bounds any loop to 288 starts per day |
| `environment_variables RUST_LOG: "warn"` | `EnvironmentVariables` | WARN and above only | the service log is append-only with no rotation; keep it small |

These were verified against Homebrew's `service.rb` on 2026-10-07 (`restart_delay` is launchd `TimeOut`, not the throttle; the throttle is `throttle_interval`). Homebrew does not rotate service logs. If the opt-in service is used long-term, add a `newsyslog.d` entry for `$(brew --prefix)/var/log/tokenscale.log` or run `serve` under a supervisor that rotates; tracked as a follow-up, not shipped.

## Schema newer than binary: the fail-safe

Migrations are forward-only (`migrations/`, applied by sqlx on every open). A database migrated by a newer tokenscale therefore contains migrations an older binary does not know, and sqlx correctly refuses to open it. As of v0.1.22 that refusal is explicit:

- Every open path records the migrating version in `_tokenscale_meta` (`last_migrated_by_version`) after a successful migration run.
- When sqlx reports an unknown applied migration, tokenscale prints which version migrated the database and which migration is unknown, says that nothing was changed, names the fix (`brew upgrade tokenscale-cli`), and exits with code **3**.
- Exit code 3 is reserved for this condition. Service managers configured to restart only on crashes leave it alone.

Databases last migrated before v0.1.22 have no recorded version; the message then says "a newer version (not recorded)" and still names the unknown migration.

## Upgrading a machine that has a live database

1. Back up first. The database is a single file; copy it with the `-wal` sidecar if that file is non-empty:
   ```
   D="$HOME/Library/Application Support/tokenscale"
   mkdir -p "$D/backups"
   cp -p "$D/tokenscale.db" "$D/backups/tokenscale-$(date +%F)-pre-<version>.db"
   sqlite3 -readonly "$D/backups/tokenscale-<...>.db" "PRAGMA quick_check;"
   ```
2. `brew upgrade tokenscale-cli`.
3. Run any command that opens the database (`tokenscale audit pricing-launch-dates` is read-mostly and also syncs pricing). Migrations apply on open; this is the one moment the schema changes.
4. Only then, if you want the opt-in service: `brew services start tokenscale-cli`.

Downgrading is not supported (forward-only). To go back, restore the backup file and install the older version.

## Incident 2026-10-07: crash-looping stale install

**What happened.** The machine had tokenscale 0.1.10 from Homebrew (installed 2026-05-16) and a `brew services` plist with `KeepAlive = true`. Development builds (v0.1.13 through v0.1.19) had since migrated the production database in place. The 0.1.10 binary failed on every start with `migration 20260520000001 was previously applied but is missing in the resolved migrations`, exited non-zero, and launchd relaunched it immediately: 41,095 restarts and a 529 MB log before it was booted out.

**Why the upgrade path did not save it.** Two independent breaks. (1) Homebrew's tap trust gate meant `brew upgrade` could not evaluate the formula, so the machine never saw the 0.1.17 and 0.1.18 formula bumps that did publish. (2) The release workflow failed for v0.1.19, v0.1.20, and v0.1.21 (all tagged 2026-08-25) at macOS notarization with Apple's `HTTP 403: A required agreement is missing or has expired`, so no release or formula existed past 0.1.18. The failures went unnoticed for six weeks.

**What changed.** Fail-safe refusal with exit code 3 and the `_tokenscale_meta` version record (this release); crash-only, throttled, WARN-level service block in the tap; `release-failure` issue automation and a self-explaining agreement error in `release.yml`; `brew trust` documented; this runbook. The Apple agreement itself has to be accepted by the account holder; see `RELEASING.md`.
