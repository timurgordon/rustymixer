//! Directory DAO — music directories to scan.

use rusqlite::Connection;

use crate::error::Result;
use crate::models::Directory;

pub struct DirectoryDao;

impl DirectoryDao {
    pub fn add(conn: &Connection, path: &str) -> Result<i64> {
        conn.execute(
            "INSERT OR IGNORE INTO directories (path) VALUES (?1)",
            [path],
        )?;
        let id = conn.query_row(
            "SELECT id FROM directories WHERE path = ?1",
            [path],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<Directory>> {
        let mut stmt = conn.prepare("SELECT id, path FROM directories WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(Directory {
                id: row.get(0)?,
                path: row.get(1)?,
            })),
            None => Ok(None),
        }
    }

    pub fn list(conn: &Connection) -> Result<Vec<Directory>> {
        let mut stmt = conn.prepare("SELECT id, path FROM directories ORDER BY path")?;
        let rows = stmt.query_map([], |row| {
            Ok(Directory {
                id: row.get(0)?,
                path: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM directories WHERE id = ?1", [id])?;
        Ok(())
    }
}
