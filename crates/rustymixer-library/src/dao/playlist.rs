//! Playlist DAO.

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::models::Playlist;
use super::track::TrackDao;
use crate::models::Track;

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
