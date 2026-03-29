//! Fundamental audio types.
//!
//! These types are used throughout the entire RustyMixer codebase.
//! Inspired by Mixxx's src/util/types.h and src/audio/types.h.

/// Audio sample type (32-bit float). Equivalent to Mixxx's CSAMPLE.
pub type Sample = f32;

/// Gain value type. Equivalent to Mixxx's CSAMPLE_GAIN.
pub type SampleGain = f32;

/// Maximum frames per audio callback buffer (matching Mixxx's kMaxEngineFrames).
pub const MAX_ENGINE_FRAMES: usize = 8192;

/// Maximum samples per buffer (stereo: 2 channels * MAX_ENGINE_FRAMES).
pub const MAX_ENGINE_SAMPLES: usize = MAX_ENGINE_FRAMES * 2;
