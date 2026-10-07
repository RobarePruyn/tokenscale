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

use crate::error::{Result, StoreError};

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

        run_migrations(&pool).await?;
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
        run_migrations(&pool).await?;
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

/// This crate's version, which is the workspace version every tokenscale
/// crate shares. Recorded in `_tokenscale_meta` after each migration run.
pub const BINARY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `_tokenscale_meta` key holding the version that last ran migrations.
const META_LAST_MIGRATED_BY: &str = "last_migrated_by_version";

/// Run embedded migrations, then record this binary's version in
/// `_tokenscale_meta`. If the database already contains a migration this
/// binary does not know (it was migrated by a newer tokenscale), map
/// sqlx's `VersionMissing` to [`StoreError::SchemaNewerThanBinary`],
/// naming the recording version when one exists. sqlx validates applied
/// migrations before applying any new ones, so this path writes nothing.
async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    match sqlx::migrate!("../../migrations").run(pool).await {
        Ok(()) => {
            record_migrated_by(pool).await?;
            Ok(())
        }
        Err(sqlx::migrate::MigrateError::VersionMissing(migration)) => {
            let migrated_by = read_migrated_by(pool).await;
            Err(StoreError::SchemaNewerThanBinary {
                migration,
                migrated_by,
                binary_version: BINARY_VERSION,
            })
        }
        Err(other) => Err(other.into()),
    }
}

/// Best-effort read of the last-migrating version. `None` when the meta
/// table does not exist yet (databases last migrated before v0.1.22) or
/// has no row.
async fn read_migrated_by(pool: &SqlitePool) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM _tokenscale_meta WHERE key = ?")
        .bind(META_LAST_MIGRATED_BY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn record_migrated_by(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "INSERT INTO _tokenscale_meta (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(META_LAST_MIGRATED_BY)
    .bind(BINARY_VERSION)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod schema_guard_tests {
    use super::*;

    #[tokio::test]
    async fn fresh_database_records_the_migrating_version() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let recorded = read_migrated_by(database.pool()).await;
        assert_eq!(recorded.as_deref(), Some(BINARY_VERSION));
    }

    #[tokio::test]
    async fn unknown_applied_migration_maps_to_schema_newer_than_binary() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        // Simulate a newer build having applied a migration this binary
        // does not ship, and having recorded itself as the migrator.
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
             VALUES (99991231000001, 'from the future', CURRENT_TIMESTAMP, 1, X'00', 0)",
        )
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query("UPDATE _tokenscale_meta SET value = '9.9.9' WHERE key = ?")
            .bind(META_LAST_MIGRATED_BY)
            .execute(database.pool())
            .await
            .unwrap();

        let error = run_migrations(database.pool()).await.unwrap_err();
        match error {
            StoreError::SchemaNewerThanBinary {
                migration,
                migrated_by,
                binary_version,
            } => {
                assert_eq!(migration, 99991231000001);
                assert_eq!(migrated_by.as_deref(), Some("9.9.9"));
                assert_eq!(binary_version, BINARY_VERSION);
            }
            other => panic!("expected SchemaNewerThanBinary, got {other:?}"),
        }
        // The guard must not have touched the recorded version.
        assert_eq!(
            read_migrated_by(database.pool()).await.as_deref(),
            Some("9.9.9")
        );
    }
}
