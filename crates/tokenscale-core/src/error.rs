//! `tokenscale-core` error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    /// The factor TOML's `schema_version` is outside the supported range.
    /// This is the load-bearing guard for Cowork-side breaking shape
    /// changes — refusing to start is correct behavior.
    #[error(
        "factor file schema_version {found} is not supported by this build (supported: {supported})"
    )]
    UnsupportedSchemaVersion { found: i64, supported: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
