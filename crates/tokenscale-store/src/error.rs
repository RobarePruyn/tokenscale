//! `tokenscale-store` error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// The database carries a migration this binary does not know: a
    /// newer tokenscale migrated it. sqlx refuses to proceed (correctly;
    /// the schema is ahead of the code). Surfaced as its own variant so
    /// the CLI can print an upgrade hint and exit with a dedicated code
    /// instead of a generic error that a service manager restarts forever.
    #[error(
        "database schema is newer than this binary: migration {migration} was applied by tokenscale {}, but this build (tokenscale {binary_version}) does not know it; upgrade tokenscale",
        .migrated_by.as_deref().unwrap_or("(version not recorded)")
    )]
    SchemaNewerThanBinary {
        migration: i64,
        migrated_by: Option<String>,
        binary_version: &'static str,
    },
}

pub type Result<T> = std::result::Result<T, StoreError>;
