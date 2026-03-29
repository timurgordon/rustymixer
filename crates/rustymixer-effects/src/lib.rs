//! Audio effects framework for RustyMixer.
//!
//! Provides a pluggable effects pipeline with:
//! - [`EffectProcessor`] trait for implementing individual effects
//! - [`EffectChain`] for sequencing multiple effects
//! - [`EffectsRegistry`] for discovering and instantiating effects
//! - [`EffectManifest`] / [`EffectParams`] for metadata and runtime control

pub mod biquad;
pub mod eq;
mod chain;
mod manifest;
mod params;
mod processor;
mod registry;

pub use chain::{EffectChain, EffectSlot};
pub use manifest::{EffectManifest, ParameterManifest, ParameterType};
pub use params::EffectParams;
pub use processor::EffectProcessor;
pub use registry::EffectsRegistry;
