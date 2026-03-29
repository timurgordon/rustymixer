//! Integration tests for rustymixer-library.

use std::path::Path;

use rustymixer_library::*;

fn test_db() -> Database {
    Database::open_memory().expect("failed to open in-memory db")
}

fn sample_new_track(added_at: i64) -> NewTrack {
    NewTrack {
        location_id: None,
        title: Some("Test Track".into()),
        artist: Some("Test Artist".into()),
        album: Some("Test Album".into()),
        album_artist: None,
        genre: Some("Electronic".into()),
        year: Some("2025".into()),
        track_number: Some("1".into()),
        comment: None,
        duration_secs: 180.5,
        sample_rate: Some(44100),
        channels: Some(2),
        bitrate: Some(320),
        bpm: Some(128.0),
        key: Some("Am".into()),
        rating: 0,
        replay_gain: None,
        added_at,
        cover_art_hash: None,
    }
}

// ---------------------------------------------------------------------------
// Migration tests
// ---------------------------------------------------------------------------

#[test]
fn migration_creates_tables_on_fresh_db() {
    let db = test_db();
    let version = schema::MigrationManager::version(db.conn()).unwrap();
    assert_eq!(version, 1);
}

#[test]
fn migration_is_idempotent() {
    let db = test_db();
    // Run migrate again — should not fail.
    schema::MigrationManager::migrate(db.conn()).unwrap();
    let version = schema::MigrationManager::version(db.conn()).unwrap();
    assert_eq!(version, 1);
}

// ---------------------------------------------------------------------------
// Settings tests
// ---------------------------------------------------------------------------

#[test]
fn settings_crud() {
    let db = test_db();
    let c = db.conn();

    assert_eq!(SettingsDao::get(c, "foo").unwrap(), None);
    SettingsDao::set(c, "foo", "bar").unwrap();
    assert_eq!(SettingsDao::get(c, "foo").unwrap(), Some("bar".into()));
    SettingsDao::set(c, "foo", "baz").unwrap();
    assert_eq!(SettingsDao::get(c, "foo").unwrap(), Some("baz".into()));
    SettingsDao::delete(c, "foo").unwrap();
    assert_eq!(SettingsDao::get(c, "foo").unwrap(), None);
}

// ---------------------------------------------------------------------------
// Directory tests
// ---------------------------------------------------------------------------

#[test]
fn directory_crud() {
    let db = test_db();
    let c = db.conn();

    let id = DirectoryDao::add(c, "/music").unwrap();
    assert!(id > 0);

    // Adding same path again returns same id.
    let id2 = DirectoryDao::add(c, "/music").unwrap();
    assert_eq!(id, id2);

    let dir = DirectoryDao::get_by_id(c, id).unwrap().unwrap();
    assert_eq!(dir.path, "/music");

    let dirs = DirectoryDao::list(c).unwrap();
    assert_eq!(dirs.len(), 1);

    DirectoryDao::delete(c, id).unwrap();
    assert!(DirectoryDao::get_by_id(c, id).unwrap().is_none());
}

// ---------------------------------------------------------------------------
// Track location tests
// ---------------------------------------------------------------------------

#[test]
fn location_crud() {
    let db = test_db();
    let c = db.conn();

    let dir_id = DirectoryDao::add(c, "/music").unwrap();
    let loc = NewTrackLocation {
        directory_id: dir_id,
        filename: "song.mp3".into(),
        filesize: Some(5_000_000),
        fs_modified_at: Some(1700000000),
    };
    let loc_id = LocationDao::insert(c, &loc).unwrap();
    let fetched = LocationDao::get_by_id(c, loc_id).unwrap().unwrap();
    assert_eq!(fetched.filename, "song.mp3");
    assert!(!fetched.needs_verification);

    let found = LocationDao::find(c, dir_id, "song.mp3").unwrap().unwrap();
    assert_eq!(found.id, loc_id);

    LocationDao::mark_needs_verification(c, loc_id).unwrap();
    let updated = LocationDao::get_by_id(c, loc_id).unwrap().unwrap();
    assert!(updated.needs_verification);
}

// ---------------------------------------------------------------------------
// Track CRUD tests
// ---------------------------------------------------------------------------

#[test]
fn track_insert_and_get() {
    let db = test_db();
    let c = db.conn();

    let new = sample_new_track(1000);
    let id = TrackDao::insert(c, &new).unwrap();
    assert!(id > 0);

    let track = TrackDao::get_by_id(c, id).unwrap().unwrap();
    assert_eq!(track.title.as_deref(), Some("Test Track"));
    assert_eq!(track.artist.as_deref(), Some("Test Artist"));
    assert_eq!(track.bpm, Some(128.0));
    assert!(!track.analyzed);
}

#[test]
fn track_update() {
    let db = test_db();
    let c = db.conn();

    let id = TrackDao::insert(c, &sample_new_track(1000)).unwrap();
    let mut track = TrackDao::get_by_id(c, id).unwrap().unwrap();
    track.title = Some("Updated Title".into());
    track.rating = 5;
    track.analyzed = true;
    TrackDao::update(c, &track).unwrap();

    let fetched = TrackDao::get_by_id(c, id).unwrap().unwrap();
    assert_eq!(fetched.title.as_deref(), Some("Updated Title"));
    assert_eq!(fetched.rating, 5);
    assert!(fetched.analyzed);
}

#[test]
fn track_delete() {
    let db = test_db();
    let c = db.conn();

    let id = TrackDao::insert(c, &sample_new_track(1000)).unwrap();
    TrackDao::delete(c, id).unwrap();
    assert!(TrackDao::get_by_id(c, id).unwrap().is_none());
}

#[test]
fn track_count() {
    let db = test_db();
    let c = db.conn();

    assert_eq!(TrackDao::count(c).unwrap(), 0);
    TrackDao::insert(c, &sample_new_track(1000)).unwrap();
    TrackDao::insert(c, &sample_new_track(2000)).unwrap();
    assert_eq!(TrackDao::count(c).unwrap(), 2);
}

#[test]
fn track_search() {
    let db = test_db();
    let c = db.conn();

    let mut t1 = sample_new_track(1000);
    t1.title = Some("Midnight Train".into());
    t1.artist = Some("DJ Shadow".into());
    TrackDao::insert(c, &t1).unwrap();

    let mut t2 = sample_new_track(2000);
    t2.title = Some("Sunrise Anthem".into());
    t2.artist = Some("Tiesto".into());
    TrackDao::insert(c, &t2).unwrap();

    let results = TrackDao::search(c, "Midnight", 100, 0).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title.as_deref(), Some("Midnight Train"));

    let results = TrackDao::search(c, "DJ", 100, 0).unwrap();
    assert_eq!(results.len(), 1);

    let results = TrackDao::search(c, "nonexistent", 100, 0).unwrap();
    assert!(results.is_empty());
}

#[test]
fn track_all_sorted() {
    let db = test_db();
    let c = db.conn();

    let mut t1 = sample_new_track(1000);
    t1.title = Some("Bravo".into());
    TrackDao::insert(c, &t1).unwrap();

    let mut t2 = sample_new_track(2000);
    t2.title = Some("Alpha".into());
    TrackDao::insert(c, &t2).unwrap();

    let asc = TrackDao::all(c, SortColumn::Title, SortOrder::Asc, 100, 0).unwrap();
    assert_eq!(asc[0].title.as_deref(), Some("Alpha"));
    assert_eq!(asc[1].title.as_deref(), Some("Bravo"));

    let desc = TrackDao::all(c, SortColumn::Title, SortOrder::Desc, 100, 0).unwrap();
    assert_eq!(desc[0].title.as_deref(), Some("Bravo"));
    assert_eq!(desc[1].title.as_deref(), Some("Alpha"));
}

#[test]
fn track_get_by_location() {
    let db = test_db();
    let c = db.conn();

    let dir_id = DirectoryDao::add(c, "/music").unwrap();
    let loc_id = LocationDao::insert(
        c,
        &NewTrackLocation {
            directory_id: dir_id,
            filename: "tune.flac".into(),
            filesize: None,
            fs_modified_at: None,
        },
    )
    .unwrap();

    let mut new = sample_new_track(1000);
    new.location_id = Some(loc_id);
    TrackDao::insert(c, &new).unwrap();

    let found = TrackDao::get_by_location(c, dir_id, "tune.flac")
        .unwrap()
        .unwrap();
    assert_eq!(found.title.as_deref(), Some("Test Track"));
}

// ---------------------------------------------------------------------------
// Cue tests
// ---------------------------------------------------------------------------

#[test]
fn cue_crud() {
    let db = test_db();
    let c = db.conn();

    let track_id = TrackDao::insert(c, &sample_new_track(1000)).unwrap();

    let cue = NewCue {
        track_id,
        cue_type: CueType::HotCue,
        position_frames: 44100.0,
        length_frames: 0.0,
        hotcue_number: Some(0),
        label: Some("Drop".into()),
        color: Some(0xFF0000),
    };
    let cue_id = CueDao::set(c, &cue).unwrap();

    let cues = CueDao::get_for_track(c, track_id).unwrap();
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].id, cue_id);
    assert_eq!(cues[0].cue_type, CueType::HotCue);
    assert_eq!(cues[0].label.as_deref(), Some("Drop"));

    CueDao::delete(c, cue_id).unwrap();
    assert!(CueDao::get_for_track(c, track_id).unwrap().is_empty());
}

#[test]
fn cue_cascade_delete_with_track() {
    let db = test_db();
    let c = db.conn();

    let track_id = TrackDao::insert(c, &sample_new_track(1000)).unwrap();
    CueDao::set(
        c,
        &NewCue {
            track_id,
            cue_type: CueType::Intro,
            position_frames: 0.0,
            length_frames: 0.0,
            hotcue_number: None,
            label: None,
            color: None,
        },
    )
    .unwrap();

    // Deleting the track should cascade-delete its cues.
    TrackDao::delete(c, track_id).unwrap();
    assert!(CueDao::get_for_track(c, track_id).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Playlist tests
// ---------------------------------------------------------------------------

#[test]
fn playlist_crud() {
    let db = test_db();
    let c = db.conn();

    let pl_id = PlaylistDao::create(c, "My Playlist").unwrap();
    let playlists = PlaylistDao::list(c).unwrap();
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].name, "My Playlist");
    assert_eq!(playlists[0].position, 0);

    PlaylistDao::rename(c, pl_id, "Renamed").unwrap();
    let playlists = PlaylistDao::list(c).unwrap();
    assert_eq!(playlists[0].name, "Renamed");

    PlaylistDao::delete(c, pl_id).unwrap();
    assert!(PlaylistDao::list(c).unwrap().is_empty());
}

#[test]
fn playlist_tracks() {
    let db = test_db();
    let c = db.conn();

    let pl_id = PlaylistDao::create(c, "Set").unwrap();
    let t1 = TrackDao::insert(c, &sample_new_track(1000)).unwrap();
    let t2 = TrackDao::insert(c, &sample_new_track(2000)).unwrap();

    PlaylistDao::add_track(c, pl_id, t1).unwrap();
    PlaylistDao::add_track(c, pl_id, t2).unwrap();

    let tracks = PlaylistDao::tracks(c, pl_id).unwrap();
    assert_eq!(tracks.len(), 2);

    PlaylistDao::remove_track(c, pl_id, t1).unwrap();
    let tracks = PlaylistDao::tracks(c, pl_id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, t2);
}

// ---------------------------------------------------------------------------
// Crate tests
// ---------------------------------------------------------------------------

#[test]
fn crate_crud() {
    let db = test_db();
    let c = db.conn();

    let cr_id = CrateDao::create(c, "Favorites").unwrap();
    let crates = CrateDao::list(c).unwrap();
    assert_eq!(crates.len(), 1);
    assert_eq!(crates[0].name, "Favorites");

    CrateDao::rename(c, cr_id, "Top Picks").unwrap();
    let crates = CrateDao::list(c).unwrap();
    assert_eq!(crates[0].name, "Top Picks");

    CrateDao::delete(c, cr_id).unwrap();
    assert!(CrateDao::list(c).unwrap().is_empty());
}

#[test]
fn crate_tracks() {
    let db = test_db();
    let c = db.conn();

    let cr_id = CrateDao::create(c, "Techno").unwrap();
    let t1 = TrackDao::insert(c, &sample_new_track(1000)).unwrap();

    CrateDao::add_track(c, cr_id, t1).unwrap();
    // Adding same track again should be ignored (INSERT OR IGNORE).
    CrateDao::add_track(c, cr_id, t1).unwrap();

    let tracks = CrateDao::tracks(c, cr_id).unwrap();
    assert_eq!(tracks.len(), 1);

    CrateDao::remove_track(c, cr_id, t1).unwrap();
    assert!(CrateDao::tracks(c, cr_id).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Concurrent access (multi-reader)
// ---------------------------------------------------------------------------

#[test]
fn concurrent_readers() {
    // Open a file-based DB so multiple connections can read concurrently.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let db1 = Database::open(&path).unwrap();

    TrackDao::insert(db1.conn(), &sample_new_track(1000)).unwrap();

    // Open a second connection to the same file.
    let db2 = Database::open(&path).unwrap();
    let count = TrackDao::count(db2.conn()).unwrap();
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// MetadataReader tests
// ---------------------------------------------------------------------------

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn metadata_read_wav_properties() {
    let path = fixture_path("silence.wav");
    let meta = MetadataReader::read(&path).unwrap();

    assert_eq!(meta.sample_rate, 44100);
    assert_eq!(meta.channels, 2);
    // ~1 second of audio
    assert!(meta.duration_secs > 0.9 && meta.duration_secs < 1.1);
    // WAV with no tags — all text fields should be None
    assert!(meta.title.is_none());
    assert!(meta.artist.is_none());
    assert!(meta.album.is_none());
    assert!(meta.bpm.is_none());
    assert!(meta.key.is_none());
    assert!(meta.replay_gain.is_none());
}

#[test]
fn metadata_cover_art_none_for_untagged_wav() {
    let path = fixture_path("silence.wav");
    let art = MetadataReader::cover_art(&path).unwrap();
    assert!(art.is_none());
}

#[test]
fn metadata_read_missing_file() {
    let result = MetadataReader::read(Path::new("/tmp/does_not_exist_9999.mp3"));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Scanner tests
// ---------------------------------------------------------------------------

#[test]
fn scanner_scan_directory_with_audio_files() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    // Copy the fixture WAV file into the music directory.
    let fixture = fixture_path("silence.wav");
    std::fs::copy(&fixture, music_dir.join("track1.wav")).unwrap();
    std::fs::copy(&fixture, music_dir.join("track2.wav")).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();

    let scanner = LibraryScanner::new(c);
    let result = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();

    assert_eq!(result.added, 2);
    assert_eq!(result.updated, 0);
    assert_eq!(result.removed, 0);
    assert_eq!(result.errors, 0);
    assert!(result.duration_secs >= 0.0);

    // Verify tracks are in DB.
    assert_eq!(TrackDao::count(c).unwrap(), 2);
}

#[test]
fn scanner_incremental_scan_skips_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    let fixture = fixture_path("silence.wav");
    std::fs::copy(&fixture, music_dir.join("track1.wav")).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();

    let scanner = LibraryScanner::new(c);

    // First scan — should add one track.
    let r1 = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();
    assert_eq!(r1.added, 1);

    // Second scan — nothing changed, so zero added/updated.
    let r2 = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();
    assert_eq!(r2.added, 0);
    assert_eq!(r2.updated, 0);

    // Still only one track in DB.
    assert_eq!(TrackDao::count(c).unwrap(), 1);
}

#[test]
fn scanner_detects_modified_file() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    let fixture = fixture_path("silence.wav");
    let target = music_dir.join("track.wav");
    std::fs::copy(&fixture, &target).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();
    let scanner = LibraryScanner::new(c);

    // First scan.
    scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();

    // Simulate modification by changing mtime stored in DB.
    let loc = LocationDao::find(c, dir_id, "track.wav").unwrap().unwrap();
    let mut loc_modified = loc;
    loc_modified.fs_modified_at = Some(0); // Force mismatch.
    LocationDao::update(c, &loc_modified).unwrap();

    // Second scan should detect the mtime mismatch and re-read.
    let r2 = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();
    assert_eq!(r2.updated, 1);
    assert_eq!(r2.added, 0);
}

#[test]
fn scanner_detects_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    let fixture = fixture_path("silence.wav");
    let target = music_dir.join("track.wav");
    std::fs::copy(&fixture, &target).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();
    let scanner = LibraryScanner::new(c);

    // First scan.
    scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();

    // Delete the file from disk.
    std::fs::remove_file(&target).unwrap();

    // Second scan should detect the missing file.
    let r2 = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();
    assert_eq!(r2.removed, 1);

    // Location should be marked needs_verification.
    let loc = LocationDao::find(c, dir_id, "track.wav").unwrap().unwrap();
    assert!(loc.needs_verification);
}

#[test]
fn scanner_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();

    let scanner = LibraryScanner::new(c);
    let result = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();

    assert_eq!(result.added, 0);
    assert_eq!(result.updated, 0);
    assert_eq!(result.removed, 0);
    assert_eq!(result.errors, 0);
    assert_eq!(TrackDao::count(c).unwrap(), 0);
}

#[test]
fn scanner_progress_reporting() {
    use std::cell::RefCell;

    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    let fixture = fixture_path("silence.wav");
    std::fs::copy(&fixture, music_dir.join("a.wav")).unwrap();
    std::fs::copy(&fixture, music_dir.join("b.wav")).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();
    let scanner = LibraryScanner::new(c);

    let progress_updates = RefCell::new(Vec::new());
    scanner
        .scan_directory(&music_dir, dir_id, &|p| {
            progress_updates.borrow_mut().push(p);
        })
        .unwrap();
    let progress_updates = progress_updates.into_inner();

    // Should have at least: 1 Discovering + 2 Reading + 2 Verifying.
    assert!(progress_updates.len() >= 5);

    // First update should be Discovering phase.
    assert_eq!(progress_updates[0].phase, ScanPhase::Discovering);

    // Should have Reading phases.
    let reading = progress_updates
        .iter()
        .filter(|p| p.phase == ScanPhase::Reading)
        .count();
    assert_eq!(reading, 2);

    // Should have Verifying phases.
    let verifying = progress_updates
        .iter()
        .filter(|p| p.phase == ScanPhase::Verifying)
        .count();
    assert_eq!(verifying, 2);
}

#[test]
fn scanner_scan_all_uses_registered_directories() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    let fixture = fixture_path("silence.wav");
    std::fs::copy(&fixture, music_dir.join("track.wav")).unwrap();

    let db = test_db();
    let c = db.conn();
    DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();

    let scanner = LibraryScanner::new(c);
    let result = scanner.scan_all(|_| {}).unwrap();

    assert_eq!(result.added, 1);
    assert_eq!(TrackDao::count(c).unwrap(), 1);
}

#[test]
fn scanner_ignores_non_audio_files() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    // Create non-audio files.
    std::fs::write(music_dir.join("readme.txt"), "hello").unwrap();
    std::fs::write(music_dir.join("cover.jpg"), &[0xFF, 0xD8]).unwrap();

    // Also copy a real audio file.
    let fixture = fixture_path("silence.wav");
    std::fs::copy(&fixture, music_dir.join("track.wav")).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();

    let scanner = LibraryScanner::new(c);
    let result = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();

    // Only the WAV file should be added.
    assert_eq!(result.added, 1);
    assert_eq!(TrackDao::count(c).unwrap(), 1);
}

#[test]
fn scanner_recursive_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    let music_dir = dir.path().join("music");
    let subdir = music_dir.join("album");
    std::fs::create_dir_all(&subdir).unwrap();

    let fixture = fixture_path("silence.wav");
    std::fs::copy(&fixture, music_dir.join("track1.wav")).unwrap();
    std::fs::copy(&fixture, subdir.join("track2.wav")).unwrap();

    let db = test_db();
    let c = db.conn();
    let dir_id = DirectoryDao::add(c, &music_dir.to_string_lossy()).unwrap();

    let scanner = LibraryScanner::new(c);
    let result = scanner
        .scan_directory(&music_dir, dir_id, &|_| {})
        .unwrap();

    // Both tracks (root and subdirectory) should be added.
    assert_eq!(result.added, 2);
    assert_eq!(TrackDao::count(c).unwrap(), 2);
}

#[test]
fn spawn_scan_background_thread() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let music_dir = dir.path().join("music");
    std::fs::create_dir(&music_dir).unwrap();

    let fixture = fixture_path("silence.wav");
    std::fs::copy(&fixture, music_dir.join("track.wav")).unwrap();

    // Ensure the database is initialized.
    {
        let _db = Database::open(&db_path).unwrap();
    }

    let handle = spawn_scan(db_path.clone(), vec![music_dir]);

    // Drain progress updates.
    let mut progress_count = 0;
    loop {
        match handle.progress().try_recv() {
            Ok(_) => progress_count += 1,
            Err(crossbeam::channel::TryRecvError::Empty) => {
                // Check if thread is still running.
                if handle.is_finished() {
                    // Drain remaining.
                    while handle.progress().try_recv().is_ok() {
                        progress_count += 1;
                    }
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(crossbeam::channel::TryRecvError::Disconnected) => break,
        }
    }

    let result = handle.join().unwrap();
    assert_eq!(result.added, 1);
    assert!(progress_count > 0);

    // Verify the track was actually persisted.
    let db = Database::open(&db_path).unwrap();
    assert_eq!(TrackDao::count(db.conn()).unwrap(), 1);
}
