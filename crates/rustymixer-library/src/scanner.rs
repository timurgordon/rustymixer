//! Background library scanner — recursively scans music directories,
//! reads metadata, and inserts/updates tracks in the database.
//! Supports incremental scanning (only processes new/modified files).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crossbeam::channel::{self, Receiver};
use rusqlite::Connection;
use tracing::{debug, info, warn};

use crate::dao::{DirectoryDao, LocationDao, TrackDao};
use crate::error::Result;
use crate::metadata::MetadataReader;
use crate::models::{NewTrack, NewTrackLocation};

/// Audio file extensions the scanner recognises.
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "aiff", "wv",
];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Phase of a scan operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanPhase {
    /// Walking directory tree to discover audio files.
    Discovering,
    /// Reading metadata from discovered files.
    Reading,
    /// Checking for files removed from disk.
    Verifying,
}

/// Progress update emitted during a scan.
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub current: usize,
    pub total: usize,
    pub current_file: Option<String>,
}

/// Summary returned when a scan completes.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub errors: usize,
    pub duration_secs: f64,
}

/// Handle returned by [`spawn_scan`] to monitor a background scan.
pub struct ScanHandle {
    handle: std::thread::JoinHandle<Result<ScanResult>>,
    progress_rx: Receiver<ScanProgress>,
}

impl ScanHandle {
    /// Non-blocking receiver for progress updates.
    pub fn progress(&self) -> &Receiver<ScanProgress> {
        &self.progress_rx
    }

    /// Check if the background scan thread has finished.
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Block until the scan thread finishes and return its result.
    pub fn join(self) -> Result<ScanResult> {
        self.handle.join().expect("scanner thread panicked")
    }
}

// ---------------------------------------------------------------------------
// LibraryScanner
// ---------------------------------------------------------------------------

/// Scans music directories and populates the database.
pub struct LibraryScanner<'a> {
    conn: &'a Connection,
}

impl<'a> LibraryScanner<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Scan all directories registered in the database.
    pub fn scan_all(&self, progress: impl Fn(ScanProgress)) -> Result<ScanResult> {
        let dirs = DirectoryDao::list(self.conn)?;
        let mut combined = ScanResult {
            added: 0,
            updated: 0,
            removed: 0,
            errors: 0,
            duration_secs: 0.0,
        };
        for dir in &dirs {
            let result = self.scan_directory(Path::new(&dir.path), dir.id, &progress)?;
            combined.added += result.added;
            combined.updated += result.updated;
            combined.removed += result.removed;
            combined.errors += result.errors;
            combined.duration_secs += result.duration_secs;
        }
        Ok(combined)
    }

    /// Scan a single directory (must already be registered with [`DirectoryDao`]).
    pub fn scan_directory(
        &self,
        path: &Path,
        dir_id: i64,
        progress: &impl Fn(ScanProgress),
    ) -> Result<ScanResult> {
        let start = Instant::now();
        let mut added = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;
        let mut errors = 0usize;

        // --- Phase 1: Discover audio files ---
        progress(ScanProgress {
            phase: ScanPhase::Discovering,
            current: 0,
            total: 0,
            current_file: None,
        });

        let audio_files = discover_audio_files(path);
        let total = audio_files.len();

        info!(directory = %path.display(), files = total, "discovery complete");

        // --- Phase 2: Read metadata & insert/update ---
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Track which relative filenames we saw on disk so we can detect removals.
        let mut seen_filenames: HashSet<String> = HashSet::with_capacity(total);

        for (i, file_path) in audio_files.iter().enumerate() {
            let relative = match file_path.strip_prefix(path) {
                Ok(r) => r.to_string_lossy().to_string(),
                Err(_) => {
                    warn!(path = %file_path.display(), "cannot compute relative path, skipping");
                    errors += 1;
                    continue;
                }
            };

            progress(ScanProgress {
                phase: ScanPhase::Reading,
                current: i + 1,
                total,
                current_file: Some(relative.clone()),
            });

            seen_filenames.insert(relative.clone());

            // Get file system metadata (size, mtime).
            let fs_meta = match std::fs::metadata(file_path) {
                Ok(m) => m,
                Err(e) => {
                    warn!(path = %file_path.display(), error = %e, "cannot stat file");
                    errors += 1;
                    continue;
                }
            };
            let fs_modified = fs_meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let filesize = Some(fs_meta.len() as i64);

            // Check if already in DB.
            let existing_loc = LocationDao::find(self.conn, dir_id, &relative)?;

            match existing_loc {
                Some(loc) if loc.fs_modified_at == fs_modified => {
                    // Unchanged — skip.
                    debug!(file = %relative, "unchanged, skipping");
                }
                Some(mut loc) => {
                    // File modified — re-read metadata and update.
                    debug!(file = %relative, "modified, updating");
                    match MetadataReader::read(file_path) {
                        Ok(meta) => {
                            // Update location.
                            loc.filesize = filesize;
                            loc.fs_modified_at = fs_modified;
                            loc.needs_verification = false;
                            LocationDao::update(self.conn, &loc)?;

                            // Update track metadata.
                            if let Some(mut track) =
                                TrackDao::get_by_location(self.conn, dir_id, &relative)?
                            {
                                track.title = meta.title;
                                track.artist = meta.artist;
                                track.album = meta.album;
                                track.album_artist = meta.album_artist;
                                track.genre = meta.genre;
                                track.year = meta.year;
                                track.track_number = meta.track_number;
                                track.comment = meta.comment;
                                track.duration_secs = meta.duration_secs;
                                track.sample_rate = Some(meta.sample_rate as i32);
                                track.channels = Some(meta.channels as i32);
                                track.bitrate = meta.bitrate.map(|b| b as i32);
                                track.bpm = meta.bpm;
                                track.key = meta.key;
                                track.replay_gain = meta.replay_gain;
                                TrackDao::update(self.conn, &track)?;
                            }
                            updated += 1;
                        }
                        Err(e) => {
                            warn!(file = %relative, error = %e, "metadata read failed on update");
                            errors += 1;
                        }
                    }
                }
                None => {
                    // New file — read metadata and insert.
                    debug!(file = %relative, "new file, inserting");
                    match MetadataReader::read(file_path) {
                        Ok(meta) => {
                            let loc_id = LocationDao::insert(
                                self.conn,
                                &NewTrackLocation {
                                    directory_id: dir_id,
                                    filename: relative.clone(),
                                    filesize,
                                    fs_modified_at: fs_modified,
                                },
                            )?;

                            let new_track = NewTrack {
                                location_id: Some(loc_id),
                                title: meta.title,
                                artist: meta.artist,
                                album: meta.album,
                                album_artist: meta.album_artist,
                                genre: meta.genre,
                                year: meta.year,
                                track_number: meta.track_number,
                                comment: meta.comment,
                                duration_secs: meta.duration_secs,
                                sample_rate: Some(meta.sample_rate as i32),
                                channels: Some(meta.channels as i32),
                                bitrate: meta.bitrate.map(|b| b as i32),
                                bpm: meta.bpm,
                                key: meta.key,
                                rating: 0,
                                replay_gain: meta.replay_gain,
                                added_at: now_ts,
                                cover_art_hash: None,
                            };
                            TrackDao::insert(self.conn, &new_track)?;
                            added += 1;
                        }
                        Err(e) => {
                            warn!(file = %relative, error = %e, "metadata read failed on insert");
                            errors += 1;
                        }
                    }
                }
            }
        }

        // --- Phase 3: Verify — mark missing files ---
        let db_locations = LocationDao::list_by_directory(self.conn, dir_id)?;
        let verify_total = db_locations.len();

        for (i, loc) in db_locations.iter().enumerate() {
            progress(ScanProgress {
                phase: ScanPhase::Verifying,
                current: i + 1,
                total: verify_total,
                current_file: Some(loc.filename.clone()),
            });

            if !seen_filenames.contains(&loc.filename) {
                // File is in DB but was not found on disk.
                let full_path = path.join(&loc.filename);
                if !full_path.exists() {
                    debug!(file = %loc.filename, "marking as missing");
                    LocationDao::mark_needs_verification(self.conn, loc.id)?;
                    removed += 1;
                }
            }
        }

        let duration_secs = start.elapsed().as_secs_f64();

        info!(
            directory = %path.display(),
            added, updated, removed, errors, duration_secs,
            "scan complete"
        );

        Ok(ScanResult {
            added,
            updated,
            removed,
            errors,
            duration_secs,
        })
    }
}

// ---------------------------------------------------------------------------
// Background scanning
// ---------------------------------------------------------------------------

/// Spawn a scan of the given directories in a background thread.
///
/// Each directory is registered via [`DirectoryDao::add`] if not already present.
/// Returns a [`ScanHandle`] to monitor progress and collect the result.
pub fn spawn_scan(db_path: PathBuf, directories: Vec<PathBuf>) -> ScanHandle {
    let (progress_tx, progress_rx) = channel::bounded(100);

    let handle = std::thread::spawn(move || {
        let db = crate::db::Database::open(&db_path)?;
        let conn = db.conn();
        let scanner = LibraryScanner::new(conn);
        let mut combined = ScanResult {
            added: 0,
            updated: 0,
            removed: 0,
            errors: 0,
            duration_secs: 0.0,
        };

        for dir in &directories {
            let dir_str = dir.to_string_lossy();
            let dir_id = DirectoryDao::add(conn, &dir_str)?;

            let result = scanner.scan_directory(dir, dir_id, &|p| {
                let _ = progress_tx.try_send(p);
            })?;

            combined.added += result.added;
            combined.updated += result.updated;
            combined.removed += result.removed;
            combined.errors += result.errors;
            combined.duration_secs += result.duration_secs;
        }

        Ok(combined)
    });

    ScanHandle {
        handle,
        progress_rx,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Recursively discover all audio files under `root`.
fn discover_audio_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_dir(root, &mut files);
    files.sort();
    files
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(dir = %dir.display(), error = %e, "cannot read directory");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, out);
        } else if is_audio_file(&path) {
            out.push(path);
        }
    }
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_audio_file_accepts_known_extensions() {
        assert!(is_audio_file(Path::new("track.mp3")));
        assert!(is_audio_file(Path::new("track.FLAC")));
        assert!(is_audio_file(Path::new("song.wav")));
        assert!(is_audio_file(Path::new("song.m4a")));
        assert!(is_audio_file(Path::new("song.ogg")));
        assert!(is_audio_file(Path::new("deep/path/song.aiff")));
    }

    #[test]
    fn is_audio_file_rejects_non_audio() {
        assert!(!is_audio_file(Path::new("readme.txt")));
        assert!(!is_audio_file(Path::new("cover.jpg")));
        assert!(!is_audio_file(Path::new("no_ext")));
    }
}
