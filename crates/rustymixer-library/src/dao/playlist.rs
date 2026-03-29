//! Playlist DAO.

use rusqlite::{Connection, params};

use crate::error::{LibraryError, Result};
use crate::models::{Playlist, PlaylistSummary, Track};
use super::track::TrackDao;

pub struct PlaylistDao;

impl PlaylistDao {
    pub fn create(conn: &Connection, name: &str) -> Result<i64> {
        let now = now_unix();
        let position: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlists",
            [],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO playlists (name, position, created_at) VALUES (?1, ?2, ?3)",
            params![name, position, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<Playlist>> {
        let mut stmt = conn.prepare(
            "SELECT id, name, position, created_at, is_locked FROM playlists WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(Playlist {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
                created_at: row.get(3)?,
                is_locked: row.get::<_, i32>(4)? != 0,
            })),
            None => Ok(None),
        }
    }

    pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<()> {
        conn.execute(
            "UPDATE playlists SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list(conn: &Connection) -> Result<Vec<Playlist>> {
        let mut stmt =
            conn.prepare("SELECT id, name, position, created_at, is_locked FROM playlists ORDER BY position")?;
        let rows = stmt.query_map([], |row| {
            Ok(Playlist {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
                created_at: row.get(3)?,
                is_locked: row.get::<_, i32>(4)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// List all playlists with their track counts.
    pub fn list_with_counts(conn: &Connection) -> Result<Vec<PlaylistSummary>> {
        let mut stmt = conn.prepare(
            "SELECT p.id, p.name, p.position, p.created_at, p.is_locked,
                    COUNT(pt.track_id) AS track_count
             FROM playlists p
             LEFT JOIN playlist_tracks pt ON p.id = pt.playlist_id
             GROUP BY p.id
             ORDER BY p.position",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PlaylistSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
                created_at: row.get(3)?,
                is_locked: row.get::<_, i32>(4)? != 0,
                track_count: row.get::<_, i64>(5)? as usize,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn add_track(conn: &Connection, playlist_id: i64, track_id: i64) -> Result<()> {
        let position: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
            params![playlist_id, track_id, position],
        )?;
        Ok(())
    }

    pub fn remove_track(conn: &Connection, playlist_id: i64, track_id: i64) -> Result<()> {
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            params![playlist_id, track_id],
        )?;
        Ok(())
    }

    /// Move a track from `from_pos` to `to_pos` within a playlist, shifting
    /// other entries to maintain contiguous ordering.
    pub fn move_track(conn: &Connection, playlist_id: i64, from_pos: i32, to_pos: i32) -> Result<()> {
        if from_pos == to_pos {
            return Ok(());
        }

        // Get the track_id at from_pos.
        let track_id: i64 = conn
            .query_row(
                "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 AND position = ?2",
                params![playlist_id, from_pos],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    LibraryError::NotFound(format!("no track at position {from_pos}"))
                }
                other => LibraryError::Database(other),
            })?;

        if from_pos < to_pos {
            // Moving down: shift items in (from_pos, to_pos] up by one.
            conn.execute(
                "UPDATE playlist_tracks SET position = position - 1
                 WHERE playlist_id = ?1 AND position > ?2 AND position <= ?3",
                params![playlist_id, from_pos, to_pos],
            )?;
        } else {
            // Moving up: shift items in [to_pos, from_pos) down by one.
            conn.execute(
                "UPDATE playlist_tracks SET position = position + 1
                 WHERE playlist_id = ?1 AND position >= ?2 AND position < ?3",
                params![playlist_id, to_pos, from_pos],
            )?;
        }

        // Place the moved track at to_pos.
        conn.execute(
            "UPDATE playlist_tracks SET position = ?1
             WHERE playlist_id = ?2 AND track_id = ?3",
            params![to_pos, playlist_id, track_id],
        )?;

        Ok(())
    }

    /// Duplicate a playlist with a new name, copying all track entries.
    pub fn duplicate(conn: &Connection, id: i64, new_name: &str) -> Result<i64> {
        let new_id = Self::create(conn, new_name)?;

        conn.execute(
            "INSERT INTO playlist_tracks (playlist_id, track_id, position)
             SELECT ?1, track_id, position FROM playlist_tracks WHERE playlist_id = ?2
             ORDER BY position",
            params![new_id, id],
        )?;

        Ok(new_id)
    }

    pub fn tracks(conn: &Connection, playlist_id: i64) -> Result<Vec<Track>> {
        let track_ids: Vec<i64> = {
            let mut stmt = conn.prepare(
                "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position",
            )?;
            let rows = stmt.query_map([playlist_id], |row| row.get(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut tracks = Vec::with_capacity(track_ids.len());
        for tid in track_ids {
            if let Some(t) = TrackDao::get_by_id(conn, tid)? {
                tracks.push(t);
            }
        }
        Ok(tracks)
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
