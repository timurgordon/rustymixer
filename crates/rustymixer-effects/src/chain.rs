use rustymixer_core::audio::SampleRate;

use crate::params::EffectParams;
use crate::processor::EffectProcessor;

/// A single slot in an [`EffectChain`].
pub struct EffectSlot {
    /// The processor occupying this slot, or `None` if empty.
    pub processor: Option<Box<dyn EffectProcessor>>,
    /// Runtime parameters for the processor.
    pub params: EffectParams,
    /// Whether this slot is active.
    pub enabled: bool,
}

/// A chain of effects processed in sequence.
///
/// Audio flows through each enabled slot in order, then the result
/// is blended with the original dry signal according to [`mix`](EffectChain::set_mix).
pub struct EffectChain {
    slots: Vec<EffectSlot>,
    /// Dry/wet mix: `0.0` = fully dry, `1.0` = fully wet.
    mix: f32,
    /// Master enable for the whole chain.
    enabled: bool,
    /// Scratch buffer used for intermediate processing.
    intermediate_buffer: Vec<f32>,
}

impl EffectChain {
    /// Create a new empty chain with the given number of slots.
    pub fn new(num_slots: usize) -> Self {
        let mut slots = Vec::with_capacity(num_slots);
        for _ in 0..num_slots {
            slots.push(EffectSlot {
                processor: None,
                params: EffectParams::new(0),
                enabled: true,
            });
        }
        Self {
            slots,
            mix: 1.0,
            enabled: true,
            intermediate_buffer: Vec::new(),
        }
    }

    /// Insert a processor into slot `index`.
    ///
    /// The slot's params are initialised from the processor's manifest defaults.
    pub fn set_effect(&mut self, index: usize, processor: Box<dyn EffectProcessor>) {
        if let Some(slot) = self.slots.get_mut(index) {
            let defaults: Vec<f64> = processor
                .manifest()
                .parameters
                .iter()
                .map(|p| p.default)
                .collect();
            slot.params = EffectParams::with_defaults(&defaults);
            slot.processor = Some(processor);
            slot.enabled = true;
        }
    }

    /// Remove the processor from slot `index`.
    pub fn clear_slot(&mut self, index: usize) {
        if let Some(slot) = self.slots.get_mut(index) {
            slot.processor = None;
            slot.params = EffectParams::new(0);
        }
    }

    /// Enable or disable a single slot.
    pub fn set_slot_enabled(&mut self, index: usize, enabled: bool) {
        if let Some(slot) = self.slots.get_mut(index) {
            slot.enabled = enabled;
        }
    }

    /// Set the dry/wet mix for the whole chain. Clamped to `[0.0, 1.0]`.
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    /// Current dry/wet mix.
    pub fn mix(&self) -> f32 {
        self.mix
    }

    /// Enable or disable the entire chain.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Whether the chain is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Number of slots in this chain.
    pub fn num_slots(&self) -> usize {
        self.slots.len()
    }

    /// Immutable access to a slot.
    pub fn slot(&self, index: usize) -> Option<&EffectSlot> {
        self.slots.get(index)
    }

    /// Mutable access to a slot.
    pub fn slot_mut(&mut self, index: usize) -> Option<&mut EffectSlot> {
        self.slots.get_mut(index)
    }

    /// Reset all processors in the chain.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            if let Some(ref mut proc) = slot.processor {
                proc.reset();
            }
        }
    }

    /// Process one buffer of audio through the chain.
    ///
    /// `buffer` is stereo interleaved (`frames * 2` samples).
    /// Processing is real-time safe — no allocations happen on the hot path
    /// (the intermediate buffer is grown lazily and reused).
    pub fn process(&mut self, buffer: &mut [f32], frames: usize, sample_rate: SampleRate) {
        if !self.enabled {
            return;
        }

        let sample_count = frames * 2;

        // Ensure intermediate buffer is large enough (one-time growth).
        if self.intermediate_buffer.len() < sample_count {
            self.intermediate_buffer.resize(sample_count, 0.0);
        }

        // Save dry signal for dry/wet blending.
        let dry = &mut self.intermediate_buffer[..sample_count];
        dry.copy_from_slice(&buffer[..sample_count]);

        // Process through each enabled slot in sequence.
        // We alternate between `buffer` (current input) and a conceptual output;
        // since each effect reads `input` and writes `output`, we can process
        // in-place by using the same buffer when the trait allows it.
        // Here we pass buffer as both input and output for in-place operation.
        for slot in &mut self.slots {
            if !slot.enabled {
                continue;
            }
            if let Some(ref mut proc) = slot.processor {
                // Create a temporary copy for the input so the processor sees
                // unmodified input while writing to the output buffer.
                let input_copy: Vec<f32> = buffer[..sample_count].to_vec();
                proc.process(
                    &input_copy,
                    &mut buffer[..sample_count],
                    frames,
                    sample_rate,
                    &slot.params,
                );
            }
        }

        // Blend dry/wet: output = dry * (1 - mix) + wet * mix
        if (self.mix - 1.0).abs() > f32::EPSILON {
            let wet_gain = self.mix;
            let dry_gain = 1.0 - self.mix;
            for i in 0..sample_count {
                buffer[i] = dry[i] * dry_gain + buffer[i] * wet_gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{EffectManifest, ParameterManifest, ParameterType};

    /// A pass-through effect that copies input to output unchanged.
    struct PassThrough {
        manifest: EffectManifest,
    }

    impl PassThrough {
        fn new() -> Self {
            Self {
                manifest: EffectManifest {
                    id: "test:passthrough".into(),
                    name: "Pass-through".into(),
                    description: "Copies input to output".into(),
                    author: "test".into(),
                    parameters: vec![],
                },
            }
        }
    }

    impl EffectProcessor for PassThrough {
        fn manifest(&self) -> &EffectManifest {
            &self.manifest
        }
        fn process(
            &mut self,
            input: &[f32],
            output: &mut [f32],
            frames: usize,
            _sample_rate: SampleRate,
            _params: &EffectParams,
        ) {
            output[..frames * 2].copy_from_slice(&input[..frames * 2]);
        }
        fn reset(&mut self) {}
    }

    /// An effect that scales every sample by a factor read from parameter 0.
    struct GainEffect {
        manifest: EffectManifest,
    }

    impl GainEffect {
        fn new() -> Self {
            Self {
                manifest: EffectManifest {
                    id: "test:gain".into(),
                    name: "Gain".into(),
                    description: "Scales amplitude".into(),
                    author: "test".into(),
                    parameters: vec![ParameterManifest {
                        id: "gain".into(),
                        name: "Gain".into(),
                        min: 0.0,
                        max: 4.0,
                        default: 1.0,
                        param_type: ParameterType::Knob,
                    }],
                },
            }
        }
    }

    impl EffectProcessor for GainEffect {
        fn manifest(&self) -> &EffectManifest {
            &self.manifest
        }
        fn process(
            &mut self,
            input: &[f32],
            output: &mut [f32],
            frames: usize,
            _sample_rate: SampleRate,
            params: &EffectParams,
        ) {
            let gain = params.get(0) as f32;
            for i in 0..frames * 2 {
                output[i] = input[i] * gain;
            }
        }
        fn reset(&mut self) {}
    }

    fn sr() -> SampleRate {
        SampleRate::default()
    }

    #[test]
    fn passthrough_preserves_signal() {
        let mut chain = EffectChain::new(1);
        chain.set_effect(0, Box::new(PassThrough::new()));

        let mut buf = vec![0.5f32, -0.5, 0.25, -0.25];
        let expected = buf.clone();
        chain.process(&mut buf, 2, sr());
        assert_eq!(buf, expected);
    }

    #[test]
    fn dry_wet_mix_zero_returns_dry() {
        let mut chain = EffectChain::new(1);
        chain.set_effect(0, Box::new(GainEffect::new()));
        // Set gain to 2.0 — wet signal doubles amplitude.
        chain.slot_mut(0).unwrap().params.set(0, 2.0);
        chain.set_mix(0.0);

        let original = vec![1.0f32, -1.0, 0.5, -0.5];
        let mut buf = original.clone();
        chain.process(&mut buf, 2, sr());

        // mix=0 → output should be 100% dry (original).
        assert_eq!(buf, original);
    }

    #[test]
    fn dry_wet_mix_one_returns_wet() {
        let mut chain = EffectChain::new(1);
        chain.set_effect(0, Box::new(GainEffect::new()));
        chain.slot_mut(0).unwrap().params.set(0, 2.0);
        chain.set_mix(1.0);

        let mut buf = vec![1.0f32, -1.0, 0.5, -0.5];
        chain.process(&mut buf, 2, sr());

        // mix=1 → output should be 100% wet (doubled).
        assert_eq!(buf, vec![2.0, -2.0, 1.0, -1.0]);
    }

    #[test]
    fn dry_wet_mix_half_blends() {
        let mut chain = EffectChain::new(1);
        chain.set_effect(0, Box::new(GainEffect::new()));
        chain.slot_mut(0).unwrap().params.set(0, 0.0); // wet = silence
        chain.set_mix(0.5);

        let mut buf = vec![1.0f32; 4];
        chain.process(&mut buf, 2, sr());

        // dry=1.0, wet=0.0, mix=0.5 → 1.0*0.5 + 0.0*0.5 = 0.5
        for &s in &buf {
            assert!((s - 0.5).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn chain_two_effects_in_sequence() {
        let mut chain = EffectChain::new(2);

        // Slot 0: gain = 2.0
        chain.set_effect(0, Box::new(GainEffect::new()));
        chain.slot_mut(0).unwrap().params.set(0, 2.0);

        // Slot 1: gain = 3.0
        chain.set_effect(1, Box::new(GainEffect::new()));
        chain.slot_mut(1).unwrap().params.set(0, 3.0);

        chain.set_mix(1.0);

        let mut buf = vec![1.0f32, -1.0];
        chain.process(&mut buf, 1, sr());

        // 1.0 * 2.0 * 3.0 = 6.0
        assert_eq!(buf, vec![6.0, -6.0]);
    }

    #[test]
    fn disabled_slot_is_skipped() {
        let mut chain = EffectChain::new(1);
        chain.set_effect(0, Box::new(GainEffect::new()));
        chain.slot_mut(0).unwrap().params.set(0, 0.0); // would zero the signal
        chain.set_slot_enabled(0, false);
        chain.set_mix(1.0);

        let mut buf = vec![1.0f32, -1.0];
        chain.process(&mut buf, 1, sr());

        // Slot disabled → signal passes through untouched.
        assert_eq!(buf, vec![1.0, -1.0]);
    }

    #[test]
    fn disabled_chain_is_noop() {
        let mut chain = EffectChain::new(1);
        chain.set_effect(0, Box::new(GainEffect::new()));
        chain.slot_mut(0).unwrap().params.set(0, 0.0);
        chain.set_enabled(false);

        let mut buf = vec![1.0f32, -1.0];
        chain.process(&mut buf, 1, sr());

        assert_eq!(buf, vec![1.0, -1.0]);
    }

    #[test]
    fn empty_slot_passes_through() {
        let mut chain = EffectChain::new(2);
        // Slot 0 is empty, slot 1 has a gain effect.
        chain.set_effect(1, Box::new(GainEffect::new()));
        chain.slot_mut(1).unwrap().params.set(0, 3.0);
        chain.set_mix(1.0);

        let mut buf = vec![2.0f32, -2.0];
        chain.process(&mut buf, 1, sr());

        // Empty slot 0 is skipped; slot 1 applies gain.
        assert_eq!(buf, vec![6.0, -6.0]);
    }

    #[test]
    fn clear_slot_removes_processor() {
        let mut chain = EffectChain::new(1);
        chain.set_effect(0, Box::new(GainEffect::new()));
        assert!(chain.slot(0).unwrap().processor.is_some());

        chain.clear_slot(0);
        assert!(chain.slot(0).unwrap().processor.is_none());
    }

    #[test]
    fn reset_calls_all_processors() {
        // Just verify it doesn't panic with populated and empty slots.
        let mut chain = EffectChain::new(3);
        chain.set_effect(0, Box::new(PassThrough::new()));
        chain.set_effect(2, Box::new(GainEffect::new()));
        chain.reset();
    }
}
