//! Generic second-order IIR (biquad) filter.
//!
//! Coefficient formulas follow the *Audio EQ Cookbook* by Robert Bristow-Johnson:
//! <https://www.w3.org/2011/audio/audio-eq-cookbook.html>

use std::f64::consts::PI;

/// Second-order IIR (biquad) filter for stereo audio.
///
/// Transfer function:
/// ```text
/// H(z) = (b0 + b1·z⁻¹ + b2·z⁻²) / (1 + a1·z⁻¹ + a2·z⁻²)
/// ```
///
/// Internal state is maintained per-channel (left = 0, right = 1).
/// All arithmetic is `f64` for numerical stability; conversion to/from `f32`
/// happens only at the sample boundary.
#[derive(Debug, Clone)]
pub struct BiquadFilter {
    // Normalised coefficients (a0 is folded into b0–b2 and a1–a2).
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    // Per-channel state: index 0 = left, 1 = right.
    x1: [f64; 2],
    x2: [f64; 2],
    y1: [f64; 2],
    y2: [f64; 2],
}

impl BiquadFilter {
    /// Create a **low-shelf** filter.
    ///
    /// Boosts or cuts frequencies below `frequency` by `gain_db` decibels.
    /// Uses a default slope S = 1.0.
    pub fn low_shelf(frequency: f64, gain_db: f64, sample_rate: f64) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0); // sqrt of linear gain
        let w0 = 2.0 * PI * frequency / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let s = 1.0; // shelf slope
        let alpha = (sin_w0 / 2.0) * ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self::from_raw(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// Create a **peaking EQ** filter.
    ///
    /// Boosts or cuts a band centred at `frequency` by `gain_db` decibels
    /// with quality factor `q`.
    pub fn peaking(frequency: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * frequency / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        let b0 = (1.0 + alpha * a) / a0;
        let b1 = (-2.0 * cos_w0) / a0;
        let b2 = (1.0 - alpha * a) / a0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha / a) / a0;

        Self::from_raw(b0, b1, b2, a1, a2)
    }

    /// Create a **high-shelf** filter.
    ///
    /// Boosts or cuts frequencies above `frequency` by `gain_db` decibels.
    /// Uses a default slope S = 1.0.
    pub fn high_shelf(frequency: f64, gain_db: f64, sample_rate: f64) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * frequency / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let s = 1.0;
        let alpha = (sin_w0 / 2.0) * ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self::from_raw(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// Build from pre-normalised coefficients with zeroed state.
    fn from_raw(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: [0.0; 2],
            x2: [0.0; 2],
            y1: [0.0; 2],
            y2: [0.0; 2],
        }
    }

    /// Process one stereo frame in-place.
    #[inline]
    pub fn process_frame(&mut self, left: &mut f32, right: &mut f32) {
        *left = self.tick(0, *left as f64) as f32;
        *right = self.tick(1, *right as f64) as f32;
    }

    /// Process a buffer of interleaved stereo samples in-place.
    pub fn process(&mut self, buffer: &mut [f32], frames: usize) {
        for i in 0..frames {
            let idx = i * 2;
            buffer[idx] = self.tick(0, buffer[idx] as f64) as f32;
            buffer[idx + 1] = self.tick(1, buffer[idx + 1] as f64) as f32;
        }
    }

    /// Reset filter state (clear delay-line history).
    pub fn reset(&mut self) {
        self.x1 = [0.0; 2];
        self.x2 = [0.0; 2];
        self.y1 = [0.0; 2];
        self.y2 = [0.0; 2];
    }

    /// Update coefficients without resetting state (for smooth parameter changes).
    pub fn update_coefficients(&mut self, b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) {
        self.b0 = b0;
        self.b1 = b1;
        self.b2 = b2;
        self.a1 = a1;
        self.a2 = a2;
    }

    /// Direct Form I tick for one channel.
    #[inline]
    fn tick(&mut self, ch: usize, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1[ch] + self.b2 * self.x2[ch]
            - self.a1 * self.y1[ch]
            - self.a2 * self.y2[ch];

        self.x2[ch] = self.x1[ch];
        self.x1[ch] = x;
        self.y2[ch] = self.y1[ch];
        self.y1[ch] = y;

        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 44100.0;
    const TOLERANCE: f64 = 1e-4;

    /// Generate `frames` of a sine wave at `freq_hz` (stereo interleaved).
    fn sine_stereo(freq_hz: f64, frames: usize, sample_rate: f64) -> Vec<f32> {
        let mut buf = vec![0.0f32; frames * 2];
        for i in 0..frames {
            let val = (2.0 * PI * freq_hz * i as f64 / sample_rate).sin() as f32;
            buf[i * 2] = val;
            buf[i * 2 + 1] = val;
        }
        buf
    }

    /// RMS of a stereo-interleaved buffer (left channel only for simplicity).
    fn rms_left(buf: &[f32]) -> f64 {
        let n = buf.len() / 2;
        if n == 0 {
            return 0.0;
        }
        let sum: f64 = (0..n).map(|i| (buf[i * 2] as f64).powi(2)).sum();
        (sum / n as f64).sqrt()
    }

    #[test]
    fn peaking_zero_gain_is_unity() {
        let mut f = BiquadFilter::peaking(1000.0, 0.0, 1.0, SR);
        let mut buf = sine_stereo(1000.0, 4096, SR);
        let original = buf.clone();
        f.process(&mut buf, 4096);

        // After transient, samples should match closely.
        for i in 200..4096 {
            let idx = i * 2;
            assert!(
                (buf[idx] - original[idx]).abs() < TOLERANCE as f32,
                "frame {i}: got {} expected {}",
                buf[idx],
                original[idx]
            );
        }
    }

    #[test]
    fn low_shelf_boost_increases_low_freq() {
        let frames = 8192;
        // 50 Hz sine — well below the 100 Hz shelf corner
        let mut boosted = sine_stereo(50.0, frames, SR);
        let mut flat = boosted.clone();

        let mut boost = BiquadFilter::low_shelf(100.0, 12.0, SR);
        let mut unity = BiquadFilter::low_shelf(100.0, 0.0, SR);

        boost.process(&mut boosted, frames);
        unity.process(&mut flat, frames);

        // Skip transient, compare RMS of the second half.
        let rms_boost = rms_left(&boosted[frames..]);
        let rms_flat = rms_left(&flat[frames..]);
        assert!(
            rms_boost > rms_flat * 1.5,
            "boosted RMS {rms_boost} should be significantly larger than flat {rms_flat}"
        );
    }

    #[test]
    fn high_shelf_boost_increases_high_freq() {
        let frames = 8192;
        let mut boosted = sine_stereo(15000.0, frames, SR);
        let mut flat = boosted.clone();

        let mut boost = BiquadFilter::high_shelf(10000.0, 12.0, SR);
        let mut unity = BiquadFilter::high_shelf(10000.0, 0.0, SR);

        boost.process(&mut boosted, frames);
        unity.process(&mut flat, frames);

        let rms_boost = rms_left(&boosted[frames..]);
        let rms_flat = rms_left(&flat[frames..]);
        assert!(
            rms_boost > rms_flat * 1.5,
            "boosted RMS {rms_boost} should be significantly larger than flat {rms_flat}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut f = BiquadFilter::peaking(1000.0, 6.0, 1.0, SR);
        let mut buf = sine_stereo(1000.0, 256, SR);
        f.process(&mut buf, 256);

        // State should be non-zero after processing.
        assert!(f.x1[0] != 0.0 || f.y1[0] != 0.0);

        f.reset();
        assert_eq!(f.x1, [0.0; 2]);
        assert_eq!(f.x2, [0.0; 2]);
        assert_eq!(f.y1, [0.0; 2]);
        assert_eq!(f.y2, [0.0; 2]);
    }

    #[test]
    fn dc_passthrough_for_flat_peaking() {
        // DC signal (all 1.0) through a 0 dB peaking filter → output ≈ 1.0.
        let mut f = BiquadFilter::peaking(1000.0, 0.0, 1.0, SR);
        let mut buf = vec![1.0f32; 512];
        f.process(&mut buf, 256);

        // After transient, should converge to ~1.0.
        for &s in &buf[400..] {
            assert!(
                (s - 1.0).abs() < 0.01,
                "DC passthrough failed: got {s}"
            );
        }
    }

    #[test]
    fn update_coefficients_preserves_state() {
        let mut f = BiquadFilter::peaking(1000.0, 6.0, 1.0, SR);
        let mut buf = sine_stereo(1000.0, 128, SR);
        f.process(&mut buf, 128);

        let y1_before = f.y1;
        // Update to 0 dB — state should still be intact.
        let flat = BiquadFilter::peaking(1000.0, 0.0, 1.0, SR);
        f.update_coefficients(flat.b0, flat.b1, flat.b2, flat.a1, flat.a2);
        assert_eq!(f.y1, y1_before);
    }
}
