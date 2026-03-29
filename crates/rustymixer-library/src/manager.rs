//\! High-level library manager that wraps all playlist and crate operations.

use std::path::Path;

use rusqlite::Connection;

use crate::dao::crate_dao::CrateDao;
use crate::dao::playlist::PlaylistDao;
use crate::error::Result;
use crate::import;
use crate::models::{
    CrateSummary, ImportResult, Playlist, PlaylistSummary, Track,
};

/// Unified API for playlist and crate management.
pub struct LibraryManager {
    db: Connection,
}

impl LibraryManager {
    /// Create a new manager wrapping the given connection.
    pub fn new(db: Connection) -> Self {
        Self { db }
    }

    /// Borrow the underlying connection (for direct DAO access or tests).
    pub fn conn(&self) -> &Connection {
        &self.db
    }

    // -----------------------------------------------------------------
    // Playlist operations
    // -----------------------------------------------------------------

    pub fn create_playlist(&self, name: &str) -> Result<Playlist> {
        let id = PlaylistDao::create(&self.db, name)?;
        PlaylistDao::get_by_id(&self.db, id)?
            .ok_or_else(|| crate::error::LibraryError::NotFound("playlist just created".into()))
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        PlaylistDao::delete(&self.db, id)
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        PlaylistDao::rename(&self.db, id, name)
    }

    pub fn playlist_add_track(&self, playlist_id: i64, track_id: i64) -> Result<()> {
        PlaylistDao::add_track(&self.db, playlist_id, track_id)
    }

    pub fn playlist_remove_track(&self, playlist_id: i64, track_id: i64) -> Result<()> {
        PlaylistDao::remove_track(&self.db, playlist_id, track_id)
    }

    pub fn playlist_move_track(&self, playlist_id: i64, from_pos: i32, to_pos: i32) -> Result<()> {
        PlaylistDao::move_track(&self.db, playlist_id, from_pos, to_pos)
    }

    pub fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<Track>> {
        PlaylistDao::tracks(&self.db, playlist_id)
    }

    pub fn list_playlists(&self) -> Result<Vec<PlaylistSummary>> {
        PlaylistDao::list_with_counts(&self.db)
    }

    pub fn duplicate_playlist(&self, id: i64, new_name: &str) -> Result<Playlist> {
        let new_id = PlaylistDao::duplicate(&self.db, id, new_name)?;
        PlaylistDao::get_by_id(&self.db, new_id)?
            .ok_or_else(|| crate::error::LibraryError::NotFound("duplicated playlist".into()))
    }

    // -----------------------------------------------------------------
    // Crate operations
    // -----------------------------------------------------------------

    pub fn create_crate(&self, name: &str) -> Result<crate::models::Crate> {
        let id = CrateDao::create(&self.db, name)?;
        CrateDao::get_by_id(&self.db, id)?
            .ok_or_else(|| crate::error::LibraryError::NotFound("crate just created".into()))
    }

    pub fn delete_crate(&self, id: i64) -> Result<()> {
        CrateDao::delete(&self.db, id)
    }

    pub fn rename_crate(&self, id: i64, name: &str) -> Result<()> {
        CrateDao::rename(&self.db, id, name)
    }

    pub fn crate_add_track(&self, crate_id: i64, track_id: i64) -> Result<()> {
        CrateDao::add_track(&self.db, crate_id, track_id)
    }

    pub fn crate_remove_track(&self, crate_id: i64, track_id: i64) -> Result<()> {
        CrateDao::remove_track(&self.db, crate_id, track_id)
    }

    pub fn crate_tracks(&self, crate_id: i64) -> Result<Vec<Track>> {
        CrateDao::tracks(&self.db, crate_id)
    }

    pub fn list_crates(&self) -> Result<Vec<CrateSummary>> {
        CrateDao::list_with_counts(&self.db)
    }

    // -----------------------------------------------------------------
    // Playlist import
    // -----------------------------------------------------------------

    pub fn import_m3u(&self, path: &Path, playlist_name: &str) -> Result<ImportResult> {
        import::import_m3u(&self.db, path, playlist_name)
    }

    pub fn import_pls(&self, path: &Path, playlist_name: &str) -> Result<ImportResult> {
        import::import_pls(&self.db, path, playlist_name)
    }
}
