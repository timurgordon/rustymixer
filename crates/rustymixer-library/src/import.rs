//\! Playlist import — M3U and PLS file parsers.

use std::io::BufRead;
use std::path::Path;

use rusqlite::{Connection, params};
use tracing::debug;

use crate::error::Result;
use crate::models::ImportResult;
use crate::dao::playlist::PlaylistDao;

/// Import an M3U playlist file into the library.
///
/// Reads line by line, skips comments (`#`), resolves relative paths against
/// the M3U file's parent directory, and matches each path to library tracks
/// via the `track_locations` + `directories` tables.
pub fn import_m3u(conn: &Connection, path: &Path, playlist_name: &str) -> Result<ImportResult> {
    let base_dir = path.parent().unwrap_or(Path::new("."));
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    let playlist_id = PlaylistDao::create(conn, playlist_name)?;
    let mut imported = 0usize;
    let mut not_found = 0usize;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        // Skip empty lines and comments.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let file_path = if Path::new(line).is_absolute() {
            std::path::PathBuf::from(line)
        } else {
            base_dir.join(line)
        };

        // Canonicalize to resolve ../ and symlinks, fall back to the
        // joined path if canonicalize fails (file might not exist).
        let file_path = file_path.canonicalize().unwrap_or(file_path);

        match find_track_by_path(conn, &file_path)? {
            Some(track_id) => {
                PlaylistDao::add_track(conn, playlist_id, track_id)?;
                imported += 1;
            }
            None => {
                debug!(?file_path, "M3U entry not found in library");
                not_found += 1;
            }
        }
    }

    Ok(ImportResult { imported, not_found })
}

/// Import a PLS playlist file into the library.
///
/// PLS files use INI-like format with `FileN=path` entries.
pub fn import_pls(conn: &Connection, path: &Path, playlist_name: &str) -> Result<ImportResult> {
    let base_dir = path.parent().unwrap_or(Path::new("."));
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    let playlist_id = PlaylistDao::create(conn, playlist_name)?;
    let mut imported = 0usize;
    let mut not_found = 0usize;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        // PLS entries look like: File1=/path/to/song.mp3
        let entry = match line.strip_prefix("File") {
            Some(rest) => {
                // Skip the number and '='
                rest.split_once('=').map(|(_, path)| path)
            }
            None => None,
        };

        let Some(entry) = entry else {
            continue;
        };

        let file_path = if Path::new(entry).is_absolute() {
            std::path::PathBuf::from(entry)
        } else {
            base_dir.join(entry)
        };

        let file_path = file_path.canonicalize().unwrap_or(file_path);

        match find_track_by_path(conn, &file_path)? {
            Some(track_id) => {
                PlaylistDao::add_track(conn, playlist_id, track_id)?;
                imported += 1;
            }
            None => {
                debug!(?file_path, "PLS entry not found in library");
                not_found += 1;
            }
        }
    }

    Ok(ImportResult { imported, not_found })
}

/// Look up a track by its full file path, joining directories and
/// track_locations tables.
fn find_track_by_path(conn: &Connection, path: &Path) -> Result<Option<i64>> {
    let path_str = path.to_string_lossy();

    // Match against `directories.path || '/' || track_locations.filename`.
    let result: std::result::Result<i64, _> = conn.query_row(
        "SELECT t.id
         FROM tracks t
         JOIN track_locations tl ON t.location_id = tl.id
         JOIN directories d ON tl.directory_id = d.id
         WHERE d.path || '/' || tl.filename = ?1",
        params![path_str],
        |row| row.get(0),
    );

    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
