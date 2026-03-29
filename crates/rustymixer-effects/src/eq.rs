//! 3-band parametric EQ using biquad filters.
//!
//! Every DJ deck gets one of these. Three bands (low, mid, high) with
//! gain knobs and kill switches.

use rustymixer_core::audio::SampleRate;

use crate::biquad::BiquadFilter;
use crate::manifest::{EffectManifest, ParameterManifest, ParameterType};
use crate::params::EffectParams;
use crate::processor::EffectProcessor;

/// Default crossover frequencies (Hz).
const LOW_FREQ: f64 = 100.0;
const MID_FREQ: f64 = 1000.0;
const HIGH_FREQ: f64 = 10000.0;

/// Mid-band quality factor.
const MID_Q: f64 = 1.0;

/// Gain applied when a kill switch is active (dB).
const KILL_GAIN_DB: f64 = -80.0;

/// Parameter indices (must match the manifest order).
const PARAM_LOW: usize = 0;
const PARAM_MID: usize = 1;
const PARAM_HIGH: usize = 2;
const PARAM_LOW_KILL: usize = 3;
const PARAM_MID_KILL: usize = 4;
const PARAM_HIGH_KILL: usize = 5;

/// 3-band parametric EQ built from biquad filters.
///
/// | Band | Filter type | Frequency | Q |
/// |------|-------------|-----------|---|
/// | Low  | Low shelf   | 100 Hz    | — |
/// | Mid  | Peaking     | 1 kHz     | 1.0 |
/// | High | High shelf  | 10 kHz    | — |
pub struct ThreeBandEQ {
    low: BiquadFilter,
    mid: BiquadFilter,
    high: BiquadFilter,
    manifest: EffectManifest,
    /// Cached per-band gain so we only recompute coefficients when params change.
    prev_low_db: f64,
    prev_mid_db: f64,
    prev_high_db: f64,
    sample_rate: f64,
}

impl ThreeBandEQ {
    /// Create a new 3-band EQ at the given sample rate.
    pub fn new(sample_rate: SampleRate) -> Self {
        let sr = sample_rate.hz() as f64;
        Self {
            low: BiquadFilter::low_shelf(LOW_FREQ, 0.0, sr),
            mid: BiquadFilter::peaking(MID_FREQ, 0.0, MID_Q, sr),
            high: BiquadFilter::high_shelf(HIGH_FREQ, 0.0, sr),
            manifest: Self::build_manifest(),
            prev_low_db: 0.0,
            prev_mid_db: 0.0,
            prev_high_db: 0.0,
            sample_rate: sr,
        }
    }

    fn build_manifest() -> EffectManifest {
        EffectManifest {
            id: "builtin:3band_eq".into(),
            name: "3-Band EQ".into(),
            description: "3-band parametric EQ with kill switches".into(),
            author: "RustyMixer".into(),
            parameters: vec![
                ParameterManifest {
                    id: "low".into(),
                    name: "Low".into(),
                    min: -24.0,
                    max: 12.0,
                    default: 0.0,
                    param_type: ParameterType::Knob,
                },
                ParameterManifest {
                    id: "mid".into(),
                    name: "Mid".into(),
                    min: -24.0,
                    max: 12.0,
                    default: 0.0,
                    param_type: ParameterType::Knob,
                },
                ParameterManifest {
                    id: "high".into(),
                    name: "High".into(),
                    min: -24.0,
                    max: 12.0,
                    default: 0.0,
                    param_type: ParameterType::Knob,
                },
                ParameterManifest {
                    id: "low_kill".into(),
                    name: "Low Kill".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    param_type: ParameterType::Button,
                },
                ParameterManifest {
                    id: "mid_kill".into(),
                    name: "Mid Kill".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    param_type: ParameterType::Button,
                },
                ParameterManifest {
                    id: "high_kill".into(),
                    name: "High Kill".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    param_type: ParameterType::Button,
                },
            ],
        }
    }

    /// Resolve the effective gain for a band, taking the kill switch into account.
    fn effective_gain(gain_db: f64, kill: f64) -> f64 {
        if kill >= 0.5 {
            KILL_GAIN_DB
        } else {
            gain_db
        }
    }

    /// Recompute filter coefficients if any parameter changed.
    fn update_filters(&mut self, params: &EffectParams) {
        let low_db = Self::effective_gain(params.get(PARAM_LOW), params.get(PARAM_LOW_KILL));
        let mid_db = Self::effective_gain(params.get(PARAM_MID), params.get(PARAM_MID_KILL));
        let high_db = Self::effective_gain(params.get(PARAM_HIGH), params.get(PARAM_HIGH_KILL));

        if low_db != self.prev_low_db {
            let f = BiquadFilter::low_shelf(LOW_FREQ, low_db, self.sample_rate);
            self.low.update_coefficients(f.b0(), f.b1(), f.b2(), f.a1(), f.a2());
            self.prev_low_db = low_db;
        }
        if mid_db != self.prev_mid_db {
            let f = BiquadFilter::peaking(MID_FREQ, mid_db, MID_Q, self.sample_rate);
            self.mid.update_coefficients(f.b0(), f.b1(), f.b2(), f.a1(), f.a2());
            self.prev_mid_db = mid_db;
        }
        if high_db != self.prev_high_db {
            let f = BiquadFilter::high_shelf(HIGH_FREQ, high_db, self.sample_rate);
            self.high.update_coefficients(f.b0(), f.b1(), f.b2(), f.a1(), f.a2());
            self.prev_high_db = high_db;
        }
    }
}

impl EffectProcessor for ThreeBandEQ {
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
        self.update_filters(params);

        let count = frames * 2;
        output[..count].copy_from_slice(&input[..count]);

        // Process in series: low → mid → high.
        self.low.process(&mut output[..count], frames);
        self.mid.process(&mut output[..count], frames);
        self.high.process(&mut output[..count], frames);
    }

    fn reset(&mut self) {
        self.low.reset();
        self.mid.reset();
        self.high.reset();
        self.prev_low_db = 0.0;
        self.prev_mid_db = 0.0;
        self.prev_high_db = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn sr() -> SampleRate {
        SampleRate::new(44100).unwrap()
    }

    fn sine_stereo(freq_hz: f64, frames: usize, sample_rate: f64) -> Vec<f32> {
        let mut buf = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let val = (2.0 * PI * freq_hz * i as f64 / sample_rate).sin() as f32;
            buf[i * 2] = val;
            buf[i * 2 + 1] = val;
        }
        buf
    }

    fn rms_left(buf: &[f32]) -> f64 {
        let n = buf.len() / 2;
        if n == 0 {
            return 0.0;
        }
        let sum: f64 = (0..n).map(|i| (buf[i * 2] as f64).powi(2)).sum();
        (sum / n as f64).sqrt()
    }

    #[test]
    fn unity_gain_passthrough() {
        let mut eq = ThreeBandEQ::new(sr());
        let params = EffectParams::with_defaults(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let frames = 4096;
        let input = sine_stereo(1000.0, frames, 44100.0);
        let mut output = vec![0.0f32; frames * 2];

        eq.process(&input, &mut output, frames, sr(), &params);

        // After transient, output ≈ input.
        for i in 200..frames {
            let idx = i * 2;
            assert!(
                (output[idx] - input[idx]).abs() < 1e-4,
                "frame {i}: got {} expected {}",
                output[idx],
                input[idx]
            );
        }
    }

    #[test]
    fn low_kill_silences_low_freq() {
        let mut eq = ThreeBandEQ::new(sr());
        let frames = 8192;
        let input = sine_stereo(50.0, frames, 44100.0);

        // No kill — normal output
        let mut output_normal = vec![0.0f32; frames * 2];
        let params_normal = EffectParams::with_defaults(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        eq.process(&input, &mut output_normal, frames, sr(), &params_normal);
        eq.reset();

        // Kill low
        let mut output_kill = vec![0.0f32; frames * 2];
        let params_kill = EffectParams::with_defaults(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        eq.process(&input, &mut output_kill, frames, sr(), &params_kill);

        let rms_normal = rms_left(&output_normal[frames..]);
        let rms_kill = rms_left(&output_kill[frames..]);

        assert!(
            rms_kill < rms_normal * 0.01,
            "kill should reduce low RMS dramatically: normal={rms_normal}, kill={rms_kill}"
        );
    }

    #[test]
    fn mid_kill_silences_mid_freq() {
        let mut eq = ThreeBandEQ::new(sr());
        let frames = 8192;
        let input = sine_stereo(1000.0, frames, 44100.0);

        let mut output_normal = vec![0.0f32; frames * 2];
        let params_normal = EffectParams::with_defaults(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        eq.process(&input, &mut output_normal, frames, sr(), &params_normal);
        eq.reset();

        let mut output_kill = vec![0.0f32; frames * 2];
        let params_kill = EffectParams::with_defaults(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        eq.process(&input, &mut output_kill, frames, sr(), &params_kill);

        let rms_normal = rms_left(&output_normal[frames..]);
        let rms_kill = rms_left(&output_kill[frames..]);

        // Peaking at -80 dB should attenuate significantly (though not as extreme as shelf).
        assert!(
            rms_kill < rms_normal * 0.1,
            "mid kill should attenuate: normal={rms_normal}, kill={rms_kill}"
        );
    }

    #[test]
    fn high_kill_silences_high_freq() {
        let mut eq = ThreeBandEQ::new(sr());
        let frames = 8192;
        let input = sine_stereo(15000.0, frames, 44100.0);

        let mut output_normal = vec![0.0f32; frames * 2];
        let params_normal = EffectParams::with_defaults(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        eq.process(&input, &mut output_normal, frames, sr(), &params_normal);
        eq.reset();

        let mut output_kill = vec![0.0f32; frames * 2];
        let params_kill = EffectParams::with_defaults(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        eq.process(&input, &mut output_kill, frames, sr(), &params_kill);

        let rms_normal = rms_left(&output_normal[frames..]);
        let rms_kill = rms_left(&output_kill[frames..]);

        assert!(
            rms_kill < rms_normal * 0.01,
            "kill should reduce high RMS: normal={rms_normal}, kill={rms_kill}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut eq = ThreeBandEQ::new(sr());
        let frames = 256;
        let input = sine_stereo(1000.0, frames, 44100.0);
        let mut output = vec![0.0f32; frames * 2];
        let params = EffectParams::with_defaults(&[6.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

        eq.process(&input, &mut output, frames, sr(), &params);
        eq.reset();

        // After reset, cached gains should be back to 0.
        assert_eq!(eq.prev_low_db, 0.0);
        assert_eq!(eq.prev_mid_db, 0.0);
        assert_eq!(eq.prev_high_db, 0.0);
    }

    #[test]
    fn manifest_has_correct_params() {
        let eq = ThreeBandEQ::new(sr());
        let m = eq.manifest();
        assert_eq!(m.id, "builtin:3band_eq");
        assert_eq!(m.parameters.len(), 6);
        assert_eq!(m.parameters[0].id, "low");
        assert_eq!(m.parameters[3].id, "low_kill");
        assert_eq!(m.parameters[3].param_type, ParameterType::Button);
    }
}
