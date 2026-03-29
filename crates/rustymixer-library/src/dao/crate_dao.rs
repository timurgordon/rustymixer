//! Crate DAO (music collection/tag groups).
//!
//! Named `crate_dao` because `crate` is a Rust keyword.

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::models::{self, CrateSummary, Track};
use super::track::TrackDao;

pub struct CrateDao;

impl CrateDao {
    pub fn create(conn: &Connection, name: &str) -> Result<i64> {
        let now = now_unix();
        conn.execute(
            "INSERT INTO crates (name, created_at) VALUES (?1, ?2)",
            params![name, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<models::Crate>> {
        let mut stmt = conn.prepare(
            "SELECT id, name, created_at FROM crates WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(models::Crate {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })),
            None => Ok(None),
        }
    }

    pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<()> {
        conn.execute(
            "UPDATE crates SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM crates WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list(conn: &Connection) -> Result<Vec<models::Crate>> {
        let mut stmt =
            conn.prepare("SELECT id, name, created_at FROM crates ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok(models::Crate {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// List all crates with their track counts.
    pub fn list_with_counts(conn: &Connection) -> Result<Vec<CrateSummary>> {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.name, c.created_at, COUNT(ct.track_id) AS track_count
             FROM crates c
             LEFT JOIN crate_tracks ct ON c.id = ct.crate_id
             GROUP BY c.id
             ORDER BY c.name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CrateSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                track_count: row.get::<_, i64>(3)? as usize,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn add_track(conn: &Connection, crate_id: i64, track_id: i64) -> Result<()> {
        conn.execute(
            "INSERT OR IGNORE INTO crate_tracks (crate_id, track_id) VALUES (?1, ?2)",
            params![crate_id, track_id],
        )?;
        Ok(())
    }

    pub fn remove_track(conn: &Connection, crate_id: i64, track_id: i64) -> Result<()> {
        conn.execute(
            "DELETE FROM crate_tracks WHERE crate_id = ?1 AND track_id = ?2",
            params![crate_id, track_id],
        )?;
        Ok(())
    }

    pub fn tracks(conn: &Connection, crate_id: i64) -> Result<Vec<Track>> {
        let track_ids: Vec<i64> = {
            let mut stmt = conn.prepare(
                "SELECT track_id FROM crate_tracks WHERE crate_id = ?1",
            )?;
            let rows = stmt.query_map([crate_id], |row| row.get(0))?;
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
