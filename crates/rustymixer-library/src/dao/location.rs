//! Track-location DAO — maps files on disk to their directory entry.

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::models::{NewTrackLocation, TrackLocation};

pub struct LocationDao;

impl LocationDao {
    pub fn insert(conn: &Connection, loc: &NewTrackLocation) -> Result<i64> {
        conn.execute(
            "INSERT INTO track_locations (directory_id, filename, filesize, fs_modified_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![loc.directory_id, loc.filename, loc.filesize, loc.fs_modified_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<TrackLocation>> {
        let mut stmt = conn.prepare(
            "SELECT id, directory_id, filename, filesize, fs_modified_at, needs_verification
             FROM track_locations WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_location(row)?)),
            None => Ok(None),
        }
    }

    pub fn find(conn: &Connection, dir_id: i64, filename: &str) -> Result<Option<TrackLocation>> {
        let mut stmt = conn.prepare(
            "SELECT id, directory_id, filename, filesize, fs_modified_at, needs_verification
             FROM track_locations WHERE directory_id = ?1 AND filename = ?2",
        )?;
        let mut rows = stmt.query(params![dir_id, filename])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_location(row)?)),
            None => Ok(None),
        }
    }

    pub fn update(conn: &Connection, loc: &TrackLocation) -> Result<()> {
        conn.execute(
            "UPDATE track_locations SET directory_id = ?1, filename = ?2, filesize = ?3,
             fs_modified_at = ?4, needs_verification = ?5 WHERE id = ?6",
            params![
                loc.directory_id,
                loc.filename,
                loc.filesize,
                loc.fs_modified_at,
                loc.needs_verification as i32,
                loc.id,
            ],
        )?;
        Ok(())
    }

    pub fn list_by_directory(conn: &Connection, dir_id: i64) -> Result<Vec<TrackLocation>> {
        let mut stmt = conn.prepare(
            "SELECT id, directory_id, filename, filesize, fs_modified_at, needs_verification
             FROM track_locations WHERE directory_id = ?1",
        )?;
        let rows = stmt.query_map([dir_id], row_to_location)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn mark_needs_verification(conn: &Connection, id: i64) -> Result<()> {
        conn.execute(
            "UPDATE track_locations SET needs_verification = 1 WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM track_locations WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn row_to_location(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackLocation> {
    Ok(TrackLocation {
        id: row.get(0)?,
        directory_id: row.get(1)?,
        filename: row.get(2)?,
        filesize: row.get(3)?,
        fs_modified_at: row.get(4)?,
        needs_verification: row.get::<_, i32>(5)? != 0,
    })
}
