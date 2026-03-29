//! Fundamental audio types.
//!
//! These types are used throughout the entire RustyMixer codebase.
//! Inspired by Mixxx's src/util/types.h and src/audio/types.h.

use serde::{Deserialize, Serialize};

/// Audio sample type (32-bit float). Equivalent to Mixxx's CSAMPLE.
pub type Sample = f32;

/// Gain value type. Equivalent to Mixxx's CSAMPLE_GAIN.
pub type SampleGain = f32;

/// Maximum frames per audio callback buffer (matching Mixxx's kMaxEngineFrames).
pub const MAX_ENGINE_FRAMES: usize = 8192;

/// Maximum samples per buffer (stereo: 2 channels * MAX_ENGINE_FRAMES).
pub const MAX_ENGINE_SAMPLES: usize = MAX_ENGINE_FRAMES * 2;

/// Validated sample rate in Hz.
///
/// Valid range: 8000..=192000. Defaults to 44100 Hz.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SampleRate(u32);

impl SampleRate {
    /// Minimum supported sample rate.
    pub const MIN: u32 = 8000;
    /// Maximum supported sample rate.
    pub const MAX: u32 = 192000;

    /// Create a new `SampleRate`, returning `None` if out of range.
    pub fn new(hz: u32) -> Option<Self> {
        if (Self::MIN..=Self::MAX).contains(&hz) {
            Some(Self(hz))
        } else {
            None
        }
    }

    /// Return the raw Hz value.
    pub fn hz(self) -> u32 {
        self.0
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        Self(44100)
    }
}

/// Channel count (1 = mono, 2 = stereo).
///
/// Defaults to stereo (2).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ChannelCount(u8);

impl ChannelCount {
    pub const MONO: Self = Self(1);
    pub const STEREO: Self = Self(2);

    /// Create a new `ChannelCount`. Only 1 (mono) and 2 (stereo) are valid.
    pub fn new(count: u8) -> Option<Self> {
        if count == 1 || count == 2 {
            Some(Self(count))
        } else {
            None
        }
    }

    /// Return the raw channel count.
    pub fn count(self) -> u8 {
        self.0
    }
}

impl Default for ChannelCount {
    fn default() -> Self {
        Self::STEREO
    }
}

/// Fractional frame position for sub-sample accuracy.
///
/// Inspired by Mixxx's `FramePos`. A frame contains one sample per channel,
/// so frame 0 in stereo corresponds to samples [0, 1].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FramePos(f64);

impl FramePos {
    /// Create a new `FramePos` from a fractional frame index.
    pub fn new(pos: f64) -> Self {
        Self(pos)
    }

    /// A frame position is valid when it is non-negative and finite.
    pub fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 >= 0.0
    }

    /// Convert this frame position to a sample position for the given channel count.
    pub fn to_sample_pos(&self, channels: ChannelCount) -> f64 {
        self.0 * channels.count() as f64
    }

    /// Advance the position by the given number of frames.
    pub fn advance(&self, frames: f64) -> Self {
        Self(self.0 + frames)
    }

    /// Create a `FramePos` from a time in seconds and a sample rate.
    pub fn from_seconds(secs: f64, sample_rate: SampleRate) -> Self {
        Self(secs * sample_rate.hz() as f64)
    }

    /// Return the raw f64 value.
    pub fn value(&self) -> f64 {
        self.0
    }
}

/// Frame-oriented audio sample buffer.
///
/// Wraps a `Vec<f32>` and provides frame-based access where each frame
/// contains `channels` interleaved samples.
#[derive(Debug, Clone)]
pub struct SampleBuffer {
    data: Vec<Sample>,
    channels: ChannelCount,
}

impl SampleBuffer {
    /// Allocate a new zeroed buffer for the given number of frames and channels.
    pub fn new(frames: usize, channels: ChannelCount) -> Self {
        let sample_count = frames * channels.count() as usize;
        Self {
            data: vec![0.0; sample_count],
            channels,
        }
    }

    /// Number of frames in this buffer.
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.count() as usize
    }

    /// Channel count of this buffer.
    pub fn channels(&self) -> ChannelCount {
        self.channels
    }

    /// Raw sample slice.
    pub fn as_slice(&self) -> &[Sample] {
        &self.data
    }

    /// Mutable raw sample slice.
    pub fn as_mut_slice(&mut self) -> &mut [Sample] {
        &mut self.data
    }

    /// Slice of samples for frame `n`.
    ///
    /// # Panics
    /// Panics if `n >= self.frames()`.
    pub fn frame(&self, n: usize) -> &[Sample] {
        let ch = self.channels.count() as usize;
        let start = n * ch;
        &self.data[start..start + ch]
    }

    /// Zero all samples.
    pub fn clear(&mut self) {
        self.data.fill(0.0);
    }

    /// Add this buffer's samples, scaled by `gain`, into `other`.
    ///
    /// # Panics
    /// Panics if `other` has a different length.
    pub fn mix_into(&self, other: &mut SampleBuffer, gain: SampleGain) {
        assert_eq!(
            self.data.len(),
            other.data.len(),
            "mix_into: buffer lengths must match"
        );
        for (dst, &src) in other.data.iter_mut().zip(self.data.iter()) {
            *dst += src * gain;
        }
    }
}

/// Basic track metadata.
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: f64,
    pub sample_rate: SampleRate,
    pub channels: ChannelCount,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SampleRate tests ---

    #[test]
    fn sample_rate_rejects_zero() {
        assert!(SampleRate::new(0).is_none());
    }

    #[test]
    fn sample_rate_rejects_below_min() {
        assert!(SampleRate::new(7999).is_none());
    }

    #[test]
    fn sample_rate_rejects_above_max() {
        assert!(SampleRate::new(192001).is_none());
    }

    #[test]
    fn sample_rate_accepts_common_rates() {
        for hz in [8000, 22050, 44100, 48000, 96000, 192000] {
            let sr = SampleRate::new(hz).unwrap();
            assert_eq!(sr.hz(), hz);
        }
    }

    #[test]
    fn sample_rate_default_is_44100() {
        assert_eq!(SampleRate::default().hz(), 44100);
    }

    // --- ChannelCount tests ---

    #[test]
    fn channel_count_mono_and_stereo() {
        assert_eq!(ChannelCount::new(1).unwrap().count(), 1);
        assert_eq!(ChannelCount::new(2).unwrap().count(), 2);
    }

    #[test]
    fn channel_count_rejects_invalid() {
        assert!(ChannelCount::new(0).is_none());
        assert!(ChannelCount::new(3).is_none());
    }

    #[test]
    fn channel_count_default_is_stereo() {
        assert_eq!(ChannelCount::default().count(), 2);
    }

    // --- FramePos tests ---

    #[test]
    fn frame_pos_is_valid() {
        assert!(FramePos::new(0.0).is_valid());
        assert!(FramePos::new(100.5).is_valid());
        assert!(!FramePos::new(-1.0).is_valid());
        assert!(!FramePos::new(f64::NAN).is_valid());
        assert!(!FramePos::new(f64::INFINITY).is_valid());
    }

    #[test]
    fn frame_pos_to_sample_pos() {
        let pos = FramePos::new(10.0);
        assert_eq!(pos.to_sample_pos(ChannelCount::MONO), 10.0);
        assert_eq!(pos.to_sample_pos(ChannelCount::STEREO), 20.0);
    }

    #[test]
    fn frame_pos_advance() {
        let pos = FramePos::new(5.0);
        let advanced = pos.advance(3.5);
        assert_eq!(advanced.value(), 8.5);
    }

    #[test]
    fn frame_pos_from_seconds() {
        let sr = SampleRate::new(44100).unwrap();
        let pos = FramePos::from_seconds(1.0, sr);
        assert_eq!(pos.value(), 44100.0);

        let pos_half = FramePos::from_seconds(0.5, sr);
        assert_eq!(pos_half.value(), 22050.0);
    }

    // --- SampleBuffer tests ---

    #[test]
    fn sample_buffer_allocation() {
        let buf = SampleBuffer::new(128, ChannelCount::STEREO);
        assert_eq!(buf.frames(), 128);
        assert_eq!(buf.channels(), ChannelCount::STEREO);
        assert_eq!(buf.as_slice().len(), 256);
        assert!(buf.as_slice().iter().all(|&s| s == 0.0));
    }

    #[test]
    fn sample_buffer_frame_access() {
        let mut buf = SampleBuffer::new(4, ChannelCount::STEREO);
        // Write to frame 2
        let ch = buf.channels().count() as usize;
        let start = 2 * ch;
        buf.as_mut_slice()[start] = 0.5;
        buf.as_mut_slice()[start + 1] = -0.5;

        let frame = buf.frame(2);
        assert_eq!(frame, &[0.5, -0.5]);
    }

    #[test]
    fn sample_buffer_clear() {
        let mut buf = SampleBuffer::new(16, ChannelCount::STEREO);
        buf.as_mut_slice().fill(1.0);
        assert!(buf.as_slice().iter().all(|&s| s == 1.0));

        buf.clear();
        assert!(buf.as_slice().iter().all(|&s| s == 0.0));
    }

    #[test]
    fn sample_buffer_mix_into() {
        let mut src = SampleBuffer::new(4, ChannelCount::STEREO);
        let mut dst = SampleBuffer::new(4, ChannelCount::STEREO);

        // Fill src with 1.0
        src.as_mut_slice().fill(1.0);
        // Fill dst with 0.5
        dst.as_mut_slice().fill(0.5);

        // Mix src into dst with gain 0.5 → dst should be 0.5 + 1.0*0.5 = 1.0
        src.mix_into(&mut dst, 0.5);
        assert!(dst.as_slice().iter().all(|&s| (s - 1.0).abs() < f32::EPSILON));
    }

    #[test]
    #[should_panic(expected = "buffer lengths must match")]
    fn sample_buffer_mix_into_panics_on_mismatch() {
        let src = SampleBuffer::new(4, ChannelCount::STEREO);
        let mut dst = SampleBuffer::new(8, ChannelCount::STEREO);
        src.mix_into(&mut dst, 1.0);
    }

    #[test]
    fn sample_buffer_mono() {
        let buf = SampleBuffer::new(64, ChannelCount::MONO);
        assert_eq!(buf.frames(), 64);
        assert_eq!(buf.as_slice().len(), 64);
        assert_eq!(buf.frame(0).len(), 1);
    }
}
