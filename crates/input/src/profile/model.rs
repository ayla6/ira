use serde::{Deserialize, Serialize};

pub const PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualGamepadBackend {
    #[default]
    XInput,
    DirectInput,
    SwitchPro,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadButton {
    A,
    B,
    X,
    Y,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    Back,
    Start,
    Guide,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    Paddle1,
    Paddle2,
    Paddle3,
    Paddle4,
    Paddle5,
    Paddle6,
    Paddle7,
    Paddle8,
}

impl GamepadButton {
    pub fn is_xinput(self) -> bool {
        !self.is_paddle()
    }

    pub fn is_paddle(self) -> bool {
        matches!(
            self,
            Self::Paddle1
                | Self::Paddle2
                | Self::Paddle3
                | Self::Paddle4
                | Self::Paddle5
                | Self::Paddle6
                | Self::Paddle7
                | Self::Paddle8
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Negative,
    Positive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GyroCalibration {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub z: f32,
}

/// Whole-controller gyro behaviour. The old model exposed each gyro axis as
/// three separate bindings the user had to wire up by hand; this config is
/// the single source of truth the engine feeds with player-space yaw/pitch
/// rates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct GyroConfig {
    pub enabled: bool,
    pub activation: GyroActivation,
    pub output: GyroOutput,
    /// Sensitivity multiplier around 1.0, applied to both axes.
    pub sensitivity: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    /// Adaptive smoothing: damps jitter during fine aim, never touches flicks.
    pub smoothing: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroActivation {
    #[default]
    Always,
    Hold(GamepadButton),
    Toggle(GamepadButton),
}

impl GyroActivation {
    /// Button that enables gyro, when activation is button-driven.
    pub fn button(self) -> Option<GamepadButton> {
        match self {
            Self::Always => None,
            Self::Hold(button) | Self::Toggle(button) => Some(button),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroOutput {
    #[default]
    Mouse,
    LeftStick,
    RightStick,
}

impl Default for GyroConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            activation: GyroActivation::Always,
            output: GyroOutput::Mouse,
            sensitivity: 1.0,
            invert_x: false,
            invert_y: false,
            smoothing: true,
        }
    }
}

impl GyroConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if !self.sensitivity.is_finite() || !(0.05..=20.0).contains(&self.sensitivity) {
            return Err("gyro sensitivity must be finite and within [0.05, 20]".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSource {
    Button(GamepadButton),
    Axis(GamepadAxis),
    AxisDirection {
        axis: GamepadAxis,
        direction: AxisDirection,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputCategory {
    Buttons,
    Dpad,
    Triggers,
    Joysticks,
    Gyro,
}

impl InputSource {
    pub fn category(self) -> InputCategory {
        match self {
            InputSource::Button(GamepadButton::LeftTrigger | GamepadButton::RightTrigger) => {
                InputCategory::Triggers
            }
            InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
            | InputSource::AxisDirection {
                axis: GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger,
                ..
            } => InputCategory::Triggers,
            InputSource::Axis(_) | InputSource::AxisDirection { .. } => InputCategory::Joysticks,
            InputSource::Button(
                GamepadButton::DpadUp
                | GamepadButton::DpadDown
                | GamepadButton::DpadLeft
                | GamepadButton::DpadRight,
            ) => InputCategory::Dpad,
            InputSource::Button(_) => InputCategory::Buttons,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAxis {
    X,
    Y,
    Wheel,
    WheelX,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Side,
    Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputAction {
    GamepadButton(GamepadButton),
    GamepadAxis(GamepadAxis),
    Keyboard { keycode: u16 },
    MouseButton(MouseButton),
    MouseAxis(MouseAxis),
    /// Engine-internal: switch the active action set. Never reaches virtual
    /// devices.
    SwitchActionSet(usize),
    /// Engine-internal: overlay an action set layer while held or until
    /// toggled off.
    EnableLayer {
        layer: usize,
        #[serde(default)]
        mode: ChordMode,
    },
    /// Engine-internal: while the activator's condition holds, `target`
    /// uses the referenced mode shift.
    ModeShiftActivate { target: InputSource },
}

impl OutputAction {
    pub fn is_xinput_compatible(&self) -> bool {
        match self {
            Self::GamepadButton(button) => button.is_xinput(),
            Self::GamepadAxis(_)
            | Self::SwitchActionSet(_)
            | Self::EnableLayer { .. }
            | Self::ModeShiftActivate { .. } => true,
            Self::Keyboard { .. } | Self::MouseButton(_) | Self::MouseAxis(_) => false,
        }
    }

    pub fn is_supported(&self) -> bool {
        self.is_supported_by(VirtualGamepadBackend::XInput)
    }

    pub fn is_supported_by(&self, backend: VirtualGamepadBackend) -> bool {
        match backend {
            VirtualGamepadBackend::XInput => {
                !matches!(self, Self::GamepadButton(button) if button.is_paddle())
            }
            VirtualGamepadBackend::DirectInput => true,
            VirtualGamepadBackend::SwitchPro => {
                !matches!(self, Self::GamepadButton(button) if button.is_paddle())
                    && !matches!(
                        self,
                        Self::GamepadAxis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
                    )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisTransform {
    #[serde(default)]
    pub dead_zone: f32,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,
    #[serde(default = "default_exponent")]
    pub exponent: f32,
    #[serde(default)]
    pub invert: bool,
}

impl Default for AxisTransform {
    fn default() -> Self {
        Self {
            dead_zone: 0.0,
            sensitivity: default_sensitivity(),
            exponent: default_exponent(),
            invert: false,
        }
    }
}

impl AxisTransform {
    pub fn validate(self) -> Result<(), String> {
        if !self.dead_zone.is_finite() || !(0.0..1.0).contains(&self.dead_zone) {
            return Err("dead_zone must be finite and in [0, 1)".to_string());
        }
        if !self.sensitivity.is_finite() || self.sensitivity < 0.0 {
            return Err("sensitivity must be finite and non-negative".to_string());
        }
        if !self.exponent.is_finite() || self.exponent <= 0.0 {
            return Err("exponent must be finite and positive".to_string());
        }
        Ok(())
    }

    pub fn apply(self, value: f32) -> f32 {
        self.apply_unbounded(value).clamp(-1.0, 1.0)
    }

    pub fn apply_unbounded(self, value: f32) -> f32 {
        let magnitude = value.abs();
        if magnitude <= self.dead_zone {
            return 0.0;
        }
        let normalized = (magnitude - self.dead_zone) / (1.0 - self.dead_zone);
        let curved = normalized.powf(self.exponent) * self.sensitivity;
        let signed = curved.copysign(value);
        if self.invert {
            -signed
        } else {
            signed
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    #[default]
    Always,
    Hold(InputSource),
    Toggle(InputSource),
    DisableWhile(InputSource),
    Chord {
        sources: Vec<InputSource>,
        #[serde(default)]
        mode: ChordMode,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordMode {
    #[default]
    Hold,
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub source: InputSource,
    pub output: OutputAction,
    #[serde(default)]
    pub activation: Activation,
    #[serde(default)]
    pub transform: AxisTransform,
}

impl Binding {
    pub fn new(source: InputSource, output: OutputAction) -> Self {
        Self {
            source,
            output,
            activation: Activation::Always,
            transform: AxisTransform::default(),
        }
    }
}

/// A named group of input mappings. Profile action set `[0]` is the default;
/// higher sets are switched to by bindings with
/// [`OutputAction::SwitchActionSet`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSet {
    pub name: String,
    #[serde(default)]
    pub inputs: Vec<InputMapping>,
}

/// Additive override applied on top of a parent action set while a layer
/// binding is held or toggled on: inputs defined here replace the parent's
/// mapping for the same source; everything else falls through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSetLayer {
    pub name: String,
    /// Name of the action set this layer applies to.
    pub parent_set: String,
    #[serde(default)]
    pub inputs: Vec<InputMapping>,
}

/// One physical input and everything it does, mirroring Steam Input's
/// per-input model instead of a flat binding list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputMapping {
    pub source: InputSource,
    /// Analog sources get an output mode; button sources leave this `None`.
    #[serde(default)]
    pub mode: Option<SourceMode>,
    #[serde(default)]
    pub mode_shifts: Vec<ModeShift>,
    #[serde(default)]
    pub activators: Vec<Activator>,
}

impl InputMapping {
    pub fn new(source: InputSource) -> Self {
        Self {
            source,
            mode: None,
            mode_shifts: Vec::new(),
            activators: Vec::new(),
        }
    }

    /// Convenience constructor: a single full-press activator, the shape all
    /// identity bindings migrate to.
    pub fn simple(source: InputSource, output: OutputAction) -> Self {
        let mut mapping = Self::new(source);
        mapping.activators.push(Activator::full_press(vec![output]));
        mapping
    }
}

/// What an analog input (stick, trigger) drives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    Joystick {
        output: StickOutput,
        #[serde(default = "default_deadzone_inner")]
        deadzone_inner: f32,
        #[serde(default = "default_deadzone_outer")]
        deadzone_outer: f32,
        #[serde(default = "default_exponent")]
        curve: f32,
    },
    Mouse {
        #[serde(default = "default_sensitivity")]
        sensitivity: f32,
    },
    /// Reserved for the phase-5 flickstick implementation; parseable now so
    /// imported profiles round-trip.
    Flickstick {
        #[serde(default = "default_flickstick_rotation")]
        rotation_sensitivity: f32,
        #[serde(default = "default_flickstick_flick_ms")]
        flick_duration_ms: u32,
    },
    Trigger {
        #[serde(default = "default_soft_threshold")]
        threshold: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickOutput {
    Left,
    Right,
}

impl SourceMode {
    pub fn validate(&self) -> Result<(), String> {
        let SourceMode::Joystick {
            deadzone_inner,
            deadzone_outer,
            curve,
            ..
        } = self
        else {
            return Ok(());
        };
        if !(0.0..1.0).contains(deadzone_inner) || !deadzone_inner.is_finite() {
            return Err("deadzone_inner must be finite and in [0, 1)".to_string());
        }
        if !(0.0..=1.0).contains(deadzone_outer) || !deadzone_outer.is_finite() {
            return Err("deadzone_outer must be finite and in (0, 1]".to_string());
        }
        if deadzone_inner >= deadzone_outer {
            return Err("deadzone_inner must be below deadzone_outer".to_string());
        }
        if !curve.is_finite() || *curve <= 0.0 {
            return Err("curve exponent must be finite and positive".to_string());
        }
        Ok(())
    }
}

/// While `trigger` is held, the source uses this shift's mode/activators
/// instead of its own — Steam Input's mode shift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeShift {
    pub trigger: InputSource,
    #[serde(default)]
    pub mode: Option<SourceMode>,
    #[serde(default)]
    pub activators: Vec<Activator>,
}

/// One press pattern an input recognizes and the outputs it fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activator {
    pub kind: ActivatorKind,
    #[serde(default)]
    pub outputs: Vec<OutputAction>,
    /// Gating that must hold for the activator to participate; carried over
    /// from the flat-binding model.
    #[serde(default)]
    pub activation: Activation,
    #[serde(default)]
    pub settings: ActivatorSettings,
}

impl Activator {
    pub fn full_press(outputs: Vec<OutputAction>) -> Self {
        Self {
            kind: ActivatorKind::FullPress,
            outputs,
            activation: Activation::Always,
            settings: ActivatorSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivatorKind {
    FullPress,
    DoublePress {
        #[serde(default = "default_double_press_ms")]
        window_ms: u32,
    },
    LongPress {
        #[serde(default = "default_long_press_ms")]
        duration_ms: u32,
    },
    StartPress,
    Release,
    /// Analog threshold crossing (triggers).
    SoftPress {
        #[serde(default = "default_soft_threshold")]
        threshold: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ActivatorSettings {
    /// Fire again every N ms while the activator's condition keeps holding.
    pub repeat_rate_ms: Option<u32>,
    /// Press-on, press-again-off instead of hold-to-fire.
    pub toggle: bool,
    /// A later press pattern may cancel this activator before it completes.
    pub interruptable: bool,
}

impl Default for ActivatorSettings {
    fn default() -> Self {
        Self {
            repeat_rate_ms: None,
            toggle: false,
            interruptable: true,
        }
    }
}

fn default_deadzone_inner() -> f32 {
    0.1
}

fn default_deadzone_outer() -> f32 {
    0.95
}

fn default_double_press_ms() -> u32 {
    320
}

fn default_long_press_ms() -> u32 {
    600
}

fn default_soft_threshold() -> f32 {
    0.5
}

fn default_flickstick_rotation() -> f32 {
    1.0
}

fn default_flickstick_flick_ms() -> u32 {
    120
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputProfile {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default = "default_profile_name")]
    pub name: String,
    #[serde(default)]
    pub backend: VirtualGamepadBackend,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub gyro_calibration: GyroCalibration,
    #[serde(default)]
    pub gyro: GyroConfig,
    /// Action-set model. Empty while a profile still uses the flat `bindings`
    /// form; loaders convert bindings to a single default action set.
    #[serde(default)]
    pub action_sets: Vec<ActionSet>,
    #[serde(default)]
    pub action_layers: Vec<ActionSetLayer>,
    /// Internal Ira game IDs this profile has been assigned to.
    /// Empty means the profile is available to every game.
    #[serde(default)]
    pub compatible_game_ids: Vec<i64>,
}

impl Default for InputProfile {
    fn default() -> Self {
        Self {
            version: PROFILE_VERSION,
            name: default_profile_name(),
            backend: VirtualGamepadBackend::default(),
            bindings: Vec::new(),
            gyro_calibration: GyroCalibration::default(),
            gyro: GyroConfig::default(),
            action_sets: Vec::new(),
            action_layers: Vec::new(),
            compatible_game_ids: Vec::new(),
        }
    }
}

impl InputProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != PROFILE_VERSION {
            return Err(format!(
                "unsupported input profile version {} (expected {})",
                self.version, PROFILE_VERSION
            ));
        }
        for (index, binding) in self.bindings.iter().enumerate() {
            if !binding.output.is_supported_by(self.backend) {
                return Err(format!(
                    "binding {index}: output is not supported by Ira's virtual input devices"
                ));
            }
            binding
                .transform
                .validate()
                .map_err(|error| format!("binding {index}: {error}"))?;
            if let Activation::Chord { sources, .. } = &binding.activation {
                if sources.is_empty() {
                    return Err(format!("binding {index}: chord cannot be empty"));
                }
            }
        }
        self.gyro.validate()?;
        self.validate_action_sets()?;
        Ok(())
    }

    fn validate_action_sets(&self) -> Result<(), String> {
        if self.action_sets.is_empty() {
            return Ok(());
        }
        for (index, output) in self.all_activator_outputs().enumerate() {
            if !output.is_supported_by(self.backend) {
                return Err(format!(
                    "action set entry {index}: output is not supported by Ira's virtual input devices"
                ));
            }
        }
        for (set_index, set) in self.action_sets.iter().enumerate() {
            if set.name.trim().is_empty() {
                return Err(format!("action set {set_index} needs a name"));
            }
            for (input_index, input) in set.inputs.iter().enumerate() {
                self.validate_input_mapping(&set.name, input_index, input)?;
            }
        }
        for (layer_index, layer) in self.action_layers.iter().enumerate() {
            if !self.action_sets.iter().any(|set| set.name == layer.parent_set) {
                return Err(format!(
                    "action layer {layer_index} references unknown parent set '{}'",
                    layer.parent_set
                ));
            }
            for (input_index, input) in layer.inputs.iter().enumerate() {
                self.validate_input_mapping(&layer.name, input_index, input)?;
            }
        }
        Ok(())
    }

    fn validate_input_mapping(
        &self,
        context: &str,
        input_index: usize,
        input: &InputMapping,
    ) -> Result<(), String> {
        let label = format!("{context} input {input_index}");
        if let Some(mode) = &input.mode {
            mode.validate()
                .map_err(|error| format!("{label}: {error}"))?;
        }
        // Button inputs express everything through activators; mode-driven
        // analog inputs (sticks, triggers) work without any.
        if input.mode.is_none() && input.activators.is_empty() {
            return Err(format!("{label}: needs at least one activator"));
        }
        for activator in &input.activators {
            if activator.outputs.is_empty() {
                return Err(format!("{label}: activator needs at least one output"));
            }
            if let ActivatorKind::SoftPress { threshold } = activator.kind {
                if !(0.0..1.0).contains(&threshold) || !threshold.is_finite() {
                    return Err(format!("{label}: soft-press threshold must be in [0, 1)"));
                }
            }
            for output in &activator.outputs {
                match output {
                    OutputAction::SwitchActionSet(target)
                        if *target >= self.action_sets.len() =>
                    {
                        return Err(format!(
                            "{label}: switch-action-set target {target} out of range"
                        ));
                    }
                    OutputAction::EnableLayer { layer, .. }
                        if *layer >= self.action_layers.len() =>
                    {
                        return Err(format!(
                            "{label}: enable-layer target {layer} out of range"
                        ));
                    }
                    _ => {}
                }
            }
        }
        for (shift_index, shift) in input.mode_shifts.iter().enumerate() {
            if !matches!(shift.trigger, InputSource::Button(_)) {
                return Err(format!(
                    "{label}: mode shift {shift_index} trigger must be a button"
                ));
            }
        }
        Ok(())
    }

    /// Every output fired by any activator anywhere in the profile.
    pub fn all_activator_outputs(&self) -> impl Iterator<Item = &OutputAction> {
        let mut outputs: Vec<&OutputAction> = self
            .bindings
            .iter()
            .map(|binding| &binding.output)
            .collect();
        for input in self
            .action_sets
            .iter()
            .flat_map(|set| set.inputs.iter())
            .chain(self.action_layers.iter().flat_map(|layer| layer.inputs.iter()))
        {
            for activator in input
                .activators
                .iter()
                .chain(input.mode_shifts.iter().flat_map(|shift| shift.activators.iter()))
            {
                outputs.extend(activator.outputs.iter());
            }
        }
        outputs.into_iter()
    }

    pub fn default_gamepad() -> Self {
        Self::default_gamepad_for_backend(VirtualGamepadBackend::XInput)
    }

    pub fn default_gamepad_for_buttons(supported_buttons: &[GamepadButton]) -> Self {
        Self::default_gamepad_for_backend_and_buttons(
            VirtualGamepadBackend::XInput,
            supported_buttons,
        )
    }

    pub fn default_gamepad_for_backend(backend: VirtualGamepadBackend) -> Self {
        let mut buttons = vec![
            GamepadButton::A,
            GamepadButton::B,
            GamepadButton::X,
            GamepadButton::Y,
            GamepadButton::LeftShoulder,
            GamepadButton::RightShoulder,
            GamepadButton::Back,
            GamepadButton::Start,
            GamepadButton::Guide,
            GamepadButton::LeftStick,
            GamepadButton::RightStick,
            GamepadButton::DpadUp,
            GamepadButton::DpadDown,
            GamepadButton::DpadLeft,
            GamepadButton::DpadRight,
        ];
        if backend == VirtualGamepadBackend::SwitchPro {
            buttons.extend([GamepadButton::LeftTrigger, GamepadButton::RightTrigger]);
        }
        if backend == VirtualGamepadBackend::DirectInput {
            buttons.extend([
                GamepadButton::Paddle1,
                GamepadButton::Paddle2,
                GamepadButton::Paddle3,
                GamepadButton::Paddle4,
                GamepadButton::Paddle5,
                GamepadButton::Paddle6,
                GamepadButton::Paddle7,
                GamepadButton::Paddle8,
            ]);
        }
        Self::default_gamepad_for_backend_and_buttons(backend, &buttons)
    }

    pub fn default_gamepad_for_backend_and_buttons(
        backend: VirtualGamepadBackend,
        supported_buttons: &[GamepadButton],
    ) -> Self {
        let mut profile = Self::default_gamepad_controls(backend);
        profile.bindings.retain(|binding| {
            matches!(binding.source, InputSource::Axis(_))
                || matches!(
                    binding.source,
                    InputSource::Button(button) if supported_buttons.contains(&button)
                )
        });
        profile
    }

    fn default_gamepad_controls(backend: VirtualGamepadBackend) -> Self {
        let mut buttons = vec![
            GamepadButton::A,
            GamepadButton::B,
            GamepadButton::X,
            GamepadButton::Y,
            GamepadButton::LeftShoulder,
            GamepadButton::RightShoulder,
            GamepadButton::Back,
            GamepadButton::Start,
            GamepadButton::Guide,
            GamepadButton::LeftStick,
            GamepadButton::RightStick,
            GamepadButton::DpadUp,
            GamepadButton::DpadDown,
            GamepadButton::DpadLeft,
            GamepadButton::DpadRight,
        ];
        if backend == VirtualGamepadBackend::SwitchPro {
            buttons.extend([GamepadButton::LeftTrigger, GamepadButton::RightTrigger]);
        }
        let mut axes = vec![
            GamepadAxis::LeftX,
            GamepadAxis::LeftY,
            GamepadAxis::RightX,
            GamepadAxis::RightY,
        ];
        if backend != VirtualGamepadBackend::SwitchPro {
            axes.extend([GamepadAxis::LeftTrigger, GamepadAxis::RightTrigger]);
        }
        let paddles = [
            GamepadButton::Paddle1,
            GamepadButton::Paddle2,
            GamepadButton::Paddle3,
            GamepadButton::Paddle4,
            GamepadButton::Paddle5,
            GamepadButton::Paddle6,
            GamepadButton::Paddle7,
            GamepadButton::Paddle8,
        ];
        let mut bindings = Vec::with_capacity(buttons.len() + axes.len() + paddles.len());
        bindings.extend(buttons.into_iter().map(|button| {
            Binding::new(
                InputSource::Button(button),
                OutputAction::GamepadButton(button),
            )
        }));
        bindings
            .extend(axes.into_iter().map(|axis| {
                Binding::new(InputSource::Axis(axis), OutputAction::GamepadAxis(axis))
            }));
        if backend == VirtualGamepadBackend::DirectInput {
            bindings.extend(paddles.into_iter().map(|button| {
                Binding::new(
                    InputSource::Button(button),
                    OutputAction::GamepadButton(button),
                )
            }));
        }
        Self {
            name: String::new(),
            backend,
            bindings,
            ..Self::default()
        }
    }

    pub fn keyboard_keycodes(&self) -> Vec<u16> {
        self.all_activator_outputs()
            .filter_map(|output| match output {
                OutputAction::Keyboard { keycode } => Some(*keycode),
                _ => None,
            })
            .collect()
    }

    pub fn uses_mouse(&self) -> bool {
        (self.gyro.enabled && self.gyro.output == GyroOutput::Mouse)
            || self.all_activator_outputs().any(|output| {
                matches!(
                    output,
                    OutputAction::MouseAxis(_) | OutputAction::MouseButton(_)
                )
            })
    }
}

fn default_profile_version() -> u32 {
    PROFILE_VERSION
}

fn default_profile_name() -> String {
    String::new()
}

fn default_sensitivity() -> f32 {
    1.0
}

fn default_exponent() -> f32 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_profile_default_is_valid() {
        let profile = InputProfile::default();
        assert_eq!(profile.version, PROFILE_VERSION);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_input_profile_allows_empty_name() {
        let profile = InputProfile {
            name: String::new(),
            ..InputProfile::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_axis_transform_applies_dead_zone_and_inversion() {
        let transform = AxisTransform {
            dead_zone: 0.2,
            sensitivity: 1.0,
            exponent: 1.0,
            invert: true,
        };
        assert_eq!(transform.apply(0.1), 0.0);
        assert!((transform.apply(0.6) + 0.5).abs() < 0.001);
        assert!((transform.apply_unbounded(2.0) + 2.25).abs() < 0.001);
    }

    #[test]
    fn test_default_gamepad_contains_standard_gamepad_controls() {
        let profile = InputProfile::default_gamepad();
        assert_eq!(profile.backend, VirtualGamepadBackend::XInput);
        assert_eq!(profile.bindings.len(), 21);
        assert!(profile.bindings.iter().any(|binding| {
            binding.source == InputSource::Axis(GamepadAxis::LeftTrigger)
                && binding.output == OutputAction::GamepadAxis(GamepadAxis::LeftTrigger)
        }));
        assert!(profile.bindings.iter().any(|binding| {
            binding.source == InputSource::Axis(GamepadAxis::RightTrigger)
                && binding.output == OutputAction::GamepadAxis(GamepadAxis::RightTrigger)
        }));
        assert!(!profile.bindings.iter().any(|binding| {
            matches!(
                binding.source,
                InputSource::Button(GamepadButton::LeftTrigger | GamepadButton::RightTrigger)
            )
        }));
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_gamepad_button_identifies_paddles() {
        assert!(GamepadButton::Paddle1.is_paddle());
        assert!(!GamepadButton::Paddle1.is_xinput());
        assert!(!GamepadButton::A.is_paddle());
        assert!(GamepadButton::A.is_xinput());
    }

    #[test]
    fn test_default_gamepad_for_buttons_omits_paddle_outputs() {
        let profile = InputProfile::default_gamepad_for_buttons(&[
            GamepadButton::A,
            GamepadButton::Paddle1,
            GamepadButton::Paddle2,
        ]);
        assert!(!profile
            .bindings
            .iter()
            .any(|binding| { binding.source == InputSource::Button(GamepadButton::Paddle1) }));
        assert!(!profile
            .bindings
            .iter()
            .any(|binding| { binding.source == InputSource::Button(GamepadButton::Paddle3) }));
    }

    #[test]
    fn test_input_source_category_matches_controller_region() {
        assert_eq!(
            InputSource::Button(GamepadButton::DpadUp).category(),
            InputCategory::Dpad
        );
        assert_eq!(
            InputSource::Axis(GamepadAxis::RightTrigger).category(),
            InputCategory::Triggers
        );
        assert_eq!(
            InputSource::Button(GamepadButton::LeftTrigger).category(),
            InputCategory::Triggers
        );
        assert_eq!(
            InputSource::Button(GamepadButton::RightTrigger).category(),
            InputCategory::Triggers
        );
        assert_eq!(
            InputSource::AxisDirection {
                axis: GamepadAxis::LeftX,
                direction: AxisDirection::Negative,
            }
            .category(),
            InputCategory::Joysticks
        );
    }

    #[test]
    fn test_profile_rejects_invalid_transform() {
        let mut profile = InputProfile::default();
        profile.bindings.push(Binding {
            source: InputSource::Axis(GamepadAxis::LeftX),
            output: OutputAction::GamepadAxis(GamepadAxis::RightX),
            activation: Activation::Always,
            transform: AxisTransform {
                dead_zone: 1.0,
                ..AxisTransform::default()
            },
        });
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_profile_accepts_keyboard_and_mouse_outputs() {
        for output in [
            OutputAction::Keyboard { keycode: 30 },
            OutputAction::MouseButton(MouseButton::Left),
            OutputAction::MouseAxis(MouseAxis::X),
        ] {
            let profile = InputProfile {
                bindings: vec![Binding::new(InputSource::Button(GamepadButton::A), output)],
                ..InputProfile::default()
            };
            assert!(profile.validate().is_ok());
        }
    }

    #[test]
    fn test_profile_rejects_paddle_output() {
        let profile = InputProfile {
            bindings: vec![Binding::new(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(GamepadButton::Paddle1),
            )],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_profile_accepts_paddle_output_for_direct_input() {
        let profile = InputProfile {
            backend: VirtualGamepadBackend::DirectInput,
            bindings: vec![Binding::new(
                InputSource::Button(GamepadButton::A),
                OutputAction::GamepadButton(GamepadButton::Paddle1),
            )],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_profile_missing_backend_defaults_to_xinput() {
        let profile: InputProfile = serde_json::from_str("{}").unwrap();
        assert_eq!(profile.backend, VirtualGamepadBackend::XInput);
    }

    #[test]
    fn test_profile_serde_roundtrip_preserves_backend() {
        let profile = InputProfile {
            backend: VirtualGamepadBackend::DirectInput,
            ..InputProfile::default()
        };
        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: InputProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn test_direct_input_defaults_include_identity_paddle_bindings() {
        let profile = InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::DirectInput);
        assert_eq!(profile.bindings.len(), 29);
        for button in [
            GamepadButton::Paddle1,
            GamepadButton::Paddle2,
            GamepadButton::Paddle3,
            GamepadButton::Paddle4,
            GamepadButton::Paddle5,
            GamepadButton::Paddle6,
            GamepadButton::Paddle7,
            GamepadButton::Paddle8,
        ] {
            assert!(profile.bindings.iter().any(|binding| {
                binding.source == InputSource::Button(button)
                    && binding.output == OutputAction::GamepadButton(button)
            }));
        }
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_switch_pro_defaults_use_digital_triggers() {
        let profile = InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::SwitchPro);
        assert_eq!(profile.bindings.len(), 21);
        for button in [GamepadButton::LeftTrigger, GamepadButton::RightTrigger] {
            assert!(profile.bindings.iter().any(|binding| {
                binding.source == InputSource::Button(button)
                    && binding.output == OutputAction::GamepadButton(button)
            }));
        }
        assert!(!profile.bindings.iter().any(|binding| {
            matches!(
                binding.output,
                OutputAction::GamepadAxis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
            )
        }));
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_switch_pro_rejects_analog_trigger_outputs() {
        let profile = InputProfile {
            backend: VirtualGamepadBackend::SwitchPro,
            bindings: vec![Binding::new(
                InputSource::Axis(GamepadAxis::LeftX),
                OutputAction::GamepadAxis(GamepadAxis::LeftTrigger),
            )],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_xinput_defaults_omit_paddle_bindings() {
        let profile = InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::XInput);
        assert_eq!(profile.bindings.len(), 21);
        assert!(!profile
            .bindings
            .iter()
            .any(|binding| binding.output == OutputAction::GamepadButton(GamepadButton::Paddle1)));
    }

    #[test]
    fn test_profile_serde_roundtrip_preserves_custom_chord() {
        let profile = InputProfile {
            name: "My layout".to_string(),
            bindings: vec![Binding {
                source: InputSource::Axis(GamepadAxis::LeftX),
                output: OutputAction::GamepadAxis(GamepadAxis::RightX),
                activation: Activation::Chord {
                    sources: vec![
                        InputSource::Button(GamepadButton::Guide),
                        InputSource::Button(GamepadButton::Paddle1),
                    ],
                    mode: ChordMode::Toggle,
                },
                transform: AxisTransform {
                    dead_zone: 0.08,
                    sensitivity: 2.5,
                    exponent: 1.2,
                    invert: true,
                },
            }],
            ..InputProfile::default()
        };
        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: InputProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn test_profile_rejects_empty_chord() {
        let profile = InputProfile {
            bindings: vec![Binding {
                source: InputSource::Axis(GamepadAxis::LeftX),
                output: OutputAction::GamepadAxis(GamepadAxis::RightX),
                activation: Activation::Chord {
                    sources: Vec::new(),
                    mode: ChordMode::Hold,
                },
                transform: AxisTransform::default(),
            }],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_gyro_config_defaults_to_disabled_mouse() {
        let config = GyroConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.activation, GyroActivation::Always);
        assert_eq!(config.output, GyroOutput::Mouse);
        assert!((config.sensitivity - 1.0).abs() < f32::EPSILON);
        assert!(config.smoothing);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_profile_rejects_out_of_range_gyro_sensitivity() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                sensitivity: 50.0,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_profile_gyro_config_roundtrips_through_json() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                activation: GyroActivation::Hold(GamepadButton::LeftTrigger),
                output: GyroOutput::RightStick,
                sensitivity: 2.5,
                invert_x: true,
                invert_y: false,
                smoothing: false,
            },
            ..InputProfile::default()
        };
        let decoded = InputProfile::from_json(&serde_json::to_string(&profile).unwrap()).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn test_from_json_drops_legacy_gyro_and_recenter_bindings() {
        let json = r#"{
            "name": "legacy",
            "bindings": [
                {"source": {"button": "a"}, "output": {"gamepad_button": "b"}},
                {"source": {"gyro": "z"}, "output": {"mouse_axis": "x"},
                 "transform": {"sensitivity": 2.0, "invert": true}},
                {"source": {"button": "x"}, "output": "recenter_gyro"}
            ],
            "gyro_mode": "rate"
        }"#;
        let profile = InputProfile::from_json(json).unwrap();
        assert_eq!(profile.bindings.len(), 1);
        assert_eq!(
            profile.bindings[0].source,
            InputSource::Button(GamepadButton::A)
        );
        assert!(!profile.gyro.enabled, "gyro starts fresh, not synthesized");
    }

    #[test]
    fn test_from_json_keeps_gyro_config_over_legacy_bindings() {
        let json = r#"{
            "bindings": [{"source": {"gyro": "x"}, "output": {"mouse_axis": "y"}}],
            "gyro": {"enabled": true, "output": "right_stick"}
        }"#;
        let profile = InputProfile::from_json(json).unwrap();
        assert!(profile.bindings.is_empty());
        assert!(profile.gyro.enabled);
        assert_eq!(profile.gyro.output, GyroOutput::RightStick);
    }

    #[test]
    fn test_action_set_profile_roundtrips_through_json() {
        let profile = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![
                    InputMapping {
                        source: InputSource::Button(GamepadButton::A),
                        activators: vec![
                            Activator::full_press(vec![OutputAction::GamepadButton(
                                GamepadButton::A,
                            )]),
                            Activator {
                                kind: ActivatorKind::DoublePress {
                                    window_ms: 280,
                                },
                                outputs: vec![OutputAction::Keyboard { keycode: 32 }],
                                activation: Activation::Always,
                                settings: ActivatorSettings::default(),
                            },
                        ],
                        ..InputMapping::new(InputSource::Button(GamepadButton::A))
                    },
                    InputMapping {
                        source: InputSource::Axis(GamepadAxis::LeftX),
                        mode: Some(SourceMode::Joystick {
                            output: StickOutput::Left,
                            deadzone_inner: 0.12,
                            deadzone_outer: 0.94,
                            curve: 1.4,
                        }),
                        ..InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))
                    },
                ],
            }],
            action_layers: vec![ActionSetLayer {
                name: "Menus".to_string(),
                parent_set: "Default".to_string(),
                inputs: vec![InputMapping::simple(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::GamepadButton(GamepadButton::B),
                )],
            }],
            ..InputProfile::default()
        };
        let decoded = InputProfile::from_json(&serde_json::to_string(&profile).unwrap()).unwrap();
        assert_eq!(decoded, profile);
        assert!(decoded.validate().is_ok());
    }

    #[test]
    fn test_action_set_validation_rejects_bad_references() {
        let out_of_range = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::simple(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::SwitchActionSet(1),
                )],
            }],
            ..InputProfile::default()
        };
        assert!(out_of_range.validate().is_err());

        let unknown_parent = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: Vec::new(),
            }],
            action_layers: vec![ActionSetLayer {
                name: "Layer".to_string(),
                parent_set: "Missing".to_string(),
                inputs: Vec::new(),
            }],
            ..InputProfile::default()
        };
        assert!(unknown_parent.validate().is_err());

        let empty_activators = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::new(InputSource::Button(GamepadButton::A))],
            }],
            ..InputProfile::default()
        };
        assert!(empty_activators.validate().is_err());
    }

    #[test]
    fn test_keyboard_and_mouse_helpers_see_activator_outputs() {
        let profile = InputProfile {
            bindings: Vec::new(),
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::simple(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::Keyboard { keycode: 42 },
                )],
            }],
            ..InputProfile::default()
        };
        assert_eq!(profile.keyboard_keycodes(), vec![42]);
        assert!(!profile.uses_mouse());
    }

    #[test]
    fn test_mode_shift_trigger_must_be_a_button() {
        let analog_trigger = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping {
                    source: InputSource::Axis(GamepadAxis::LeftX),
                    mode_shifts: vec![ModeShift {
                        trigger: InputSource::Axis(GamepadAxis::RightX),
                        mode: None,
                        activators: Vec::new(),
                    }],
                    ..InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))
                }],
            }],
            ..InputProfile::default()
        };
        assert!(analog_trigger.validate().is_err());
    }
}
