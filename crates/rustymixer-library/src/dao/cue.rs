//! Cue-point DAO.

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::models::{Cue, CueType, NewCue};

pub struct CueDao;

impl CueDao {
    /// Insert or update a cue. Returns the cue id.
    pub fn set(conn: &Connection, cue: &NewCue) -> Result<i64> {
        conn.execute(
            "INSERT INTO cues (track_id, type, position_frames, length_frames, hotcue_number, label, color)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                cue.track_id,
                cue.cue_type as i32,
                cue.position_frames,
                cue.length_frames,
                cue.hotcue_number,
                cue.label,
                cue.color,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_for_track(conn: &Connection, track_id: i64) -> Result<Vec<Cue>> {
        let mut stmt = conn.prepare(
            "SELECT id, track_id, type, position_frames, length_frames, hotcue_number, label, color
             FROM cues WHERE track_id = ?1 ORDER BY position_frames",
        )?;
        let rows = stmt.query_map([track_id], |row| {
            let type_val: i32 = row.get(2)?;
            Ok(Cue {
                id: row.get(0)?,
                track_id: row.get(1)?,
                cue_type: CueType::from_i32(type_val).unwrap_or(CueType::HotCue),
                position_frames: row.get(3)?,
                length_frames: row.get(4)?,
                hotcue_number: row.get(5)?,
                label: row.get(6)?,
                color: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM cues WHERE id = ?1", [id])?;
        Ok(())
    }
}
