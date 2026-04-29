//! `tokenscale-ingest-cc` error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("store error: {0}")]
    Store(#[from] tokenscale_store::StoreError),

    #[error("claude code root not found at {0}")]
    RootNotFound(std::path::PathBuf),
}

pub type Result<T> = std::result::Result<T, IngestError>;
