//! Crossfader algorithm and curves for blending between left/right decks.
//!
//! Inspired by Mixxx's `enginexfader.h` / `enginexfader.cpp`.

use crate::gain::AtomicF32;
use std::sync::atomic::Ordering;

/// Crossfader curve type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossfaderCurve {
    /// Additive: linear fade. Both sides at half volume when centered.
    /// Left gain = 1 − pos, Right gain = pos (where pos is normalized 0..1).
    Additive,

    /// Constant power: maintains perceived loudness across the fade.
    /// Uses cos/sin so that left² + right² ≈ 1.0 at every position.
    ConstantPower,
}

/// Crossfader that blends audio between left- and right-oriented channels.
///
/// Position ranges from −1.0 (full left) to +1.0 (full right), with 0.0
/// being center. The position is stored atomically so the UI thread can
/// update it while the audio thread reads it.
pub struct Crossfader {
    position: AtomicF32,
    curve: CrossfaderCurve,
}

impl Crossfader {
    /// Create a new crossfader at center position with the given curve.
    pub fn new(curve: CrossfaderCurve) -> Self {
        Self {
            position: AtomicF32::new(0.0),
            curve,
        }
    }

    /// Set the crossfader position (−1.0 to +1.0). Safe to call from any thread.
    pub fn set_position(&self, position: f32) {
        self.position
            .store(position.clamp(-1.0, 1.0), Ordering::Relaxed);
    }

    /// Current crossfader position.
    pub fn position(&self) -> f32 {
        self.position.load(Ordering::Relaxed)
    }

    /// Set the crossfader curve. Must be called from the non-RT thread before
    /// processing (not safe to change mid-callback without synchronization).
    pub fn set_curve(&mut self, curve: CrossfaderCurve) {
        self.curve = curve;
    }

    /// Current curve type.
    pub fn curve(&self) -> CrossfaderCurve {
        self.curve
    }

    /// Calculate gain multipliers for left- and right-oriented channels.
    /// Returns `(left_gain, right_gain)`.
    pub fn gains(&self) -> (f32, f32) {
        let pos = (self.position.load(Ordering::Relaxed) + 1.0) / 2.0; // normalize to 0.0..1.0
        match self.curve {
            CrossfaderCurve::Additive => {
                let left = 1.0 - pos;
                let right = pos;
                (left, right)
            }
            CrossfaderCurve::ConstantPower => {
                let angle = pos * std::f32::consts::FRAC_PI_2;
                let left = angle.cos();
                let right = angle.sin();
                (left, right)
            }
        }
    }
}

impl Default for Crossfader {
    fn default() -> Self {
        Self::new(CrossfaderCurve::Additive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 0.001;

    // --- Additive curve ---

    #[test]
    fn additive_full_left() {
        let xf = Crossfader::new(CrossfaderCurve::Additive);
        xf.set_position(-1.0);
        let (l, r) = xf.gains();
        assert!((l - 1.0).abs() < EPS, "left={l}");
        assert!(r.abs() < EPS, "right={r}");
    }

    #[test]
    fn additive_center() {
        let xf = Crossfader::new(CrossfaderCurve::Additive);
        xf.set_position(0.0);
        let (l, r) = xf.gains();
        assert!((l - 0.5).abs() < EPS, "left={l}");
        assert!((r - 0.5).abs() < EPS, "right={r}");
    }

    #[test]
    fn additive_full_right() {
        let xf = Crossfader::new(CrossfaderCurve::Additive);
        xf.set_position(1.0);
        let (l, r) = xf.gains();
        assert!(l.abs() < EPS, "left={l}");
        assert!((r - 1.0).abs() < EPS, "right={r}");
    }

    // --- Constant power curve ---

    #[test]
    fn constant_power_full_left() {
        let xf = Crossfader::new(CrossfaderCurve::ConstantPower);
        xf.set_position(-1.0);
        let (l, r) = xf.gains();
        assert!((l - 1.0).abs() < EPS, "left={l}");
        assert!(r.abs() < EPS, "right={r}");
    }

    #[test]
    fn constant_power_full_right() {
        let xf = Crossfader::new(CrossfaderCurve::ConstantPower);
        xf.set_position(1.0);
        let (l, r) = xf.gains();
        assert!(l.abs() < EPS, "left={l}");
        assert!((r - 1.0).abs() < EPS, "right={r}");
    }

    #[test]
    fn constant_power_center_preserves_power() {
        let xf = Crossfader::new(CrossfaderCurve::ConstantPower);
        xf.set_position(0.0);
        let (l, r) = xf.gains();
        let power = l * l + r * r;
        assert!(
            (power - 1.0).abs() < EPS,
            "power at center should be ~1.0, got {power}"
        );
    }

    #[test]
    fn constant_power_preserves_power_at_all_positions() {
        let xf = Crossfader::new(CrossfaderCurve::ConstantPower);
        for i in 0..=20 {
            let pos = (i as f32 / 10.0) - 1.0; // -1.0 to 1.0
            xf.set_position(pos);
            let (l, r) = xf.gains();
            let power = l * l + r * r;
            assert!(
                (power - 1.0).abs() < EPS,
                "power at pos={pos} should be ~1.0, got {power}"
            );
        }
    }

    // --- Position clamping ---

    #[test]
    fn position_clamps_to_range() {
        let xf = Crossfader::default();
        xf.set_position(-5.0);
        assert!((xf.position() - (-1.0)).abs() < EPS);

        xf.set_position(5.0);
        assert!((xf.position() - 1.0).abs() < EPS);
    }

    // --- Curve switching ---

    #[test]
    fn switch_curve() {
        let mut xf = Crossfader::new(CrossfaderCurve::Additive);
        assert_eq!(xf.curve(), CrossfaderCurve::Additive);

        xf.set_curve(CrossfaderCurve::ConstantPower);
        assert_eq!(xf.curve(), CrossfaderCurve::ConstantPower);
    }
}
