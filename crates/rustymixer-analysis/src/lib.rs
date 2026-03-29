//! Audio analysis for RustyMixer.
//!
//! BPM/beat detection, key detection, waveform generation,
//! and loudness analysis.

pub mod waveform;

pub use waveform::{BandData, WaveformAnalyzer, WaveformData, WaveformPoint, WaveformResolution};

/// Errors that can occur during audio analysis.
#[derive(thiserror::Error, Debug)]
pub enum AnalysisError {
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
    #[error("track duration unknown — cannot generate waveform")]
    UnknownDuration,
    #[error("decode error: {0}")]
    Decode(String),
    #[error("internal error: {0}")]
    Internal(String),
}
