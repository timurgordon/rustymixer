/// Describes an effect's identity and parameters.
#[derive(Debug, Clone)]
pub struct EffectManifest {
    /// Unique identifier, e.g. `"builtin:echo"`.
    pub id: String,
    /// Human-readable name, e.g. `"Echo"`.
    pub name: String,
    /// Short description of what the effect does.
    pub description: String,
    /// Author or source of the effect.
    pub author: String,
    /// Declared parameters in display order.
    pub parameters: Vec<ParameterManifest>,
}

/// Describes a single adjustable parameter on an effect.
#[derive(Debug, Clone)]
pub struct ParameterManifest {
    /// Unique key within the effect, e.g. `"delay_ms"`.
    pub id: String,
    /// Human-readable label, e.g. `"Delay"`.
    pub name: String,
    /// Minimum value.
    pub min: f64,
    /// Maximum value.
    pub max: f64,
    /// Default value.
    pub default: f64,
    /// How the parameter is presented in the UI.
    pub param_type: ParameterType,
}

/// How a parameter is presented in the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterType {
    /// Continuous rotary knob — the raw value is mapped to `min..max`.
    Knob,
    /// Toggle button — value is `0.0` (off) or `1.0` (on).
    Button,
}
