//! Domain model structs for the music library.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Track
// ---------------------------------------------------------------------------

/// A fully-persisted track row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub location_id: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub track_number: Option<String>,
    pub comment: Option<String>,
    pub duration_secs: f64,
    pub sample_rate: Option<i32>,
    pub channels: Option<i32>,
    pub bitrate: Option<i32>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub rating: i32,
    pub play_count: i32,
    pub last_played_at: Option<i64>,
    pub replay_gain: Option<f64>,
    pub added_at: i64,
    pub cover_art_hash: Option<String>,
    pub analyzed: bool,
}

/// Data required to insert a new track (no `id` yet).
#[derive(Debug, Clone)]
pub struct NewTrack {
    pub location_id: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub track_number: Option<String>,
    pub comment: Option<String>,
    pub duration_secs: f64,
    pub sample_rate: Option<i32>,
    pub channels: Option<i32>,
    pub bitrate: Option<i32>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub rating: i32,
    pub replay_gain: Option<f64>,
    pub added_at: i64,
    pub cover_art_hash: Option<String>,
}

// ---------------------------------------------------------------------------
// TrackLocation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackLocation {
    pub id: i64,
    pub directory_id: i64,
    pub filename: String,
    pub filesize: Option<i64>,
    pub fs_modified_at: Option<i64>,
    pub needs_verification: bool,
}

#[derive(Debug, Clone)]
pub struct NewTrackLocation {
    pub directory_id: i64,
    pub filename: String,
    pub filesize: Option<i64>,
    pub fs_modified_at: Option<i64>,
}

// ---------------------------------------------------------------------------
// Directory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directory {
    pub id: i64,
    pub path: String,
}

// ---------------------------------------------------------------------------
// Cue
// ---------------------------------------------------------------------------

/// Type of cue point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum CueType {
    HotCue = 0,
    Intro = 1,
    Outro = 2,
    Loop = 3,
}

impl CueType {
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::HotCue),
            1 => Some(Self::Intro),
            2 => Some(Self::Outro),
            3 => Some(Self::Loop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cue {
    pub id: i64,
    pub track_id: i64,
    pub cue_type: CueType,
    pub position_frames: f64,
    pub length_frames: f64,
    pub hotcue_number: Option<i32>,
    pub label: Option<String>,
    pub color: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct NewCue {
    pub track_id: i64,
    pub cue_type: CueType,
    pub position_frames: f64,
    pub length_frames: f64,
    pub hotcue_number: Option<i32>,
    pub label: Option<String>,
    pub color: Option<i32>,
}

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub position: i32,
    pub created_at: i64,
    pub is_locked: bool,
}

// ---------------------------------------------------------------------------
// Crate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crate {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

/// Columns available for sorting track listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Title,
    Artist,
    Album,
    Duration,
    Bpm,
    Key,
    Rating,
    AddedAt,
}

impl SortColumn {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Duration => "duration_secs",
            Self::Bpm => "bpm",
            Self::Key => "key",
            Self::Rating => "rating",
            Self::AddedAt => "added_at",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}
