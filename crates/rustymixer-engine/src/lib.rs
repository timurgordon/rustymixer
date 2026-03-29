//! Real-time audio mixing engine for RustyMixer.
//!
//! Contains the central [`EngineMixer`] that orchestrates all audio channels,
//! applies gain with click-free ramping, and mixes them into a main stereo
//! output bus.  The [`CachingReader`] pre-reads audio in a background thread
//! to provide lock-free sample access for the audio callback.

mod caching_reader;
mod channel;
mod crossfader;
mod gain;
mod mixer;

pub use caching_reader::{CachingReader, HintPriority, ReadHint};
pub use channel::{ChannelId, ChannelOrientation, EngineChannel};
pub use crossfader::{Crossfader, CrossfaderCurve};
pub use gain::{apply_gain, apply_gain_ramped, AtomicF32};
pub use mixer::{EngineMixer, EngineParameters};
