//! Database schema definitions and migration system.

use rusqlite::Connection;
use tracing::info;

use crate::error::{LibraryError, Result};

/// Current schema version. Bump this when adding new migrations.
const CURRENT_VERSION: u32 = 1;

/// SQL for the initial schema (version 1).
const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT
);

CREATE TABLE IF NOT EXISTS directories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT UNIQUE NOT NULL
);

CREATE TABLE IF NOT EXISTS track_locations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    directory_id INTEGER REFERENCES directories(id),
    filename TEXT NOT NULL,
    filesize INTEGER,
    fs_modified_at INTEGER,
    needs_verification INTEGER DEFAULT 0,
    UNIQUE(directory_id, filename)
);

CREATE TABLE IF NOT EXISTS tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    location_id INTEGER UNIQUE REFERENCES track_locations(id),
    title TEXT,
    artist TEXT,
    album TEXT,
    album_artist TEXT,
    genre TEXT,
    year TEXT,
    track_number TEXT,
    comment TEXT,
    duration_secs REAL NOT NULL DEFAULT 0,
    sample_rate INTEGER,
    channels INTEGER,
    bitrate INTEGER,
    bpm REAL,
    key TEXT,
    rating INTEGER DEFAULT 0,
    play_count INTEGER DEFAULT 0,
    last_played_at INTEGER,
    replay_gain REAL,
    added_at INTEGER NOT NULL,
    cover_art_hash TEXT,
    analyzed INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS cues (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    type INTEGER NOT NULL DEFAULT 0,
    position_frames REAL NOT NULL,
    length_frames REAL DEFAULT 0,
    hotcue_number INTEGER,
    label TEXT,
    color INTEGER
);

CREATE TABLE IF NOT EXISTS playlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    is_locked INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS playlist_tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS crates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS crate_tracks (
    crate_id INTEGER NOT NULL REFERENCES crates(id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    PRIMARY KEY (crate_id, track_id)
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_tracks_title ON tracks(title);
CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album);
CREATE INDEX IF NOT EXISTS idx_tracks_bpm ON tracks(bpm);
CREATE INDEX IF NOT EXISTS idx_cues_track_id ON cues(track_id);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist ON playlist_tracks(playlist_id, position);
CREATE INDEX IF NOT EXISTS idx_crate_tracks_crate ON crate_tracks(crate_id);
"#;

/// Manages database schema migrations.
pub struct MigrationManager;

impl MigrationManager {
    /// Run all pending migrations. Creates tables if the database is new.
    pub fn migrate(conn: &Connection) -> Result<()> {
        // Enable WAL mode and foreign keys
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;",
        )?;

        let current = Self::version(conn).unwrap_or(0);

        if current == 0 {
            info!("initializing database schema v{CURRENT_VERSION}");
            conn.execute_batch(SCHEMA_V1)
                .map_err(|e| LibraryError::Migration(format!("v1 migration failed: {e}")))?;
            Self::set_version(conn, CURRENT_VERSION)?;
        } else if current < CURRENT_VERSION {
            // Future migrations go here:
            // if current < 2 { run_v2_migration(conn)?; }
            Self::set_version(conn, CURRENT_VERSION)?;
        }

        info!("database schema at version {CURRENT_VERSION}");
        Ok(())
    }

    /// Get the current schema version (0 if settings table doesn't exist yet).
    pub fn version(conn: &Connection) -> Result<u32> {
        // Check if the settings table exists at all.
        let table_exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='settings'",
            [],
            |row| row.get(0),
        )?;

        if !table_exists {
            return Ok(0);
        }

        let version: Option<String> = conn.query_row(
            "SELECT value FROM settings WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        ).optional()?;

        match version {
            Some(v) => v
                .parse::<u32>()
                .map_err(|e| LibraryError::Migration(format!("bad schema_version: {e}"))),
            None => Ok(0),
        }
    }

    fn set_version(conn: &Connection, version: u32) -> Result<()> {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('schema_version', ?1)",
            [version.to_string()],
        )?;
        Ok(())
    }
}

/// Extension trait to make `query_row` return `Option` on missing rows.
trait OptionalRow<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalRow<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
