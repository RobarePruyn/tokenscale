//! tokenscale — command-line entrypoint.
//!
//! `tokenscale` is a self-hostable local dashboard that reconciles Anthropic
//! usage across two ingest paths: local Claude Code JSONL logs and the
//! Anthropic Admin API. It computes real cost, counterfactual API cost, and
//! environmental impact (energy, water, CO2e) per query and over time.
//!
//! Phase 1 surface:
//!
//! - `tokenscale init`            — create the config file and the database.
//! - `tokenscale scan`            — ingest Claude Code JSONL session logs.
//! - `tokenscale serve`           — start the local HTTP server (Phase 1 next step).
//! - `tokenscale factors update`  — Phase 3 — reserved no-op.
//! - `tokenscale factors publish` — Phase 3 — reserved no-op.

mod config;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokenscale_core::{EnvironmentalFactorsFile, PricingFile};
use tokenscale_ingest_cc::run_scan;
use tokenscale_server::{serve, AppState};
use tokenscale_store::{
    clear_file_state_for_source, delete_events_for_source, sync_environmental_factors, Database,
};

/// Source-kind constant the scan paths key off — mirrors the constant
/// inside `tokenscale-ingest-cc::scan`. Kept here too so `--rescan` /
/// `--rebuild` know which rows to clear without dragging the ingest
/// crate's internals into the CLI.
const CLAUDE_CODE_SOURCE: &str = "claude_code";

/// How the user asked us to scan — surfaces three modes the same way
/// `git checkout` exposes `--detach` etc. Keeping this in CLI-land (not
/// the ingest crate) so the ingest API stays idempotent and the
/// destructive operations are gated at the user-facing layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    /// Default — Layer-1 (mtime+len) skip, Layer-2 dedup as usual.
    Incremental,
    /// Forget file_state, re-parse every file. Events stay; dedup
    /// handles re-inserts.
    Rescan,
    /// Wipe events + file_state for the source, then re-parse. Used
    /// when prior parser output is known-bad. Requires `--yes`.
    Rebuild,
}
use tracing::{info, warn};

use crate::config::{resolve_config_path, Config};

/// Self-hostable Anthropic usage and impact dashboard.
#[derive(Parser, Debug)]
#[command(name = "tokenscale", version, about, long_about = None)]
struct CommandLineArguments {
    /// Path to the configuration TOML. Overrides $TOKENSCALE_CONFIG and the
    /// default at ~/.config/tokenscale/config.toml.
    #[arg(long, env = "TOKENSCALE_CONFIG", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: TopLevelCommand,
}

#[derive(Subcommand, Debug)]
#[command(rename_all = "kebab-case")]
enum TopLevelCommand {
    /// Initialize the local data directory, configuration file, and SQLite schema.
    Init,

    /// Incrementally ingest Claude Code JSONL session logs from ~/.claude/projects/.
    /// Idempotent: re-runs against unchanged files cost only a stat() per file.
    Scan {
        /// Forget Layer-1 file_state (mtime + len) for this source so every
        /// file gets re-parsed. Layer-2 dedup at the events table
        /// (UNIQUE source, request_id) prevents double-inserts. Useful
        /// when you suspect mtime is lying — e.g., cloud-FS sync drift.
        #[arg(long, conflicts_with = "rebuild")]
        rescan: bool,

        /// Wipe both file_state AND every event for this source, then
        /// re-parse from scratch. Destructive: only appropriate when a
        /// prior parser version emitted bad data. Requires --yes.
        #[arg(long, requires = "yes", conflicts_with = "rescan")]
        rebuild: bool,

        /// Confirms the destructive flag (`--rebuild`). No effect without
        /// it.
        #[arg(long)]
        yes: bool,
    },

    /// Start the local HTTP server and dashboard.
    Serve {
        /// Address to bind. Defaults to localhost-only.
        #[arg(long)]
        bind: Option<String>,
    },

    /// Manage the environmental-factor model. Reserved — Phase 3 surface.
    Factors {
        #[command(subcommand)]
        action: FactorsAction,
    },
}

#[derive(Subcommand, Debug)]
#[command(rename_all = "kebab-case")]
enum FactorsAction {
    /// Pull the upstream factor model from the public Git repo.
    /// Phase 3 — not yet implemented.
    Update,

    /// Publish a locally-edited factor model upstream (maintainer-only).
    /// Phase 3 — not yet implemented.
    Publish,
}

#[tokio::main]
async fn main() -> Result<()> {
    initialize_tracing();
    let arguments = CommandLineArguments::parse();
    let config_path = resolve_config_path(arguments.config.as_deref())?;

    match arguments.command {
        TopLevelCommand::Init => command_init(&config_path).await,
        TopLevelCommand::Scan {
            rescan,
            rebuild,
            yes: _yes,
        } => {
            let mode = if rebuild {
                ScanMode::Rebuild
            } else if rescan {
                ScanMode::Rescan
            } else {
                ScanMode::Incremental
            };
            command_scan(&config_path, mode).await
        }
        TopLevelCommand::Serve { bind } => command_serve(&config_path, bind).await,
        TopLevelCommand::Factors { action } => match action {
            FactorsAction::Update | FactorsAction::Publish => {
                println!("Phase 3 — not yet implemented");
                Ok(())
            }
        },
    }
}

/// Configure the tracing subscriber. Honors `RUST_LOG`; defaults to `info`.
fn initialize_tracing() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();
}

/// Implementation of `tokenscale init`.
///
/// Idempotent: re-running on a configured machine will (a) write a default
/// config only if no file exists, and (b) open the database, which applies
/// any new migrations and is a no-op when the schema is already current.
async fn command_init(config_path: &std::path::Path) -> Result<()> {
    if config_path.exists() {
        info!(path = %config_path.display(), "configuration already present, leaving as-is");
        println!("Config already exists at {}", config_path.display());
    } else {
        if let Some(parent_directory) = config_path.parent() {
            tokio::fs::create_dir_all(parent_directory)
                .await
                .with_context(|| format!("creating {}", parent_directory.display()))?;
        }
        let serialized = render_starter_config();
        tokio::fs::write(config_path, serialized)
            .await
            .with_context(|| format!("writing {}", config_path.display()))?;
        info!(path = %config_path.display(), "wrote default configuration");
        println!("Wrote default config to {}", config_path.display());
    }

    let config = Config::load_or_default(config_path)?;
    let database_path = config.effective_database_path()?;
    let database = Database::open(&database_path)
        .await
        .with_context(|| format!("opening database at {}", database_path.display()))?;
    drop(database);
    info!(path = %database_path.display(), "database initialized");
    println!("Database initialized at {}", database_path.display());
    Ok(())
}

/// Implementation of `tokenscale serve`.
///
/// Bind precedence: `--bind` flag > config `[server].bind` > built-in default
/// `127.0.0.1:8787`. The default is loopback-only because the Phase 1 build
/// has no auth.
async fn command_serve(config_path: &std::path::Path, bind_override: Option<String>) -> Result<()> {
    let config = Config::load_or_default(config_path)?;
    let database_path = config.effective_database_path()?;
    let database = Database::open(&database_path)
        .await
        .with_context(|| format!("opening database at {}", database_path.display()))?;

    let pricing = load_pricing(&config)?;
    if pricing.is_review_pending() {
        warn!(
            file_status = %pricing.file_status,
            "pricing.toml has not been reviewed against current Anthropic prices — \
             the dashboard's billable view is approximate. Set file_status = \"production\" \
             after verifying values."
        );
    }
    let pricing = Arc::new(pricing);

    let factors = load_factors(&config)?;
    info!(
        schema_version = factors.schema_version,
        file_status = %factors.file_status,
        models = factors.model_count(),
        regions = factors.region_count(),
        "loaded environmental-factors.toml"
    );
    if factors.is_placeholder() {
        warn!(
            "environmental-factors.toml is a placeholder — every numeric value is null. The \
             environmental-impact view ships in Phase 2 and will light up once Cowork research's \
             deliverable 3 lands real values."
        );
    }
    let sync_summary = sync_environmental_factors(&database, &factors)
        .await
        .context("syncing environmental factors into the database")?;
    info!(
        ?sync_summary,
        "synced environmental factors into the database"
    );
    let factors = Arc::new(factors);

    let bind_string = bind_override.unwrap_or_else(|| config.server.bind.clone());
    let bind_address: SocketAddr = bind_string
        .parse()
        .with_context(|| format!("parsing bind address {bind_string:?}"))?;

    let inference_region = config.effective_inference_region().to_owned();

    // Spawn the background scan loop before starting the HTTP server so
    // (a) the server can bind and accept requests immediately even if
    // the first scan is slow, and (b) the loop is a tokio task that
    // outlives any single request.
    let scan_handle = spawn_auto_scan_loop(
        database.clone(),
        config.effective_claude_code_root()?,
        config.ingest.store_raw,
        config.ingest.scan_interval_seconds,
    );

    info!(
        region = %inference_region,
        address = %bind_address,
        scan_interval_seconds = config.ingest.scan_interval_seconds,
        "starting tokenscale server"
    );
    let serve_result = serve(
        AppState::new(database, pricing, factors, inference_region),
        bind_address,
    )
    .await;

    // Best-effort task teardown — abort then ignore. The OS would clean
    // up regardless when the process exits, but explicit abort means
    // tests and graceful-shutdown flows don't leak the task.
    if let Some(handle) = scan_handle {
        handle.abort();
    }
    serve_result
}

/// Spawn the auto-scan loop. Returns `None` (no task) when
/// `scan_interval_seconds == 0` — the user-opt-out, equivalent to
/// running `tokenscale scan` manually whenever they want fresh data.
///
/// The loop logs scan failures but never propagates them — a transient
/// IO error shouldn't take the dashboard offline. The first scan runs
/// immediately at startup (catching the user up since their last
/// interactive session); subsequent scans run on the configured
/// interval with `MissedTickBehavior::Delay` so back-to-back ticks
/// don't pile up if a scan ever takes longer than the interval.
fn spawn_auto_scan_loop(
    database: Database,
    claude_code_root: std::path::PathBuf,
    capture_raw_payloads: bool,
    interval_seconds: u64,
) -> Option<tokio::task::JoinHandle<()>> {
    if interval_seconds == 0 {
        info!("auto-scan disabled (scan_interval_seconds = 0)");
        return None;
    }

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match run_scan(&database, &claude_code_root, capture_raw_payloads).await {
                Ok(summary) if summary.events_inserted > 0 || summary.files_parsed > 0 => {
                    info!(?summary, "auto-scan ingested fresh data");
                }
                Ok(_) => {
                    // Steady state — every file unchanged. Drop to
                    // debug so the per-minute tick doesn't spam logs.
                    tracing::debug!("auto-scan: no new data");
                }
                Err(error) => {
                    warn!(?error, "auto-scan failed; will retry on next interval");
                }
            }
        }
    });
    Some(handle)
}

/// Render the starter config that `tokenscale init` writes to disk.
/// We hand-author this rather than serializing `Config::default()` so the
/// file leads with a commented field reference — the user can read it
/// without cross-referencing the README to find every override.
fn render_starter_config() -> String {
    "# tokenscale configuration file.
# Generated by `tokenscale init`. Override any value below — anything left
# commented out resolves to its built-in default. The README and
# docs/architecture.md carry the full reference.

[inference]
# Region whose grid factors apply to your usage. Anthropic does not disclose
# which region served any given request — this is your declared assumption,
# surfaced prominently in the dashboard's environmental view. Common values:
# \"us-east-1\" (default — eGRID SRVC), \"us-east-2\" (RFCW), \"us-west-2\" (NWPP).
# default_inference_region = \"us-east-1\"

[ingest]
# Persist the full JSONL payload in events.raw. Disable to keep token counts
# and metadata only — see Privacy in the README.
store_raw = true
# Override for the Claude Code session root (default: ~/.claude/projects).
# claude_code_root = \"/path/to/claude/projects\"
# How often `tokenscale serve` re-runs an incremental scan in the background,
# in seconds. The first scan runs at startup; the interval governs subsequent
# runs. Set to 0 to disable auto-scan and run `tokenscale scan` manually.
scan_interval_seconds = 60

[storage]
# Override for the SQLite database path. Default: platform-specific —
# ~/.local/share/tokenscale/tokenscale.db on Linux,
# ~/Library/Application Support/tokenscale/tokenscale.db on macOS.
# database_path = \"/path/to/tokenscale.db\"

[server]
bind = \"127.0.0.1:8787\"

[auth]
# `localhost` (no auth, loopback bind) or `network` (passkey required).
# `network` is Phase 3.
mode = \"localhost\"

[pricing]
# Override path to pricing.toml. Unset = use the copy embedded in the
# binary at compile time. Set this to a local file to ship custom or
# freshly-verified prices without rebuilding.
# file = \"/path/to/pricing.toml\"

[factors]
# Override path to environmental-factors.toml. Same pattern as pricing —
# unset = embedded copy. The CHARTER's \"local research mode\" power-user
# workflow points this at a working copy.
# file = \"/path/to/environmental-factors.toml\"
"
    .to_owned()
}

/// Resolve and load the pricing file. Precedence: `[pricing].file` in the
/// config wins; otherwise the embedded copy ships with the binary.
fn load_pricing(config: &Config) -> Result<PricingFile> {
    if let Some(override_path) = config.pricing.file.as_ref() {
        info!(path = %override_path.display(), "loading pricing from configured override");
        return PricingFile::load_from_path(override_path)
            .with_context(|| format!("loading pricing from {}", override_path.display()));
    }
    info!("loading embedded pricing.toml shipped with this build");
    PricingFile::embedded_default().context("parsing embedded pricing.toml")
}

/// Resolve and load the environmental factor file. Same precedence rules
/// as `load_pricing`. The "local research mode" power-user workflow from
/// the CHARTER points `[factors].file` at a working copy.
fn load_factors(config: &Config) -> Result<EnvironmentalFactorsFile> {
    if let Some(override_path) = config.factors.file.as_ref() {
        info!(path = %override_path.display(), "loading environmental factors from configured override");
        return EnvironmentalFactorsFile::load_from_path(override_path).with_context(|| {
            format!(
                "loading environmental factors from {}",
                override_path.display()
            )
        });
    }
    info!("loading embedded environmental-factors.toml shipped with this build");
    EnvironmentalFactorsFile::embedded_default()
        .context("parsing embedded environmental-factors.toml")
}

/// Implementation of `tokenscale scan` — incremental by default, with
/// `--rescan` (forget file_state) and `--rebuild` (forget file_state +
/// drop events) escape hatches.
async fn command_scan(config_path: &std::path::Path, mode: ScanMode) -> Result<()> {
    let config = Config::load_or_default(config_path)?;
    let database_path = config.effective_database_path()?;
    let database = Database::open(&database_path)
        .await
        .with_context(|| format!("opening database at {}", database_path.display()))?;

    if matches!(mode, ScanMode::Rebuild) {
        let deleted = delete_events_for_source(&database, CLAUDE_CODE_SOURCE)
            .await
            .context("deleting events for rebuild")?;
        let cleared = clear_file_state_for_source(&database, CLAUDE_CODE_SOURCE)
            .await
            .context("clearing file_state for rebuild")?;
        warn!(
            deleted_events = deleted,
            cleared_file_state_rows = cleared,
            "--rebuild: events and file_state for source={CLAUDE_CODE_SOURCE} wiped; re-parsing from scratch"
        );
    } else if matches!(mode, ScanMode::Rescan) {
        let cleared = clear_file_state_for_source(&database, CLAUDE_CODE_SOURCE)
            .await
            .context("clearing file_state for rescan")?;
        info!(
            cleared_file_state_rows = cleared,
            "--rescan: file_state for source={CLAUDE_CODE_SOURCE} cleared; every file will be re-parsed"
        );
    }

    let claude_code_root = config.effective_claude_code_root()?;
    let summary = run_scan(&database, &claude_code_root, config.ingest.store_raw)
        .await
        .context("running Claude Code scan")?;

    println!(
        "Scan complete: {} files seen, {} parsed, {} unchanged. {} new events, {} duplicates skipped. {} non-assistant lines, {} malformed.",
        summary.files_seen,
        summary.files_parsed,
        summary.files_unchanged,
        summary.events_inserted,
        summary.events_duplicates,
        summary.lines_skipped,
        summary.lines_malformed
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_config_parses_cleanly_back_to_a_config() {
        // The starter we write should always be a valid Config — otherwise
        // a fresh `tokenscale init` followed by any other command would
        // fail to load the file we just produced.
        let starter = render_starter_config();
        let parsed: Config =
            toml::from_str(&starter).expect("starter config must parse as a Config");
        assert!(parsed.ingest.store_raw);
        assert_eq!(parsed.server.bind, "127.0.0.1:8787");
        assert_eq!(parsed.auth.mode, "localhost");
        // Override paths are commented out by default.
        assert!(parsed.pricing.file.is_none());
        assert!(parsed.factors.file.is_none());
    }
}
