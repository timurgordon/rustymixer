//! Database connection wrapper.

use std::path::Path;

use rusqlite::Connection;
use tracing::info;

use crate::error::Result;
use crate::schema::MigrationManager;

/// Wraps a SQLite connection, running migrations on open.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) a database file at `path` and run migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        info!(?path, "opening library database");
        let conn = Connection::open(path)?;
        MigrationManager::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Create an in-memory database (useful for tests).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        MigrationManager::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Borrow the underlying connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
