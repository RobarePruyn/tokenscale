//! tokenscale — command-line entrypoint.
//!
//! `tokenscale` is a self-hostable local dashboard that reconciles Anthropic
//! usage across two ingest paths: local Claude Code JSONL logs and the
//! Anthropic Admin API. It computes real cost, counterfactual API cost, and
//! environmental impact (energy, water, CO2e) per query and over time.
//!
//! Subcommand surface is intentionally small in Phase 1, with `factors`
//! reserved as a no-op group so the Phase 3 distribution model
//! (pull-from-upstream and maintainer-only push) can land without a CLI
//! redesign.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    Scan,

    /// Start the local HTTP server and dashboard.
    Serve {
        /// Address to bind. Defaults to localhost-only.
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
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

    match arguments.command {
        TopLevelCommand::Init => {
            // TODO: lands with the store crate's migration runner.
            unimplemented!("`tokenscale init` lands with the store crate")
        }
        TopLevelCommand::Scan => {
            // TODO: lands with the ingest-cc crate.
            unimplemented!("`tokenscale scan` lands with the ingest-cc crate")
        }
        TopLevelCommand::Serve { bind: _ } => {
            // TODO: lands with the server crate.
            unimplemented!("`tokenscale serve` lands with the server crate")
        }
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
