use rustymixer_core::audio::SampleRate;

use crate::manifest::EffectManifest;
use crate::params::EffectParams;

/// A single audio effect that processes samples.
///
/// Implementations must be real-time safe: no allocations, no locks, no I/O
/// inside [`process`](EffectProcessor::process).
pub trait EffectProcessor: Send {
    /// Return the effect's metadata/manifest.
    fn manifest(&self) -> &EffectManifest;

    /// Process audio. Called from the audio thread — must be real-time safe.
    ///
    /// * `input`  — interleaved stereo `f32` source buffer (`frames * 2` samples).
    /// * `output` — interleaved stereo `f32` destination buffer (same length).
    /// * `frames` — number of audio frames to process.
    /// * `sample_rate` — current playback sample rate.
    /// * `params`  — current parameter values for this instance.
    fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        frames: usize,
        sample_rate: SampleRate,
        params: &EffectParams,
    );

    /// Reset internal state (e.g. clear delay buffers).
    ///
    /// Called when a new track is loaded or the effect is re-enabled.
    fn reset(&mut self);
}
