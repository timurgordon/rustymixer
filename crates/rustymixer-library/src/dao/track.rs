//! Track DAO — CRUD, search, and listing for tracks.

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::models::{NewTrack, SortColumn, SortOrder, Track};

pub struct TrackDao;

impl TrackDao {
    pub fn insert(conn: &Connection, t: &NewTrack) -> Result<i64> {
        conn.execute(
            "INSERT INTO tracks (
                location_id, title, artist, album, album_artist, genre,
                year, track_number, comment, duration_secs, sample_rate,
                channels, bitrate, bpm, key, rating, replay_gain, added_at,
                cover_art_hash
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                ?19
            )",
            params![
                t.location_id,
                t.title,
                t.artist,
                t.album,
                t.album_artist,
                t.genre,
                t.year,
                t.track_number,
                t.comment,
                t.duration_secs,
                t.sample_rate,
                t.channels,
                t.bitrate,
                t.bpm,
                t.key,
                t.rating,
                t.replay_gain,
                t.added_at,
                t.cover_art_hash,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<Track>> {
        let mut stmt = conn.prepare(
            "SELECT id, location_id, title, artist, album, album_artist, genre,
                    year, track_number, comment, duration_secs, sample_rate,
                    channels, bitrate, bpm, key, rating, play_count,
                    last_played_at, replay_gain, added_at, cover_art_hash, analyzed
             FROM tracks WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_track(row)?)),
            None => Ok(None),
        }
    }

    pub fn get_by_location(
        conn: &Connection,
        dir_id: i64,
        filename: &str,
    ) -> Result<Option<Track>> {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.location_id, t.title, t.artist, t.album, t.album_artist,
                    t.genre, t.year, t.track_number, t.comment, t.duration_secs,
                    t.sample_rate, t.channels, t.bitrate, t.bpm, t.key, t.rating,
                    t.play_count, t.last_played_at, t.replay_gain, t.added_at,
                    t.cover_art_hash, t.analyzed
             FROM tracks t
             JOIN track_locations tl ON t.location_id = tl.id
             WHERE tl.directory_id = ?1 AND tl.filename = ?2",
        )?;
        let mut rows = stmt.query(params![dir_id, filename])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_track(row)?)),
            None => Ok(None),
        }
    }

    pub fn update(conn: &Connection, t: &Track) -> Result<()> {
        conn.execute(
            "UPDATE tracks SET
                location_id = ?1, title = ?2, artist = ?3, album = ?4,
                album_artist = ?5, genre = ?6, year = ?7, track_number = ?8,
                comment = ?9, duration_secs = ?10, sample_rate = ?11,
                channels = ?12, bitrate = ?13, bpm = ?14, key = ?15,
                rating = ?16, play_count = ?17, last_played_at = ?18,
                replay_gain = ?19, added_at = ?20, cover_art_hash = ?21,
                analyzed = ?22
             WHERE id = ?23",
            params![
                t.location_id,
                t.title,
                t.artist,
                t.album,
                t.album_artist,
                t.genre,
                t.year,
                t.track_number,
                t.comment,
                t.duration_secs,
                t.sample_rate,
                t.channels,
                t.bitrate,
                t.bpm,
                t.key,
                t.rating,
                t.play_count,
                t.last_played_at,
                t.replay_gain,
                t.added_at,
                t.cover_art_hash,
                t.analyzed,
                t.id,
            ],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM tracks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn search(
        conn: &Connection,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Track>> {
        let pattern = format!("%{query}%");
        let mut stmt = conn.prepare(
            "SELECT id, location_id, title, artist, album, album_artist, genre,
                    year, track_number, comment, duration_secs, sample_rate,
                    channels, bitrate, bpm, key, rating, play_count,
                    last_played_at, replay_gain, added_at, cover_art_hash, analyzed
             FROM tracks
             WHERE title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1
             ORDER BY artist, title
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows =
            stmt.query_map(params![pattern, limit as i64, offset as i64], row_to_track)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn all(
        conn: &Connection,
        sort: SortColumn,
        order: SortOrder,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Track>> {
        // sort column and order are from a closed enum, safe to interpolate
        let sql = format!(
            "SELECT id, location_id, title, artist, album, album_artist, genre,
                    year, track_number, comment, duration_secs, sample_rate,
                    channels, bitrate, bpm, key, rating, play_count,
                    last_played_at, replay_gain, added_at, cover_art_hash, analyzed
             FROM tracks
             ORDER BY {} {}
             LIMIT ?1 OFFSET ?2",
            sort.as_sql(),
            order.as_sql(),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows =
            stmt.query_map(params![limit as i64, offset as i64], row_to_track)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn count(conn: &Connection) -> Result<usize> {
        let c: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;
        Ok(c as usize)
    }
}

fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(0)?,
        location_id: row.get(1)?,
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        album_artist: row.get(5)?,
        genre: row.get(6)?,
        year: row.get(7)?,
        track_number: row.get(8)?,
        comment: row.get(9)?,
        duration_secs: row.get(10)?,
        sample_rate: row.get(11)?,
        channels: row.get(12)?,
        bitrate: row.get(13)?,
        bpm: row.get(14)?,
        key: row.get(15)?,
        rating: row.get(16)?,
        play_count: row.get(17)?,
        last_played_at: row.get(18)?,
        replay_gain: row.get(19)?,
        added_at: row.get(20)?,
        cover_art_hash: row.get(21)?,
        analyzed: row.get::<_, i32>(22)? != 0,
    })
}
