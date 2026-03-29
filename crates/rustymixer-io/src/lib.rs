//! Audio I/O backends for RustyMixer.
//!
//! Desktop: cpal-based audio output.
//! WASM: WebAudio API via ScriptProcessorNode.

use rustymixer_core::audio::{ChannelCount, SampleRate};

#[cfg(not(target_arch = "wasm32"))]
mod cpal_output;

#[cfg(not(target_arch = "wasm32"))]
pub use cpal_output::CpalOutput;

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
pub use web::WebAudioOutput;

/// Audio output configuration.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate: SampleRate,
    pub channels: ChannelCount,
    /// Buffer size in frames per callback.
    pub buffer_frames: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: SampleRate::default(),
            channels: ChannelCount::STEREO,
            buffer_frames: 2048,
        }
    }
}

/// Errors from audio I/O operations.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("failed to create audio context: {0}")]
    ContextCreation(String),
    #[error("audio backend error: {0}")]
    Backend(String),
    #[error("audio context is in suspended state — user gesture required")]
    Suspended,
    #[error("unsupported configuration: {0}")]
    UnsupportedConfig(String),
}

/// Trait for audio output backends.
///
/// Both the cpal (desktop) and WebAudio (WASM) backends implement this trait.
pub trait AudioOutput {
    /// Start playback. After this call, the backend will begin pulling samples
    /// from the ring buffer and sending them to the audio device.
    fn start(&mut self) -> Result<(), AudioError>;

    /// Stop playback and disconnect from the audio device.
    fn stop(&mut self) -> Result<(), AudioError>;

    /// Write interleaved f32 samples into the output ring buffer.
    /// Returns the number of samples actually written (may be less than
    /// `samples.len()` if the buffer is full).
    fn write(&mut self, samples: &[f32]) -> usize;

    /// Returns the audio configuration this backend was opened with.
    fn config(&self) -> &AudioConfig;

    /// Returns true if the backend is currently playing.
    fn is_playing(&self) -> bool;
}
