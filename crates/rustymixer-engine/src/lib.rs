//! Real-time audio mixing engine for RustyMixer.
//!
//! Contains the central [`EngineMixer`] that orchestrates all audio channels,
//! applies gain with click-free ramping, and mixes them into a main stereo
//! output bus.

mod channel;
mod gain;
mod mixer;

pub use channel::{ChannelId, ChannelOrientation, EngineChannel};
pub use gain::{apply_gain, apply_gain_ramped, AtomicF32};
pub use mixer::{EngineMixer, EngineParameters};
