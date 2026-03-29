use std::sync::atomic::{AtomicU32, Ordering};

/// Thread-safe `f32` using atomic bit operations.
///
/// Allows the UI thread to update gain values while the audio thread
/// reads them, without locks.
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(val: f32) -> Self {
        Self(AtomicU32::new(val.to_bits()))
    }

    pub fn load(&self, order: Ordering) -> f32 {
        f32::from_bits(self.0.load(order))
    }

    pub fn store(&self, val: f32, order: Ordering) {
        self.0.store(val.to_bits(), order);
    }
}

/// Apply a constant gain to every sample in the buffer.
#[inline]
pub fn apply_gain(buffer: &mut [f32], gain: f32) {
    if (gain - 1.0).abs() < f32::EPSILON {
        return;
    }
    if gain == 0.0 {
        buffer.fill(0.0);
        return;
    }
    for s in buffer.iter_mut() {
        *s *= gain;
    }
}

/// Linearly ramp gain from `old_gain` to `new_gain` across `frames` stereo
/// frames, preventing audible clicks on gain changes.
#[inline]
pub fn apply_gain_ramped(buffer: &mut [f32], old_gain: f32, new_gain: f32, frames: usize) {
    if (old_gain - new_gain).abs() < f32::EPSILON {
        apply_gain(buffer, new_gain);
        return;
    }

    let step = (new_gain - old_gain) / frames as f32;
    let mut gain = old_gain;

    for frame in 0..frames {
        let i = frame * 2;
        buffer[i] *= gain;
        buffer[i + 1] *= gain;
        gain += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_gain() {
        let mut buf = vec![2.0f32; 8];
        apply_gain(&mut buf, 0.5);
        assert!(buf.iter().all(|&s| (s - 1.0).abs() < f32::EPSILON));
    }

    #[test]
    fn unity_gain_is_noop() {
        let mut buf = vec![0.7f32; 4];
        apply_gain(&mut buf, 1.0);
        assert!(buf.iter().all(|&s| (s - 0.7).abs() < f32::EPSILON));
    }

    #[test]
    fn zero_gain_silences() {
        let mut buf = vec![0.9f32; 6];
        apply_gain(&mut buf, 0.0);
        assert!(buf.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn ramped_gain() {
        // 4 stereo frames, all 1.0 — ramp from 0.0 to 1.0
        let mut buf = vec![1.0f32; 8];
        apply_gain_ramped(&mut buf, 0.0, 1.0, 4);

        // Frame 0: gain = 0.00
        assert!(buf[0].abs() < 0.001);
        assert!(buf[1].abs() < 0.001);
        // Frame 1: gain = 0.25
        assert!((buf[2] - 0.25).abs() < 0.001);
        // Frame 2: gain = 0.50
        assert!((buf[4] - 0.50).abs() < 0.001);
        // Frame 3: gain = 0.75
        assert!((buf[6] - 0.75).abs() < 0.001);
        assert!((buf[7] - 0.75).abs() < 0.001);
    }

    #[test]
    fn ramped_same_gain_is_constant() {
        let mut buf = vec![1.0f32; 8];
        apply_gain_ramped(&mut buf, 0.5, 0.5, 4);
        assert!(buf.iter().all(|&s| (s - 0.5).abs() < f32::EPSILON));
    }

    #[test]
    fn atomic_f32_roundtrip() {
        let a = AtomicF32::new(3.14);
        assert!((a.load(Ordering::Relaxed) - 3.14).abs() < f32::EPSILON);

        a.store(2.718, Ordering::Relaxed);
        assert!((a.load(Ordering::Relaxed) - 2.718).abs() < f32::EPSILON);
    }
}
