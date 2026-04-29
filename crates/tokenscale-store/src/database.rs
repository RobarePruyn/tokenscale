//! SQLite connection pool — open, migrate, expose.
//!
//! `Database` owns the `SqlitePool`. Every other module in this crate borrows
//! the pool through `Database::pool()` to run queries. Putting the pool
//! behind a struct lets us evolve the open/migrate sequence (turning on
//! integrity checks, swapping journal modes, attaching read replicas later)
//! without rewriting call sites.

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
use tracing::info;

use crate::error::Result;

/// Workspace-relative path to the migrations directory. `sqlx::migrate!`
/// resolves it relative to `CARGO_MANIFEST_DIR`, so this is two levels up
/// from `crates/tokenscale-store/`.
const MIGRATIONS_PATH: &str = "../../migrations";

/// The handle every other crate uses to talk to the database.
#[derive(Clone, Debug)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open (creating if necessary) the SQLite file at `database_file_path`,
    /// run migrations, and return a connected handle.
    ///
    /// Configures the connection for the workload tokenscale actually has:
    ///
    /// - **WAL journal mode** — concurrent reads while ingest writes, with
    ///   a single writer at a time. Suits a local desktop tool with one
    ///   ingest process and several read queries from the dashboard.
    /// - **NORMAL synchronous** — fsync on transaction commit but not on
    ///   every page write. Loss window is one transaction on power-loss,
    ///   which is acceptable for usage telemetry.
    /// - **Foreign keys ON** — required for the events.source → sources.kind
    ///   reference to be enforced.
    pub async fn open(database_file_path: &Path) -> Result<Self> {
        if let Some(parent_directory) = database_file_path.parent() {
            tokio::fs::create_dir_all(parent_directory).await?;
        }

        let connect_options =
            SqliteConnectOptions::from_str(&format!("sqlite://{}", database_file_path.display()))?
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options)
            .await?;

        info!(path = %database_file_path.display(), "opened SQLite database");

        sqlx::migrate!("../../migrations").run(&pool).await?;
        info!("migrations applied");

        Ok(Self { pool })
    }

    /// Open a fresh in-memory database with all migrations applied. For tests.
    #[doc(hidden)]
    pub async fn open_in_memory_for_tests() -> Result<Self> {
        let connect_options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connect_options)
            .await?;
        sqlx::migrate!("../../migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Where the migrations live, relative to this crate's manifest. Exposed
    /// for tooling that wants to point `sqlx-cli` at the same directory.
    #[must_use]
    pub fn migrations_path() -> &'static str {
        MIGRATIONS_PATH
    }
}
