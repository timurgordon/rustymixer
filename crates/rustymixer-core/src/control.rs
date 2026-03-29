//! Control object system for UI-to-engine communication.
//!
//! Inspired by Mixxx's ControlObject / ControlProxy pattern.
//!
//! Every controllable parameter is identified by a `(group, key)` pair
//! (e.g. `("[Channel1]", "volume")`). Values are stored as atomic `f64`
//! so the UI and audio threads can communicate without locks.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Identifies a single control parameter by group and key.
///
/// Groups follow Mixxx convention: `"[Channel1]"`, `"[Master]"`, etc.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControlId {
    pub group: String,
    pub key: String,
}

impl ControlId {
    pub fn new(group: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            group: group.into(),
            key: key.into(),
        }
    }
}

impl std::fmt::Display for ControlId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{}", self.group, self.key)
    }
}

/// Atomic f64 value that can be read/written from any thread without locks.
///
/// Uses `AtomicU64` with `f64::to_bits` / `f64::from_bits` for lock-free access.
pub struct ControlValue {
    value: AtomicU64,
}

impl ControlValue {
    pub fn new(initial: f64) -> Self {
        Self {
            value: AtomicU64::new(initial.to_bits()),
        }
    }

    /// Read the current value (lock-free).
    pub fn get(&self) -> f64 {
        f64::from_bits(self.value.load(Ordering::Relaxed))
    }

    /// Write a new value (lock-free).
    pub fn set(&self, value: f64) {
        self.value.store(value.to_bits(), Ordering::Relaxed);
    }
}

impl std::fmt::Debug for ControlValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlValue")
            .field("value", &self.get())
            .finish()
    }
}

/// Min/max/default range for a control parameter.
#[derive(Debug, Clone, Copy)]
pub struct ControlRange {
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

impl ControlRange {
    pub fn new(min: f64, max: f64, default: f64) -> Self {
        debug_assert!(min <= max, "ControlRange: min must be <= max");
        debug_assert!(
            default >= min && default <= max,
            "ControlRange: default must be within [min, max]"
        );
        Self { min, max, default }
    }

    /// Clamp a value into this range.
    pub fn clamp(&self, value: f64) -> f64 {
        value.clamp(self.min, self.max)
    }
}

/// Owner handle for a registered control. Holds the `Arc<ControlValue>`
/// and optional range for validation.
pub struct ControlHandle {
    id: ControlId,
    value: Arc<ControlValue>,
    range: Option<ControlRange>,
}

impl ControlHandle {
    /// Read the current value.
    pub fn get(&self) -> f64 {
        self.value.get()
    }

    /// Write a new value, clamping to range if one is set.
    pub fn set(&self, value: f64) {
        let clamped = match &self.range {
            Some(range) => range.clamp(value),
            None => value,
        };
        self.value.set(clamped);
    }

    /// Get the control identifier.
    pub fn id(&self) -> &ControlId {
        &self.id
    }

    /// Get the range, if any.
    pub fn range(&self) -> Option<&ControlRange> {
        self.range.as_ref()
    }

    /// Reset value to range default (or 0.0 if no range).
    pub fn reset(&self) {
        let default = self.range.map_or(0.0, |r| r.default);
        self.value.set(default);
    }
}

/// Lightweight read/write proxy. Can be cloned and sent to any thread.
#[derive(Clone)]
pub struct ControlProxy {
    id: ControlId,
    value: Arc<ControlValue>,
}

impl ControlProxy {
    /// Read the current value (lock-free).
    pub fn get(&self) -> f64 {
        self.value.get()
    }

    /// Write a new value (lock-free, no range clamping).
    pub fn set(&self, value: f64) {
        self.value.set(value);
    }

    /// Get the control identifier.
    pub fn id(&self) -> &ControlId {
        &self.id
    }
}

impl std::fmt::Debug for ControlProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlProxy")
            .field("id", &self.id)
            .field("value", &self.get())
            .finish()
    }
}

/// Central registry of all controls. Created at startup, shared across threads.
pub struct ControlRegistry {
    controls: HashMap<ControlId, Arc<ControlValue>>,
    ranges: HashMap<ControlId, ControlRange>,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self {
            controls: HashMap::new(),
            ranges: HashMap::new(),
        }
    }

    /// Register a new control with a default value and optional range.
    /// Returns a [`ControlHandle`] for the owner.
    ///
    /// # Panics
    /// Panics if a control with the same id is already registered.
    pub fn register(
        &mut self,
        id: ControlId,
        default: f64,
        range: Option<ControlRange>,
    ) -> ControlHandle {
        assert!(
            !self.controls.contains_key(&id),
            "control already registered: {}",
            id
        );

        let initial = match &range {
            Some(r) => r.clamp(default),
            None => default,
        };
        let value = Arc::new(ControlValue::new(initial));
        self.controls.insert(id.clone(), Arc::clone(&value));
        if let Some(r) = range {
            self.ranges.insert(id.clone(), r);
        }

        ControlHandle {
            id,
            value,
            range,
        }
    }

    /// Get a [`ControlProxy`] for reading/writing a control from any thread.
    pub fn proxy(&self, id: &ControlId) -> Option<ControlProxy> {
        self.controls.get(id).map(|value| ControlProxy {
            id: id.clone(),
            value: Arc::clone(value),
        })
    }

    /// Get a direct `Arc<ControlValue>` reference (for the audio thread).
    pub fn get(&self, id: &ControlId) -> Option<&Arc<ControlValue>> {
        self.controls.get(id)
    }

    /// Check whether a control is registered.
    pub fn contains(&self, id: &ControlId) -> bool {
        self.controls.contains_key(id)
    }

    /// Number of registered controls.
    pub fn len(&self) -> usize {
        self.controls.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

impl Default for ControlRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Register the standard set of DJ mixer controls into a [`ControlRegistry`].
///
/// Returns a vec of all [`ControlHandle`]s so the engine can hold onto them.
pub fn register_standard_controls(registry: &mut ControlRegistry) -> Vec<ControlHandle> {
    let mut handles = Vec::new();

    // Master controls
    handles.push(registry.register(
        ControlId::new("[Master]", "crossfader"),
        0.0,
        Some(ControlRange::new(-1.0, 1.0, 0.0)),
    ));
    handles.push(registry.register(
        ControlId::new("[Master]", "volume"),
        0.8,
        Some(ControlRange::new(0.0, 1.0, 0.8)),
    ));

    // Per-channel controls
    for ch in &["[Channel1]", "[Channel2]"] {
        handles.push(registry.register(
            ControlId::new(*ch, "volume"),
            0.8,
            Some(ControlRange::new(0.0, 1.0, 0.8)),
        ));
        handles.push(registry.register(
            ControlId::new(*ch, "play"),
            0.0,
            Some(ControlRange::new(0.0, 1.0, 0.0)),
        ));
        handles.push(registry.register(
            ControlId::new(*ch, "cue_point"),
            0.0,
            None,
        ));
        handles.push(registry.register(
            ControlId::new(*ch, "playposition"),
            0.0,
            Some(ControlRange::new(0.0, 1.0, 0.0)),
        ));
        handles.push(registry.register(
            ControlId::new(*ch, "rate"),
            1.0,
            Some(ControlRange::new(0.5, 2.0, 1.0)),
        ));
        handles.push(registry.register(
            ControlId::new(*ch, "orientation"),
            1.0,
            Some(ControlRange::new(0.0, 2.0, 1.0)),
        ));
        handles.push(registry.register(
            ControlId::new(*ch, "track_loaded"),
            0.0,
            Some(ControlRange::new(0.0, 1.0, 0.0)),
        ));
        handles.push(registry.register(
            ControlId::new(*ch, "duration"),
            0.0,
            None,
        ));
    }

    handles
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    // --- ControlValue tests ---

    #[test]
    fn control_value_get_set() {
        let cv = ControlValue::new(0.5);
        assert_eq!(cv.get(), 0.5);
        cv.set(1.0);
        assert_eq!(cv.get(), 1.0);
        cv.set(-0.25);
        assert_eq!(cv.get(), -0.25);
    }

    #[test]
    fn control_value_atomic_multithreaded() {
        let cv = Arc::new(ControlValue::new(0.0));
        let cv_writer = Arc::clone(&cv);
        let cv_reader = Arc::clone(&cv);

        let writer = thread::spawn(move || {
            for i in 0..10_000 {
                cv_writer.set(i as f64);
            }
        });

        let reader = thread::spawn(move || {
            let mut last = -1.0_f64;
            for _ in 0..10_000 {
                let val = cv_reader.get();
                // Value must always be a valid f64 (no torn reads)
                assert!(val.is_finite());
                // Values should be monotonically non-decreasing within reason,
                // but we can't guarantee strict ordering with Relaxed — just
                // check they're in valid range.
                assert!(val >= 0.0 && val < 10_000.0);
                if val > last {
                    last = val;
                }
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
        // After writer is done, value should be 9999
        assert_eq!(cv.get(), 9999.0);
    }

    // --- ControlRange tests ---

    #[test]
    fn control_range_clamp() {
        let range = ControlRange::new(0.0, 1.0, 0.5);
        assert_eq!(range.clamp(0.5), 0.5);
        assert_eq!(range.clamp(-0.1), 0.0);
        assert_eq!(range.clamp(1.5), 1.0);
        assert_eq!(range.clamp(0.0), 0.0);
        assert_eq!(range.clamp(1.0), 1.0);
    }

    // --- ControlId tests ---

    #[test]
    fn control_id_equality_and_hash() {
        let a = ControlId::new("[Channel1]", "volume");
        let b = ControlId::new("[Channel1]", "volume");
        let c = ControlId::new("[Channel2]", "volume");
        assert_eq!(a, b);
        assert_ne!(a, c);

        // Works as HashMap key
        let mut map = HashMap::new();
        map.insert(a.clone(), 42);
        assert_eq!(map.get(&b), Some(&42));
        assert_eq!(map.get(&c), None);
    }

    #[test]
    fn control_id_display() {
        let id = ControlId::new("[Master]", "crossfader");
        assert_eq!(format!("{}", id), "[Master],crossfader");
    }

    // --- ControlRegistry tests ---

    #[test]
    fn registry_register_and_lookup() {
        let mut reg = ControlRegistry::new();
        let handle = reg.register(
            ControlId::new("[Master]", "volume"),
            0.8,
            Some(ControlRange::new(0.0, 1.0, 0.8)),
        );

        assert_eq!(handle.get(), 0.8);
        assert_eq!(reg.len(), 1);
        assert!(reg.contains(&ControlId::new("[Master]", "volume")));
        assert!(!reg.contains(&ControlId::new("[Master]", "play")));
    }

    #[test]
    #[should_panic(expected = "control already registered")]
    fn registry_rejects_duplicate() {
        let mut reg = ControlRegistry::new();
        reg.register(ControlId::new("[Master]", "volume"), 0.8, None);
        reg.register(ControlId::new("[Master]", "volume"), 0.5, None);
    }

    #[test]
    fn registry_proxy() {
        let mut reg = ControlRegistry::new();
        let id = ControlId::new("[Channel1]", "volume");
        let _handle = reg.register(id.clone(), 0.8, None);

        let proxy = reg.proxy(&id).expect("proxy should exist");
        assert_eq!(proxy.get(), 0.8);
        assert_eq!(proxy.id(), &id);

        // Proxy for non-existent control
        let missing = reg.proxy(&ControlId::new("[Channel3]", "volume"));
        assert!(missing.is_none());
    }

    #[test]
    fn registry_get_arc() {
        let mut reg = ControlRegistry::new();
        let id = ControlId::new("[Channel1]", "play");
        let _handle = reg.register(id.clone(), 0.0, None);

        let arc = reg.get(&id).expect("arc should exist");
        assert_eq!(arc.get(), 0.0);
    }

    // --- ControlHandle tests ---

    #[test]
    fn handle_clamps_to_range() {
        let mut reg = ControlRegistry::new();
        let handle = reg.register(
            ControlId::new("[Channel1]", "volume"),
            0.8,
            Some(ControlRange::new(0.0, 1.0, 0.8)),
        );

        handle.set(1.5);
        assert_eq!(handle.get(), 1.0); // clamped to max

        handle.set(-0.5);
        assert_eq!(handle.get(), 0.0); // clamped to min

        handle.set(0.5);
        assert_eq!(handle.get(), 0.5); // within range
    }

    #[test]
    fn handle_reset_to_default() {
        let mut reg = ControlRegistry::new();
        let handle = reg.register(
            ControlId::new("[Master]", "crossfader"),
            0.0,
            Some(ControlRange::new(-1.0, 1.0, 0.0)),
        );

        handle.set(0.75);
        assert_eq!(handle.get(), 0.75);

        handle.reset();
        assert_eq!(handle.get(), 0.0);
    }

    // --- ControlProxy tests ---

    #[test]
    fn proxy_clone_and_cross_thread() {
        let mut reg = ControlRegistry::new();
        let id = ControlId::new("[Channel1]", "volume");
        let _handle = reg.register(id.clone(), 0.8, None);

        let proxy1 = reg.proxy(&id).unwrap();
        let proxy2 = proxy1.clone();

        // Mutate from one proxy, read from the other
        proxy1.set(0.42);
        assert_eq!(proxy2.get(), 0.42);

        // Cross-thread
        let proxy_thread = proxy2.clone();
        let join = thread::spawn(move || {
            proxy_thread.set(0.99);
        });
        join.join().unwrap();
        assert_eq!(proxy1.get(), 0.99);
    }

    // --- Shared access between Handle, Proxy, and Arc ---

    #[test]
    fn handle_proxy_arc_share_value() {
        let mut reg = ControlRegistry::new();
        let id = ControlId::new("[Channel2]", "rate");
        let handle = reg.register(id.clone(), 1.0, Some(ControlRange::new(0.5, 2.0, 1.0)));

        let proxy = reg.proxy(&id).unwrap();
        let arc = Arc::clone(reg.get(&id).unwrap());

        // All see the initial value
        assert_eq!(handle.get(), 1.0);
        assert_eq!(proxy.get(), 1.0);
        assert_eq!(arc.get(), 1.0);

        // Handle writes (clamped), others read
        handle.set(1.5);
        assert_eq!(proxy.get(), 1.5);
        assert_eq!(arc.get(), 1.5);

        // Proxy writes (unclamped), handle reads
        proxy.set(0.3);
        assert_eq!(handle.get(), 0.3);
        assert_eq!(arc.get(), 0.3);

        // Direct arc writes, all read
        arc.set(1.75);
        assert_eq!(handle.get(), 1.75);
        assert_eq!(proxy.get(), 1.75);
    }

    // --- Standard controls ---

    #[test]
    fn register_standard_controls_populates_registry() {
        let mut reg = ControlRegistry::new();
        let handles = register_standard_controls(&mut reg);

        // 2 master + 8 per channel * 2 channels = 18 total
        assert_eq!(reg.len(), 18);
        assert_eq!(handles.len(), 18);

        // Spot-check a few
        assert!(reg.contains(&ControlId::new("[Master]", "crossfader")));
        assert!(reg.contains(&ControlId::new("[Master]", "volume")));
        assert!(reg.contains(&ControlId::new("[Channel1]", "volume")));
        assert!(reg.contains(&ControlId::new("[Channel1]", "play")));
        assert!(reg.contains(&ControlId::new("[Channel2]", "rate")));
        assert!(reg.contains(&ControlId::new("[Channel2]", "duration")));
    }

    #[test]
    fn standard_controls_have_correct_defaults() {
        let mut reg = ControlRegistry::new();
        let _handles = register_standard_controls(&mut reg);

        let crossfader = reg.proxy(&ControlId::new("[Master]", "crossfader")).unwrap();
        assert_eq!(crossfader.get(), 0.0);

        let master_vol = reg.proxy(&ControlId::new("[Master]", "volume")).unwrap();
        assert_eq!(master_vol.get(), 0.8);

        let ch1_vol = reg.proxy(&ControlId::new("[Channel1]", "volume")).unwrap();
        assert_eq!(ch1_vol.get(), 0.8);

        let ch2_play = reg.proxy(&ControlId::new("[Channel2]", "play")).unwrap();
        assert_eq!(ch2_play.get(), 0.0);

        let ch1_rate = reg.proxy(&ControlId::new("[Channel1]", "rate")).unwrap();
        assert_eq!(ch1_rate.get(), 1.0);

        let ch2_orientation = reg.proxy(&ControlId::new("[Channel2]", "orientation")).unwrap();
        assert_eq!(ch2_orientation.get(), 1.0); // Center
    }

    // --- Performance ---

    #[test]
    fn control_value_throughput() {
        let cv = Arc::new(ControlValue::new(0.0));

        let start = std::time::Instant::now();
        for i in 0..1_000_000u64 {
            cv.set(i as f64);
            let _ = cv.get();
        }
        let elapsed = start.elapsed();

        // Issue requirement: 1M get/set in < 100ms
        assert!(
            elapsed.as_millis() < 100,
            "1M get/set took {}ms (must be < 100ms)",
            elapsed.as_millis()
        );
    }
}
