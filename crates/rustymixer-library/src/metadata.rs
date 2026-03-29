//! Audio file metadata extraction using the `lofty` crate.
//!
//! Reads ID3v2 (MP3), Vorbis Comments (FLAC, OGG), MP4 atoms (M4A/AAC),
//! and other tag formats to extract title, artist, album, BPM, key,
//! duration, genre, year, and embedded cover art.

use std::path::Path;

use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::prelude::Accessor;
use lofty::tag::ItemKey;

use crate::error::Result;

/// Extracted metadata from an audio file.
#[derive(Debug, Clone)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub track_number: Option<String>,
    pub comment: Option<String>,
    pub duration_secs: f64,
    pub sample_rate: u32,
    pub channels: u8,
    pub bitrate: Option<u32>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub replay_gain: Option<f64>,
}

/// Embedded cover art extracted from an audio file.
#[derive(Debug, Clone)]
pub struct CoverArt {
    pub data: Vec<u8>,
    pub mime_type: String,
}

/// Reads metadata and cover art from audio files.
pub struct MetadataReader;

impl MetadataReader {
    /// Read metadata from an audio file at `path`.
    pub fn read(path: &Path) -> Result<TrackMetadata> {
        let tagged_file = lofty::read_from_path(path)?;
        let properties = tagged_file.properties();
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let title = tag.and_then(|t| t.title().map(|s| s.to_string()));
        let artist = tag.and_then(|t| t.artist().map(|s| s.to_string()));
        let album = tag.and_then(|t| t.album().map(|s| s.to_string()));
        let genre = tag.and_then(|t| t.genre().map(|s| s.to_string()));
        let comment = tag.and_then(|t| t.comment().map(|s| s.to_string()));

        let year = tag.and_then(|t| t.year().map(|y| y.to_string()));

        let track_number = tag.and_then(|t| t.track().map(|n| n.to_string()));

        let album_artist = tag.and_then(|t| {
            t.get_string(&ItemKey::AlbumArtist)
                .map(|s| s.to_string())
        });

        let bpm = tag.and_then(|t| {
            // Try the string BPM key first (fractional values), then integer BPM.
            t.get_string(&ItemKey::Bpm)
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| {
                    t.get_string(&ItemKey::IntegerBpm)
                        .and_then(|s| s.parse::<f64>().ok())
                })
        });

        let key = tag.and_then(|t| {
            t.get_string(&ItemKey::InitialKey)
                .map(|s| s.to_string())
        });

        let replay_gain = tag.and_then(|t| {
            t.get_string(&ItemKey::ReplayGainTrackGain)
                .and_then(parse_replay_gain)
        });

        let duration_secs = properties.duration().as_secs_f64();
        let sample_rate = properties.sample_rate().unwrap_or(44100);
        let channels = properties.channels().unwrap_or(2);
        let bitrate = properties.audio_bitrate().or(properties.overall_bitrate());

        Ok(TrackMetadata {
            title,
            artist,
            album,
            album_artist,
            genre,
            year,
            track_number,
            comment,
            duration_secs,
            sample_rate,
            channels,
            bitrate,
            bpm,
            key,
            replay_gain,
        })
    }

    /// Extract the first embedded cover art from an audio file.
    pub fn cover_art(path: &Path) -> Result<Option<CoverArt>> {
        let tagged_file = lofty::read_from_path(path)?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let art = tag.and_then(|t| {
            let pictures = t.pictures();
            pictures.first().map(|pic| CoverArt {
                data: pic.data().to_vec(),
                mime_type: pic
                    .mime_type()
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| "image/jpeg".to_string()),
            })
        });

        Ok(art)
    }
}

/// Parse a replay gain string like "+1.23 dB" or "-0.5 dB" into an f64 value.
fn parse_replay_gain(s: &str) -> Option<f64> {
    let trimmed = s.trim();
    let without_db = trimmed
        .strip_suffix("dB")
        .or_else(|| trimmed.strip_suffix("db"))
        .unwrap_or(trimmed)
        .trim();
    without_db.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_replay_gain_values() {
        assert_eq!(parse_replay_gain("+1.23 dB"), Some(1.23));
        assert_eq!(parse_replay_gain("-0.5 dB"), Some(-0.5));
        assert_eq!(parse_replay_gain("3.0"), Some(3.0));
        assert_eq!(parse_replay_gain("  -2.1 db "), Some(-2.1));
        assert_eq!(parse_replay_gain("not a number"), None);
    }

    #[test]
    fn read_nonexistent_file_returns_error() {
        let result = MetadataReader::read(Path::new("/nonexistent/audio.mp3"));
        assert!(result.is_err());
    }
}
