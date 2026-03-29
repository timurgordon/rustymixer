//! Error types for the music library.

use thiserror::Error;

/// Errors produced by music library operations.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("metadata error: {0}")]
    Metadata(#[from] lofty::error::LoftyError),
}

pub type Result<T> = std::result::Result<T, LibraryError>;
