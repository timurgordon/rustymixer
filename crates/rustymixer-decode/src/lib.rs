//! Audio file decoding using Symphonia.
//!
//! Provides a unified decoder interface for MP3, FLAC, WAV,
//! Vorbis, AAC, and other formats. All output is interleaved
//! stereo f32 regardless of the source format.

mod symphonia_decoder;

use rustymixer_core::audio::Sample;

/// Position in a track measured in frames.
pub type FramePos = u64;

/// Convenience alias for decode results.
pub type Result<T> = std::result::Result<T, DecodeError>;

/// Errors that can occur during audio decoding.
#[derive(thiserror::Error, Debug)]
pub enum DecodeError {
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("seek error: {0}")]
    Seek(String),
    #[error("end of stream")]
    EndOfStream,
}

/// Metadata and format information for a decoded track.
#[derive(Debug, Clone)]
pub struct TrackInfo {
    /// Sample rate in Hz (e.g. 44100, 48000).
    pub sample_rate: u32,
    /// Number of channels in the source file (before stereo conversion).
    pub channels: u16,
    /// Total duration in frames, if known.
    pub total_frames: Option<u64>,
    /// Track title from file metadata.
    pub title: Option<String>,
    /// Artist from file metadata.
    pub artist: Option<String>,
    /// Album from file metadata.
    pub album: Option<String>,
}

/// Trait for audio file decoders.
///
/// Implementations read encoded audio files and produce interleaved
/// stereo f32 samples regardless of the source format.
pub trait AudioDecoder: Send {
    /// Total duration in frames (`None` if unknown or streaming).
    fn total_frames(&self) -> Option<u64>;

    /// Track metadata and format information.
    fn track_info(&self) -> &TrackInfo;

    /// Read decoded frames into `output`. Returns the number of frames
    /// actually read. Output is always interleaved stereo f32, so
    /// `output` must have room for at least `max_frames * 2` samples.
    fn read_frames(&mut self, output: &mut [Sample], max_frames: usize) -> Result<usize>;

    /// Seek to a frame position. Returns the actual position seeked to.
    fn seek(&mut self, pos: FramePos) -> Result<FramePos>;

    /// Current position in frames from the start of the track.
    fn position(&self) -> FramePos;
}

pub use symphonia_decoder::SymphoniaDecoder;
