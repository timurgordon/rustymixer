/// Runtime parameter values for an effect instance.
///
/// Values are indexed by parameter order in the corresponding [`EffectManifest`].
#[derive(Debug, Clone)]
pub struct EffectParams {
    values: Vec<f64>,
}

impl EffectParams {
    /// Create a new `EffectParams` with the given number of parameters, all set to `0.0`.
    pub fn new(count: usize) -> Self {
        Self {
            values: vec![0.0; count],
        }
    }

    /// Create params pre-filled with the provided default values.
    pub fn with_defaults(defaults: &[f64]) -> Self {
        Self {
            values: defaults.to_vec(),
        }
    }

    /// Get the value at `index`. Returns `0.0` if out of range.
    pub fn get(&self, index: usize) -> f64 {
        self.values.get(index).copied().unwrap_or(0.0)
    }

    /// Set the value at `index`. Does nothing if out of range.
    pub fn set(&mut self, index: usize, value: f64) {
        if let Some(slot) = self.values.get_mut(index) {
            *slot = value;
        }
    }

    /// Number of parameter slots.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no parameter slots.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_new_zeroed() {
        let p = EffectParams::new(3);
        assert_eq!(p.len(), 3);
        assert_eq!(p.get(0), 0.0);
        assert_eq!(p.get(1), 0.0);
        assert_eq!(p.get(2), 0.0);
    }

    #[test]
    fn params_with_defaults() {
        let p = EffectParams::with_defaults(&[1.0, 2.5, 0.5]);
        assert_eq!(p.get(0), 1.0);
        assert_eq!(p.get(1), 2.5);
        assert_eq!(p.get(2), 0.5);
    }

    #[test]
    fn params_get_out_of_range() {
        let p = EffectParams::new(2);
        assert_eq!(p.get(99), 0.0);
    }

    #[test]
    fn params_set() {
        let mut p = EffectParams::new(2);
        p.set(0, 42.0);
        assert_eq!(p.get(0), 42.0);
        assert_eq!(p.get(1), 0.0);
    }

    #[test]
    fn params_set_out_of_range_noop() {
        let mut p = EffectParams::new(1);
        p.set(5, 99.0); // should not panic
        assert_eq!(p.get(0), 0.0);
    }

    #[test]
    fn params_is_empty() {
        assert!(EffectParams::new(0).is_empty());
        assert!(!EffectParams::new(1).is_empty());
    }
}
