use std::collections::HashMap;

use crate::manifest::EffectManifest;
use crate::processor::EffectProcessor;

/// Factory registry for discovering and creating effect instances.
///
/// Effects are registered with a factory closure keyed by their manifest id.
/// The registry can then list available effects and instantiate them on demand.
pub struct EffectsRegistry {
    factories: HashMap<String, Box<dyn Fn() -> Box<dyn EffectProcessor>>>,
    manifests: HashMap<String, EffectManifest>,
}

impl EffectsRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            manifests: HashMap::new(),
        }
    }

    /// Register an effect factory.
    ///
    /// The factory is called once immediately to capture the manifest,
    /// then stored for future instantiation.
    pub fn register<F>(&mut self, id: &str, factory: F)
    where
        F: Fn() -> Box<dyn EffectProcessor> + 'static,
    {
        // Create a temporary instance to grab the manifest.
        let sample = factory();
        let manifest = sample.manifest().clone();
        self.manifests.insert(id.to_string(), manifest);
        self.factories.insert(id.to_string(), Box::new(factory));
    }

    /// Create a new instance of the effect with the given `id`.
    pub fn create(&self, id: &str) -> Option<Box<dyn EffectProcessor>> {
        self.factories.get(id).map(|f| f())
    }

    /// List manifests of all registered effects.
    pub fn list(&self) -> Vec<&EffectManifest> {
        self.manifests.values().collect()
    }

    /// Look up a single manifest by id.
    pub fn manifest(&self, id: &str) -> Option<&EffectManifest> {
        self.manifests.get(id)
    }

    /// Number of registered effects.
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl Default for EffectsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{EffectManifest, ParameterManifest, ParameterType};
    use crate::params::EffectParams;
    use rustymixer_core::audio::SampleRate;

    /// Dummy effect for testing the registry.
    struct DummyEffect {
        manifest: EffectManifest,
    }

    impl DummyEffect {
        fn new(id: &str, name: &str) -> Self {
            Self {
                manifest: EffectManifest {
                    id: id.into(),
                    name: name.into(),
                    description: "A dummy effect".into(),
                    author: "test".into(),
                    parameters: vec![ParameterManifest {
                        id: "level".into(),
                        name: "Level".into(),
                        min: 0.0,
                        max: 1.0,
                        default: 0.5,
                        param_type: ParameterType::Knob,
                    }],
                },
            }
        }
    }

    impl EffectProcessor for DummyEffect {
        fn manifest(&self) -> &EffectManifest {
            &self.manifest
        }
        fn process(
            &mut self,
            input: &[f32],
            output: &mut [f32],
            frames: usize,
            _sr: SampleRate,
            _params: &EffectParams,
        ) {
            output[..frames * 2].copy_from_slice(&input[..frames * 2]);
        }
        fn reset(&mut self) {}
    }

    #[test]
    fn register_and_create() {
        let mut reg = EffectsRegistry::new();
        reg.register("test:dummy", || {
            Box::new(DummyEffect::new("test:dummy", "Dummy"))
        });

        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());

        let effect = reg.create("test:dummy").expect("should create");
        assert_eq!(effect.manifest().id, "test:dummy");
        assert_eq!(effect.manifest().name, "Dummy");
    }

    #[test]
    fn create_unknown_returns_none() {
        let reg = EffectsRegistry::new();
        assert!(reg.create("nonexistent").is_none());
    }

    #[test]
    fn list_manifests() {
        let mut reg = EffectsRegistry::new();
        reg.register("test:a", || Box::new(DummyEffect::new("test:a", "A")));
        reg.register("test:b", || Box::new(DummyEffect::new("test:b", "B")));

        let manifests = reg.list();
        assert_eq!(manifests.len(), 2);

        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"test:a"));
        assert!(ids.contains(&"test:b"));
    }

    #[test]
    fn manifest_lookup() {
        let mut reg = EffectsRegistry::new();
        reg.register("test:echo", || {
            Box::new(DummyEffect::new("test:echo", "Echo"))
        });

        let m = reg.manifest("test:echo").unwrap();
        assert_eq!(m.name, "Echo");
        assert!(reg.manifest("missing").is_none());
    }

    #[test]
    fn create_returns_fresh_instances() {
        let mut reg = EffectsRegistry::new();
        reg.register("test:fx", || Box::new(DummyEffect::new("test:fx", "FX")));

        let a = reg.create("test:fx").unwrap();
        let b = reg.create("test:fx").unwrap();
        // Both are independent instances (different pointers).
        let ptr_a = &*a as *const dyn EffectProcessor as *const u8;
        let ptr_b = &*b as *const dyn EffectProcessor as *const u8;
        assert_ne!(ptr_a, ptr_b);
    }
}
