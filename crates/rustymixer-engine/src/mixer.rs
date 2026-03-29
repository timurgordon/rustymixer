use std::sync::atomic::Ordering;

use rustymixer_core::audio::{ChannelCount, SampleBuffer, SampleRate, MAX_ENGINE_FRAMES};

use crate::channel::{ChannelId, EngineChannel};
use crate::gain::{apply_gain_ramped, AtomicF32};

/// Configuration for the engine mixer.
#[derive(Debug, Clone)]
pub struct EngineParameters {
    pub sample_rate: SampleRate,
    pub frames_per_buffer: usize,
}

impl Default for EngineParameters {
    fn default() -> Self {
        Self {
            sample_rate: SampleRate::default(),
            frames_per_buffer: 1024,
        }
    }
}

/// Central audio mixer that sums all channels into a stereo output bus.
///
/// The [`process`](EngineMixer::process) method is designed to run in the
/// audio callback thread and is **real-time safe**: it performs no heap
/// allocations, no mutex locks, and no blocking I/O.
pub struct EngineMixer {
    channels: Vec<Box<dyn EngineChannel>>,
    main_buffer: SampleBuffer,
    channel_buffer: SampleBuffer,
    main_gain: AtomicF32,
    prev_main_gain: f32,
    prev_channel_gains: Vec<f32>,
    params: EngineParameters,
}

impl EngineMixer {
    /// Create a new mixer with the given parameters.
    ///
    /// All internal buffers are allocated up-front so that [`process`](Self::process)
    /// never allocates.
    pub fn new(params: EngineParameters) -> Self {
        Self {
            channels: Vec::new(),
            main_buffer: SampleBuffer::new(MAX_ENGINE_FRAMES, ChannelCount::STEREO),
            channel_buffer: SampleBuffer::new(MAX_ENGINE_FRAMES, ChannelCount::STEREO),
            main_gain: AtomicF32::new(1.0),
            prev_main_gain: 1.0,
            prev_channel_gains: Vec::new(),
            params,
        }
    }

    /// Register a channel with the mixer.
    pub fn add_channel(&mut self, channel: Box<dyn EngineChannel>) {
        self.prev_channel_gains.push(channel.gain());
        self.channels.push(channel);
    }

    /// Remove a channel by its id. Returns the channel if found.
    pub fn remove_channel(&mut self, id: ChannelId) -> Option<Box<dyn EngineChannel>> {
        if let Some(pos) = self.channels.iter().position(|c| c.id() == id) {
            self.prev_channel_gains.remove(pos);
            Some(self.channels.remove(pos))
        } else {
            None
        }
    }

    /// Set the master gain. Safe to call from any thread.
    pub fn set_main_gain(&self, gain: f32) {
        self.main_gain.store(gain, Ordering::Relaxed);
    }

    /// Current master gain.
    pub fn main_gain(&self) -> f32 {
        self.main_gain.load(Ordering::Relaxed)
    }

    /// Engine parameters.
    pub fn params(&self) -> &EngineParameters {
        &self.params
    }

    /// Number of registered channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Process one buffer of audio. Called from the audio I/O callback.
    ///
    /// **Real-time safe**: no allocations, no locks, no blocking I/O.
    ///
    /// `output` must hold at least `frames * 2` samples (stereo interleaved).
    pub fn process(&mut self, output: &mut [f32], frames: usize) {
        debug_assert!(frames <= MAX_ENGINE_FRAMES);
        let samples = frames * 2;
        debug_assert!(output.len() >= samples);

        // 1. Zero the main accumulation buffer.
        let main = &mut self.main_buffer.as_mut_slice()[..samples];
        main.fill(0.0);

        // 2. For each active channel, process → apply gain → mix into main.
        for (i, channel) in self.channels.iter_mut().enumerate() {
            if !channel.is_active() {
                continue;
            }

            let chan = &mut self.channel_buffer.as_mut_slice()[..samples];
            chan.fill(0.0);

            if !channel.process(chan, frames) {
                continue;
            }

            // Apply channel gain with ramping.
            let new_gain = channel.gain();
            let old_gain = self.prev_channel_gains[i];
            let chan = &mut self.channel_buffer.as_mut_slice()[..samples];
            apply_gain_ramped(chan, old_gain, new_gain, frames);
            self.prev_channel_gains[i] = new_gain;

            // Sum into main buffer.
            let main = &mut self.main_buffer.as_mut_slice()[..samples];
            let chan = &self.channel_buffer.as_slice()[..samples];
            for (dst, &src) in main.iter_mut().zip(chan.iter()) {
                *dst += src;
            }
        }

        // 3. Apply master gain with ramping.
        let new_main_gain = self.main_gain.load(Ordering::Relaxed);
        let main = &mut self.main_buffer.as_mut_slice()[..samples];
        apply_gain_ramped(main, self.prev_main_gain, new_main_gain, frames);
        self.prev_main_gain = new_main_gain;

        // 4. Copy to caller's output buffer.
        output[..samples].copy_from_slice(&self.main_buffer.as_slice()[..samples]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelId, ChannelOrientation, EngineChannel};

    // ---- helpers -------------------------------------------------------

    /// Deterministic test channel that fills its buffer with a constant value.
    struct TestChannel {
        id: ChannelId,
        gain: f32,
        value: f32,
        active: bool,
    }

    impl TestChannel {
        fn new(id: u32, gain: f32, value: f32) -> Self {
            Self {
                id: ChannelId(id),
                gain,
                value,
                active: true,
            }
        }

        fn inactive(id: u32) -> Self {
            Self {
                id: ChannelId(id),
                gain: 1.0,
                value: 0.8,
                active: false,
            }
        }
    }

    impl EngineChannel for TestChannel {
        fn process(&mut self, buffer: &mut [f32], frames: usize) -> bool {
            for s in buffer[..frames * 2].iter_mut() {
                *s = self.value;
            }
            true
        }

        fn gain(&self) -> f32 {
            self.gain
        }

        fn orientation(&self) -> ChannelOrientation {
            ChannelOrientation::Center
        }

        fn is_active(&self) -> bool {
            self.active
        }

        fn id(&self) -> ChannelId {
            self.id
        }
    }

    // ---- tests ---------------------------------------------------------

    #[test]
    fn silence_when_no_channels() {
        let mut mixer = EngineMixer::new(EngineParameters::default());
        let mut output = vec![1.0f32; 2048];
        mixer.process(&mut output, 1024);
        assert!(output[..2048].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn mix_two_channels_additive() {
        let mut mixer = EngineMixer::new(EngineParameters::default());
        mixer.add_channel(Box::new(TestChannel::new(1, 1.0, 0.25)));
        mixer.add_channel(Box::new(TestChannel::new(2, 1.0, 0.25)));

        let frames = 128;
        let mut output = vec![0.0f32; frames * 2];
        mixer.process(&mut output, frames);

        // 0.25 + 0.25 = 0.50, at unity gain throughout.
        for &s in &output[..frames * 2] {
            assert!((s - 0.5).abs() < 0.001, "expected ~0.5, got {s}");
        }
    }

    #[test]
    fn channel_gain_applied() {
        let mut mixer = EngineMixer::new(EngineParameters::default());
        mixer.add_channel(Box::new(TestChannel::new(1, 0.5, 1.0)));

        let frames = 128;
        let mut output = vec![0.0f32; frames * 2];
        mixer.process(&mut output, frames);

        // Channel produces 1.0, gain ramps 0.5→0.5 (constant) → output ≈ 0.5.
        for &s in &output[..frames * 2] {
            assert!((s - 0.5).abs() < 0.001, "expected ~0.5, got {s}");
        }
    }

    #[test]
    fn inactive_channels_produce_silence() {
        let mut mixer = EngineMixer::new(EngineParameters::default());
        mixer.add_channel(Box::new(TestChannel::inactive(1)));

        let frames = 128;
        let mut output = vec![1.0f32; frames * 2];
        mixer.process(&mut output, frames);

        assert!(
            output[..frames * 2].iter().all(|&s| s == 0.0),
            "inactive channel should not contribute audio"
        );
    }

    #[test]
    fn master_gain_applied() {
        let mut mixer = EngineMixer::new(EngineParameters::default());
        mixer.set_main_gain(0.5);
        mixer.add_channel(Box::new(TestChannel::new(1, 1.0, 1.0)));

        let frames = 128;
        let mut output = vec![0.0f32; frames * 2];

        // First call ramps master gain from 1.0 (initial) to 0.5.
        mixer.process(&mut output, frames);
        // Second call: master gain is now stable at 0.5.
        mixer.process(&mut output, frames);

        for &s in &output[..frames * 2] {
            assert!((s - 0.5).abs() < 0.001, "expected ~0.5, got {s}");
        }
    }

    #[test]
    fn remove_channel() {
        let mut mixer = EngineMixer::new(EngineParameters::default());
        mixer.add_channel(Box::new(TestChannel::new(1, 1.0, 0.3)));
        mixer.add_channel(Box::new(TestChannel::new(2, 1.0, 0.7)));
        assert_eq!(mixer.channel_count(), 2);

        let removed = mixer.remove_channel(ChannelId(1));
        assert!(removed.is_some());
        assert_eq!(mixer.channel_count(), 1);

        // Only channel 2 remains (0.7).
        let frames = 64;
        let mut output = vec![0.0f32; frames * 2];
        mixer.process(&mut output, frames);
        for &s in &output[..frames * 2] {
            assert!((s - 0.7).abs() < 0.001, "expected ~0.7, got {s}");
        }
    }

    #[test]
    fn process_performance_two_channels_1024_frames() {
        let mut mixer = EngineMixer::new(EngineParameters::default());
        mixer.add_channel(Box::new(TestChannel::new(1, 0.8, 0.5)));
        mixer.add_channel(Box::new(TestChannel::new(2, 0.7, 0.3)));

        let frames = 1024;
        let mut output = vec![0.0f32; frames * 2];

        // Warm up.
        for _ in 0..10 {
            mixer.process(&mut output, frames);
        }

        let start = std::time::Instant::now();
        let iterations = 1000;
        for _ in 0..iterations {
            mixer.process(&mut output, frames);
        }
        let per_call = start.elapsed() / iterations;

        // At 44100 Hz, 1024 frames ≈ 23 ms of audio. process() must finish
        // well under 1 ms.
        assert!(
            per_call.as_micros() < 1000,
            "process() took {per_call:?}, expected < 1 ms"
        );
    }
}
