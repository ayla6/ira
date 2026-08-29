use serde::{Deserialize, Serialize};

pub const PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualGamepadBackend {
    #[default]
    XInput,
    DirectInput,
    SwitchPro,
    DualShock4,
    DualSense,
    /// No kernel device at all: the whole controller is presented over the
    /// cemuhook DSU stream (the flatpak-friendly path Cemu binds as one
    /// provider for buttons and motion).
    Dsu,
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

    /// Human-readable name for messages that surface in the editor.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::A => "A (bottom face button)",
            Self::B => "B (right face button)",
            Self::X => "X (left face button)",
            Self::Y => "Y (top face button)",
            Self::LeftShoulder => "left shoulder button",
            Self::RightShoulder => "right shoulder button",
            Self::LeftTrigger => "left trigger",
            Self::RightTrigger => "right trigger",
            Self::Back => "select / back button",
            Self::Start => "start / options button",
            Self::Guide => "guide / home button",
            Self::LeftStick => "left stick click",
            Self::RightStick => "right stick click",
            Self::DpadUp => "d-pad up",
            Self::DpadDown => "d-pad down",
            Self::DpadLeft => "d-pad left",
            Self::DpadRight => "d-pad right",
            Self::Paddle1 => "back paddle 1",
            Self::Paddle2 => "back paddle 2",
            Self::Paddle3 => "back paddle 3",
            Self::Paddle4 => "back paddle 4",
            Self::Paddle5 => "back paddle 5",
            Self::Paddle6 => "back paddle 6",
            Self::Paddle7 => "back paddle 7",
            Self::Paddle8 => "back paddle 8",
        }
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

impl GamepadAxis {
    /// Human-readable name for messages that surface in the editor.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::LeftX => "left stick horizontal",
            Self::LeftY => "left stick vertical",
            Self::RightX => "right stick horizontal",
            Self::RightY => "right stick vertical",
            Self::LeftTrigger => "left trigger axis",
            Self::RightTrigger => "right trigger axis",
        }
    }
}

impl InputSource {
    /// Human-readable name for messages that surface in the editor.
    pub fn display_name(self) -> String {
        match self {
            Self::Button(button) => button.display_name().to_string(),
            Self::Axis(axis) => axis.display_name().to_string(),
            Self::AxisDirection { axis, direction } => match direction {
                AxisDirection::Negative => {
                    format!("{} (pushed left/up)", axis.display_name())
                }
                AxisDirection::Positive => {
                    format!("{} (pushed right/down)", axis.display_name())
                }
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Negative,
    Positive,
}

/// Per-controller calibration. Describes the *controller*, not the profile:
/// the gyro bias measured on one pad and its calibrated stick deadzone apply
/// to every profile played with that pad.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ControllerCalibration {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub z: f32,
    /// Stick movement ignored around each stick's center, as a fraction of
    /// full deflection. Joystick modes with [`StickDeadzone::Controller`]
    /// read the value for their stick.
    #[serde(default)]
    pub stick_deadzone_left: f32,
    #[serde(default)]
    pub stick_deadzone_right: f32,
    /// Controller-level Nintendo button layout, like Steam's per-controller
    /// toggle: face buttons swap so the physical A position reports as A and
    /// X as X (A↔B and X↔Y on the positional standard). Off by default —
    /// positional layout matches what every non-Nintendo game expects.
    #[serde(default)]
    pub nintendo_layout: bool,
}

/// Where a joystick's deadzone radii come from, mirroring Steam Input's
/// "Deadzone Source": raw input, the value calibrated for this controller,
/// or per-profile radii.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickDeadzone {
    /// No deadzone: the raw input of the joystick is sent.
    #[default]
    None,
    /// The deadzone value comes from this controller's calibration.
    Controller,
    /// Use the profile's own inner/outer radii.
    Custom,
}

/// Which components of a joystick's deflection reach the output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickOutputAxis {
    #[default]
    Both,
    Horizontal,
    Vertical,
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
    /// Which rotation axes feed horizontal/vertical output.
    pub orientation: GyroOrientation,
    /// Sensitivity multiplier around 1.0, applied to both axes.
    pub sensitivity: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    /// Adaptive smoothing: damps jitter during fine aim, never touches flicks.
    pub smoothing: bool,
    /// Glide after the gyro deactivates: its last motion keeps outputting
    /// while friction bleeds it off.
    pub momentum: GyroMomentum,
    /// Scales gyro mouse output down while the chosen trigger is held.
    pub trigger_dampening: TriggerDampening,
    /// Fraction of gyro mouse output removed while dampening is active:
    /// 0.0 leaves it untouched, 1.0 freezes it.
    pub dampening_amount: f32,
    /// Clockwise rotation applied to the gyro's 2D output, in degrees.
    /// Compensates for a favorite hold angle that leaves the camera
    /// diagonal (Steam's "Rotate Output").
    pub rotate_output: f32,
    /// Mouse pixels generated by one full 360° physical turn at 1x
    /// sensitivity — Steam's "Dots Per 360°". Shared with the flick stick
    /// so both calibrate against the same in-game angle.
    pub dots_per_360: f32,
    /// Steam's "Gyro To Joystick" shaping: how gyro rotation becomes
    /// stick deflection.
    pub stick: GyroStickSettings,
}

/// Steam's Gyro-To-Joystick camera model. The gyro names a desired camera
/// turn rate; that rate is deadzoned, normalized against the full-deflection
/// turn rate, shaped by a power curve, and clamped to a maximum output —
/// in that order, like Steam Input's own pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct GyroStickSettings {
    /// Final deflection ceiling as a fraction of full throw (0..=1).
    /// Lower it to keep games with "extra yaw" at extreme deflection calm.
    pub max_output: f32,
    /// Whether the power curve applies per axis or to the deflection
    /// magnitude (Steam's "Response Axis Style").
    pub response_style: GyroStickResponseStyle,
    /// Deflection exponent: 0.1 extremely aggressive (small motions deflect
    /// far), 1 linear, 4 extremely relaxed.
    pub power_curve: f32,
    /// Clamp the deflection vector to the maximum output; off, diagonals
    /// keep the full per-axis range.
    pub lock_at_edges: bool,
    /// Rotation speed below which nothing outputs, in degrees per second.
    pub deadzone_dps: f32,
}

impl Default for GyroStickSettings {
    fn default() -> Self {
        Self {
            max_output: 1.0,
            response_style: GyroStickResponseStyle::Circular,
            power_curve: 1.0,
            lock_at_edges: false,
            deadzone_dps: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroStickResponseStyle {
    PerAxis,
    #[default]
    Circular,
}

/// Gyro momentum, mirroring Steam Input: when the gyro is deactivated (by
/// its enable button), motion continues to output for a short time instead
/// of stopping dead.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct GyroMomentum {
    pub enabled: bool,
    /// Velocity decay per second; higher friction stops the glide sooner.
    pub friction: f32,
}

impl Default for GyroMomentum {
    fn default() -> Self {
        Self {
            enabled: false,
            friction: 2.0,
        }
    }
}

/// Which trigger state scales gyro mouse output down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerDampening {
    #[default]
    Off,
    RightTriggerSoftPull,
    RightTriggerFullPull,
    LeftTriggerSoftPull,
    LeftTriggerFullPull,
    BothTriggersFullPull,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroActivation {
    #[default]
    Always,
    /// Gyro is off unless the button is held (Steam's "Hold to Enable").
    Hold(GamepadButton),
    /// Gyro is on unless the button is held (Steam's "Hold to Suppress").
    Suppress(GamepadButton),
    Toggle(GamepadButton),
}

impl GyroActivation {
    /// Button that gates gyro, when activation is button-driven.
    pub fn button(self) -> Option<GamepadButton> {
        match self {
            Self::Always => None,
            Self::Hold(button) | Self::Suppress(button) | Self::Toggle(button) => Some(button),
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
    /// The mapping engine does not consume the gyro at all: the physical
    /// controller itself is exposed to the game over uhid (see
    /// [`InputProfile::wants_native_controller`]), so the sensors reach the
    /// game through its own native driver. Only meaningful with a backend
    /// that supports it (Switch Pro, DS4, DualSense) and a motion sensor.
    NativeMotion,
}

/// How gyro rotation maps to output axes, mirroring Steam Input's
/// orientation presets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroOrientation {
    /// Raw controller axes with no gravity math: reported yaw drives
    /// horizontal output and reported pitch drives vertical output exactly
    /// as the sensor delivers them. The basic pass-through preset.
    Local,
    /// Turn the controller around its own vertical axis for horizontal
    /// output; tilt around its lateral axis for vertical output.
    Yaw,
    /// Lean the controller around its forward axis for horizontal output;
    /// tilt for vertical.
    Roll,
    /// Lean and turn added together for horizontal output; tilt stays local.
    YawPlusRoll,
    /// Rotation around the gravity axis drives horizontal output and local
    /// pitch drives vertical output — consistent at any hold angle.
    #[default]
    PlayerSpace,
    /// Gravity-axis rotation for horizontal output like Player Space, but
    /// vertical output stays anchored to the world instead of following the
    /// controller's lateral axis when the pad is rolled on its side.
    WorldSpace,
    /// Aim like a laser pointer: vertical output follows the controller's
    /// long axis against gravity (absolute), horizontal output integrates
    /// the gyro yaw (relative).
    LaserPointer,
}

impl Default for GyroConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            activation: GyroActivation::Always,
            output: GyroOutput::Mouse,
            orientation: GyroOrientation::PlayerSpace,
            sensitivity: 1.0,
            invert_x: false,
            invert_y: false,
            smoothing: true,
            momentum: GyroMomentum::default(),
            trigger_dampening: TriggerDampening::Off,
            dampening_amount: 0.5,
            rotate_output: 0.0,
            dots_per_360: 6545.0,
            stick: GyroStickSettings::default(),
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
        if self.momentum.enabled
            && (!self.momentum.friction.is_finite()
                || !(0.5..=10.0).contains(&self.momentum.friction))
        {
            return Err("gyro momentum friction must be finite and within [0.5, 10]".to_string());
        }
        if self.trigger_dampening != TriggerDampening::Off
            && (!self.dampening_amount.is_finite() || !(0.0..=1.0).contains(&self.dampening_amount))
        {
            return Err("gyro dampening amount must be finite and within [0, 1]".to_string());
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
    Keyboard {
        keycode: u16,
    },
    MouseButton(MouseButton),
    MouseAxis(MouseAxis),
    /// One discrete scroll-wheel detent per activation (Steam's
    /// `mouse_wheel` command). `amount` is detents; negative scrolls down /
    /// left. Axes other than Wheel/WheelX are rejected by validation.
    WheelClick {
        axis: MouseAxis,
        amount: i32,
    },
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
    ModeShiftActivate {
        target: InputSource,
    },
}

impl OutputAction {
    pub fn is_xinput_compatible(&self) -> bool {
        match self {
            Self::GamepadButton(button) => button.is_xinput(),
            Self::GamepadAxis(_)
            | Self::WheelClick { .. }
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
            VirtualGamepadBackend::DirectInput | VirtualGamepadBackend::Dsu => true,
            // Sony pads expose all six axes (triggers on ABS_RX/ABS_RY);
            // only the paddles are missing.
            VirtualGamepadBackend::DualShock4 | VirtualGamepadBackend::DualSense => {
                !matches!(self, Self::GamepadButton(button) if button.is_paddle())
            }
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// Gate on an analog axis's state — Steam's "activate when the input is
    /// at rest / not at zero / maxed out" family.
    Analog {
        axis: GamepadAxis,
        condition: AnalogCondition,
        #[serde(default = "default_analog_threshold")]
        threshold: f32,
    },
}

impl Activation {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Analog { threshold, .. } => {
                if !threshold.is_finite() || !(0.0..0.9).contains(threshold) {
                    return Err("analog activation threshold must be in [0, 0.9)".to_string());
                }
            }
            Self::Chord { sources, .. } if sources.is_empty() => {
                return Err("chord cannot be empty".to_string());
            }
            _ => {}
        }
        Ok(())
    }
}

/// How [`Activation::Analog`] interprets `threshold` against the axis
/// magnitude `|v|`: at rest means `|v| <= threshold`, active means
/// `|v| > threshold`, maxed out means `|v| >= 1 - threshold`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalogCondition {
    AtRest,
    Active,
    MaxedOut,
}

pub(crate) fn default_analog_threshold() -> f32 {
    0.1
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordMode {
    #[default]
    Hold,
    Toggle,
}

/// A named group of input mappings. Profile action set `[0]` is the default;
/// higher sets are switched to by [`OutputAction::SwitchActionSet`].
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
    Joystick(JoystickSettings),
    Mouse {
        #[serde(default = "default_sensitivity")]
        sensitivity: f32,
        /// The deflection processing shared with the joystick behavior:
        /// deadzone, curve, scaling, rotation — applied before the pointer
        /// velocity is derived.
        #[serde(default)]
        stick: StickProcessing,
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
    /// Stick deflection as four digital directions (stick-as-dpad preset
    /// and Steam's dpad group).
    Dpad {
        #[serde(default = "default_dpad_threshold")]
        threshold: f32,
    },
}

/// Steam's joystick behavior: one stick mapped onto another, with deadzone,
/// response curve, per-axis sensitivity and invert, rotation, and axis
/// limiting. A newtype payload of [`SourceMode::Joystick`] so the JSON shape
/// matches the struct-variant form older profiles were written in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct JoystickSettings {
    pub output: StickOutput,
    #[serde(flatten)]
    pub processing: StickProcessing,
}

impl JoystickSettings {
    /// Neutral settings targeting `output`: raw passthrough with no
    /// deadzone and a linear curve.
    pub fn new(output: StickOutput) -> Self {
        Self {
            output,
            processing: StickProcessing::default(),
        }
    }
}

/// The analog-stick processing pipeline shared by the Joystick and Joystick
/// Mouse behaviors: rotation, deadzone, response curve, per-axis scale and
/// invert, axis limiting, and the outer ring command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct StickProcessing {
    /// Which components of the deflection reach the output.
    pub output_axis: StickOutputAxis,
    /// Rotates the input vector before deadzone and curve: at 90°, pushing
    /// the stick north reads as east.
    pub rotation: f32,
    pub sensitivity_x: f32,
    pub sensitivity_y: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    /// Where the deadzone radii come from.
    pub deadzone: StickDeadzone,
    pub deadzone_inner: f32,
    pub deadzone_outer: f32,
    pub curve: f32,
    /// Whether the response curve bends each axis on its own or the
    /// deflection's distance from the deadzone.
    pub response_axis_style: ResponseAxisStyle,
    /// Command held while the stick sits past the outer ring radius.
    pub outer_ring: Option<OuterRingCommand>,
}

impl Default for StickProcessing {
    fn default() -> Self {
        Self {
            output_axis: StickOutputAxis::default(),
            rotation: 0.0,
            sensitivity_x: 1.0,
            sensitivity_y: 1.0,
            invert_x: false,
            invert_y: false,
            deadzone: StickDeadzone::default(),
            deadzone_inner: 0.1,
            deadzone_outer: 0.95,
            curve: 1.0,
            response_axis_style: ResponseAxisStyle::default(),
            outer_ring: None,
        }
    }
}

impl StickProcessing {
    pub fn validate(&self) -> Result<(), String> {
        if !self.rotation.is_finite() || !(0.0..360.0).contains(&self.rotation) {
            return Err("rotation must be finite and in [0, 360)".to_string());
        }
        for (name, sensitivity) in [
            ("sensitivity_x", self.sensitivity_x),
            ("sensitivity_y", self.sensitivity_y),
        ] {
            if !sensitivity.is_finite() || !(0.0..=10.0).contains(&sensitivity) {
                return Err(format!("{name} must be finite and within [0, 10]"));
            }
        }
        if !(0.0..1.0).contains(&self.deadzone_inner) || !self.deadzone_inner.is_finite() {
            return Err("deadzone_inner must be finite and in [0, 1)".to_string());
        }
        if !(0.0..=1.0).contains(&self.deadzone_outer) || !self.deadzone_outer.is_finite() {
            return Err("deadzone_outer must be finite and in (0, 1]".to_string());
        }
        if self.deadzone_inner >= self.deadzone_outer {
            return Err("deadzone_inner must be below deadzone_outer".to_string());
        }
        if !self.curve.is_finite() || self.curve <= 0.0 {
            return Err("curve exponent must be finite and positive".to_string());
        }
        if let Some(ring) = &self.outer_ring {
            ring.validate()?;
        }
        Ok(())
    }
}

/// How the response curve is applied, mirroring Steam's "Response Axis
/// Style": per axis, or on the deflection's distance from the deadzone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseAxisStyle {
    #[default]
    Distance,
    PerAxis,
}

/// A command held while the stick is outside the outer ring radius (inside,
/// when inverted) — Steam's Outer Ring Command: hold the stick at the edge
/// to walk or sprint instead of the mapped walk speed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct OuterRingCommand {
    /// Fraction of full deflection where the ring begins.
    pub radius: f32,
    /// Fire inside the radius instead of outside.
    pub invert: bool,
    /// The discrete output (button, key, mouse button) held while active.
    pub output: OutputAction,
}

impl Default for OuterRingCommand {
    fn default() -> Self {
        Self {
            radius: 25000.0 / 32767.0,
            invert: false,
            output: OutputAction::GamepadButton(GamepadButton::A),
        }
    }
}

impl OuterRingCommand {
    pub fn validate(&self) -> Result<(), String> {
        if !self.radius.is_finite() || !(0.0 < self.radius && self.radius <= 1.0) {
            return Err("outer ring radius must be finite and in (0, 1]".to_string());
        }
        if !matches!(
            self.output,
            OutputAction::GamepadButton(_)
                | OutputAction::Keyboard { .. }
                | OutputAction::MouseButton(_)
        ) {
            return Err(
                "outer ring command must be a gamepad button, keyboard key, or mouse button"
                    .to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickOutput {
    #[default]
    Left,
    Right,
}

impl SourceMode {
    /// A neutral joystick mode: raw passthrough to the given stick with no
    /// deadzone and a linear curve — the starting point for every stick.
    pub fn joystick(output: StickOutput) -> Self {
        Self::Joystick(JoystickSettings::new(output))
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Joystick(settings) => settings.processing.validate(),
            Self::Mouse { stick, .. } => stick.validate(),
            _ => Ok(()),
        }
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

fn default_double_press_ms() -> u32 {
    320
}

fn default_long_press_ms() -> u32 {
    600
}

fn default_soft_threshold() -> f32 {
    0.5
}

fn default_dpad_threshold() -> f32 {
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
    /// Legacy fallback bias used when the controller has no stored
    /// calibration; the field name predates per-controller stick calibration.
    #[serde(default, alias = "gyro_calibration")]
    pub controller_calibration: ControllerCalibration,
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
    /// Platforms this profile was made for ("wii", "ps1", ...). Empty means
    /// the profile is offered on every platform; a profile created from a
    /// console game's controller page carries that platform so it never
    /// clutters unrelated games.
    #[serde(default)]
    pub compatible_platform_ids: Vec<String>,
    /// Whether the layout also exposes the physical motion sensors as
    /// standard evdev axes next to the virtual pad. Off by default: until
    /// the kernel grows UNIQ support for uinput and SDL falls back to its
    /// ioctl heuristics without udev tags, no consumer can pair with the
    /// node (flatpak sandboxes doubly so). Enable to experiment with
    /// future SDL versions or raw evdev readers.
    #[serde(default)]
    pub native_motion: bool,
    /// Also run the cemuhook (DSU) motion server when the backend is not
    /// Dsu. The stream then carries motion only — its controller state
    /// reads neutral — so emulators can bind it as a pure motion source
    /// while input keeps flowing through the virtual controller.
    #[serde(default)]
    pub dsu_motion: bool,
    /// Forward rumble the game plays on the virtual pad to the physical
    /// controller. On by default — a controller that never rumbles reads
    /// as broken.
    #[serde(default = "default_rumble_enabled")]
    pub rumble: bool,
    /// Action set to switch to when the game shows the mouse cursor
    /// (Steam Input's "action set when cursor shown"). `None` disables.
    #[serde(default)]
    pub action_set_when_cursor_shown: Option<usize>,
    /// Action set to switch back to when the game hides the cursor again.
    #[serde(default)]
    pub action_set_when_cursor_hidden: Option<usize>,
}

fn default_rumble_enabled() -> bool {
    true
}

impl Default for InputProfile {
    fn default() -> Self {
        Self {
            version: PROFILE_VERSION,
            name: default_profile_name(),
            backend: VirtualGamepadBackend::default(),
            controller_calibration: ControllerCalibration::default(),
            gyro: GyroConfig::default(),
            action_sets: Vec::new(),
            action_layers: Vec::new(),
            compatible_game_ids: Vec::new(),
            compatible_platform_ids: Vec::new(),
            native_motion: false,
            dsu_motion: false,
            rumble: true,
            action_set_when_cursor_shown: None,
            action_set_when_cursor_hidden: None,
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
            // Layers are parented by name, so duplicate names would make
            // "which set does this layer belong to" ambiguous.
            if self.action_sets.iter().any(|other| {
                other.name == set.name && !std::ptr::eq(other, set)
            }) {
                return Err(format!(
                    "action set {set_index}: the name '{}' is used twice",
                    set.name
                ));
            }
            for input in &set.inputs {
                self.validate_input_mapping(&set.name, input)?;
            }
        }
        for (label, target) in [
            ("cursor shown", self.action_set_when_cursor_shown),
            ("cursor hidden", self.action_set_when_cursor_hidden),
        ] {
            if let Some(target) = target {
                if target >= self.action_sets.len() {
                    return Err(format!(
                        "action set when {label} points at set {target}, but only {} sets exist",
                        self.action_sets.len()
                    ));
                }
            }
        }
        for (layer_index, layer) in self.action_layers.iter().enumerate() {
            if !self
                .action_sets
                .iter()
                .any(|set| set.name == layer.parent_set)
            {
                return Err(format!(
                    "action layer {layer_index} references unknown parent set '{}'",
                    layer.parent_set
                ));
            }
            for input in &layer.inputs {
                self.validate_input_mapping(&layer.name, input)?;
            }
        }
        Ok(())
    }

    fn validate_input_mapping(
        &self,
        context: &str,
        input: &InputMapping,
    ) -> Result<(), String> {
        let label = format!(
            "{context}: {source}",
            source = input.source.display_name()
        );
        if let Some(mode) = &input.mode {
            mode.validate()
                .map_err(|error| format!("{label}: {error}"))?;
        }
        // Button inputs express everything through activators; analog
        // inputs (sticks, triggers) may legitimately have no mode — that
        // is Steam's "None" behavior, an inert input the user chose to
        // leave unbound.
        if matches!(input.source, InputSource::Button(_))
            && input.mode.is_none()
            && input.activators.is_empty()
        {
            return Err(format!(
                "{context}: the {source} has nothing bound to it. Every button \
                 input needs at least one activator — a press or release that \
                 triggers it. Open this input in the controller editor and add \
                 an activator under it, or remove the input if it is unused.",
                context = context,
                source = input.source.display_name(),
            ));
        }
        for activator in &input.activators {
            if activator.outputs.is_empty() {
                return Err(format!("{label}: activator needs at least one output"));
            }
            activator
                .activation
                .validate()
                .map_err(|error| format!("{label}: {error}"))?;
            if let ActivatorKind::SoftPress { threshold } = activator.kind {
                if !(0.0..1.0).contains(&threshold) || !threshold.is_finite() {
                    return Err(format!("{label}: soft-press threshold must be in [0, 1)"));
                }
            }
            for output in &activator.outputs {
                match output {
                    OutputAction::SwitchActionSet(target) if *target >= self.action_sets.len() => {
                        return Err(format!(
                            "{label}: switch-action-set target {target} out of range"
                        ));
                    }
                    OutputAction::EnableLayer { layer, .. }
                        if *layer >= self.action_layers.len() =>
                    {
                        return Err(format!("{label}: enable-layer target {layer} out of range"));
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
        let mut outputs: Vec<&OutputAction> = Vec::new();
        for input in self
            .action_sets
            .iter()
            .flat_map(|set| set.inputs.iter())
            .chain(
                self.action_layers
                    .iter()
                    .flat_map(|layer| layer.inputs.iter()),
            )
        {
            for activator in input.activators.iter().chain(
                input
                    .mode_shifts
                    .iter()
                    .flat_map(|shift| shift.activators.iter()),
            ) {
                outputs.extend(activator.outputs.iter());
            }
        }
        outputs.into_iter()
    }

    pub fn default_gamepad() -> Self {
        Self::default_gamepad_for_backend(VirtualGamepadBackend::XInput)
    }

    /// Seed an identity default action set from the standard controls,
    /// replacing whatever sets the profile had. Used for fresh profiles and
    /// the reset-to-defaults flow.
    pub fn with_default_action_set(mut self) -> Self {
        self.action_sets = vec![ActionSet {
            name: "Default".to_string(),
            inputs: default_action_set_inputs(self.backend, &standard_buttons(self.backend)),
        }];
        self
    }

    pub fn default_gamepad_for_buttons(supported_buttons: &[GamepadButton]) -> Self {
        Self::default_gamepad_for_backend_and_buttons(
            VirtualGamepadBackend::XInput,
            supported_buttons,
        )
    }

    pub fn default_gamepad_for_backend(backend: VirtualGamepadBackend) -> Self {
        Self::default_gamepad_for_backend_and_buttons(backend, &standard_buttons(backend))
    }

    pub fn default_gamepad_for_backend_and_buttons(
        backend: VirtualGamepadBackend,
        supported_buttons: &[GamepadButton],
    ) -> Self {
        Self {
            backend,
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: default_action_set_inputs(backend, supported_buttons),
            }],
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
                    OutputAction::MouseAxis(_)
                        | OutputAction::MouseButton(_)
                        | OutputAction::WheelClick { .. }
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

/// Every button the backend's identity layout carries.
fn standard_buttons(backend: VirtualGamepadBackend) -> Vec<GamepadButton> {
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
    // Switch Pro presents its digital trigger clicks as buttons.
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
    buttons
}

/// Identity mappings for the standard controls: buttons passthrough, one
/// Joystick-mode mapping per stick (on its X axis), triggers thresholded
/// passthrough. Buttons the device does not report are left out.
fn default_action_set_inputs(
    backend: VirtualGamepadBackend,
    supported_buttons: &[GamepadButton],
) -> Vec<InputMapping> {
    let mut inputs: Vec<InputMapping> = standard_buttons(backend)
        .into_iter()
        .filter(|button| supported_buttons.contains(button))
        .map(|button| {
            InputMapping::simple(
                InputSource::Button(button),
                OutputAction::GamepadButton(button),
            )
        })
        .collect();
    for (x_axis, output) in [
        (GamepadAxis::LeftX, StickOutput::Left),
        (GamepadAxis::RightX, StickOutput::Right),
    ] {
        inputs.push(InputMapping {
            mode: Some(SourceMode::joystick(output)),
            ..InputMapping::new(InputSource::Axis(x_axis))
        });
    }
    if backend != VirtualGamepadBackend::SwitchPro {
        for axis in [GamepadAxis::LeftTrigger, GamepadAxis::RightTrigger] {
            inputs.push(InputMapping {
                mode: Some(SourceMode::Trigger { threshold: 0.5 }),
                ..InputMapping::new(InputSource::Axis(axis))
            });
        }
    }
    inputs
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
    fn test_default_gamepad_contains_standard_gamepad_controls() {
        let profile = InputProfile::default_gamepad();
        assert_eq!(profile.backend, VirtualGamepadBackend::XInput);
        let inputs = &profile.action_sets[0].inputs;
        // 15 buttons + 2 sticks + 2 triggers.
        assert_eq!(inputs.len(), 19);
        let trigger = inputs
            .iter()
            .find(|input| input.source == InputSource::Axis(GamepadAxis::LeftTrigger))
            .unwrap();
        assert!(matches!(trigger.mode, Some(SourceMode::Trigger { .. })));
        assert!(!inputs.iter().any(|input| matches!(
            input.source,
            InputSource::Button(GamepadButton::LeftTrigger | GamepadButton::RightTrigger)
        )));
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
        let inputs = &profile.action_sets[0].inputs;
        assert!(inputs
            .iter()
            .any(|input| input.source == InputSource::Button(GamepadButton::A)));
        assert!(!inputs
            .iter()
            .any(|input| input.source == InputSource::Button(GamepadButton::Paddle1)));
        assert!(!inputs
            .iter()
            .any(|input| input.source == InputSource::Button(GamepadButton::Paddle3)));
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
    fn test_profile_accepts_keyboard_and_mouse_outputs() {
        for output in [
            OutputAction::Keyboard { keycode: 30 },
            OutputAction::MouseButton(MouseButton::Left),
        ] {
            let profile = InputProfile {
                action_sets: vec![ActionSet {
                    name: "Default".to_string(),
                    inputs: vec![InputMapping::simple(
                        InputSource::Button(GamepadButton::A),
                        output,
                    )],
                }],
                ..InputProfile::default()
            };
            assert!(profile.validate().is_ok());
        }
    }

    #[test]
    fn test_profile_rejects_paddle_output() {
        let profile = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::simple(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::GamepadButton(GamepadButton::Paddle1),
                )],
            }],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_profile_accepts_paddle_output_for_direct_input() {
        let profile = InputProfile {
            backend: VirtualGamepadBackend::DirectInput,
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::simple(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::GamepadButton(GamepadButton::Paddle1),
                )],
            }],
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
    fn test_profile_missing_dsu_motion_defaults_to_off() {
        let profile: InputProfile = serde_json::from_str("{}").unwrap();
        assert!(!profile.dsu_motion);
    }

    #[test]
    fn test_profile_serde_roundtrip_preserves_dsu_motion() {
        let profile = InputProfile {
            dsu_motion: true,
            ..InputProfile::default()
        };
        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: InputProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);
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
        let inputs = &profile.action_sets[0].inputs;
        // 23 buttons (paddles included) + 2 sticks + 2 triggers.
        assert_eq!(inputs.len(), 27);
        for button in [
            GamepadButton::Paddle1,
            GamepadButton::Paddle4,
            GamepadButton::Paddle8,
        ] {
            let mapping = inputs
                .iter()
                .find(|input| input.source == InputSource::Button(button))
                .unwrap();
            assert_eq!(
                mapping.activators[0].outputs,
                vec![OutputAction::GamepadButton(button)]
            );
        }
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_switch_pro_defaults_use_digital_triggers() {
        let profile = InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::SwitchPro);
        let inputs = &profile.action_sets[0].inputs;
        // 17 buttons (digital trigger clicks) + 2 sticks, no analog triggers.
        assert_eq!(inputs.len(), 19);
        for button in [GamepadButton::LeftTrigger, GamepadButton::RightTrigger] {
            assert!(inputs
                .iter()
                .any(|input| input.source == InputSource::Button(button)));
        }
        assert!(!inputs.iter().any(|input| {
            matches!(
                input.source,
                InputSource::Axis(GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
            )
        }));
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_switch_pro_rejects_analog_trigger_outputs() {
        let profile = InputProfile {
            backend: VirtualGamepadBackend::SwitchPro,
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::simple(
                    InputSource::Button(GamepadButton::A),
                    OutputAction::GamepadAxis(GamepadAxis::LeftTrigger),
                )],
            }],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_xinput_defaults_omit_paddle_bindings() {
        let profile = InputProfile::default_gamepad_for_backend(VirtualGamepadBackend::XInput);
        let inputs = &profile.action_sets[0].inputs;
        assert!(!inputs.iter().any(|input| {
            input.activators.iter().any(|activator| {
                activator
                    .outputs
                    .contains(&OutputAction::GamepadButton(GamepadButton::Paddle1))
            })
        }));
    }

    #[test]
    fn test_profile_serde_roundtrip_preserves_custom_chord() {
        let profile = InputProfile {
            name: "My layout".to_string(),
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping {
                    activators: vec![Activator {
                        kind: ActivatorKind::FullPress,
                        outputs: vec![OutputAction::GamepadButton(GamepadButton::B)],
                        activation: Activation::Chord {
                            sources: vec![
                                InputSource::Button(GamepadButton::Guide),
                                InputSource::Button(GamepadButton::Paddle1),
                            ],
                            mode: ChordMode::Toggle,
                        },
                        settings: ActivatorSettings::default(),
                    }],
                    ..InputMapping::new(InputSource::Button(GamepadButton::A))
                }],
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
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping {
                    activators: vec![Activator {
                        kind: ActivatorKind::FullPress,
                        outputs: vec![OutputAction::GamepadButton(GamepadButton::B)],
                        activation: Activation::Chord {
                            sources: Vec::new(),
                            mode: ChordMode::Hold,
                        },
                        settings: ActivatorSettings::default(),
                    }],
                    ..InputMapping::new(InputSource::Button(GamepadButton::A))
                }],
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
        assert!(!config.momentum.enabled);
        assert_eq!(config.trigger_dampening, TriggerDampening::Off);
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
    fn test_gyro_config_deserializes_without_momentum_or_dampening_fields() {
        // Profiles saved before those settings existed must load with the
        // defaults rather than failing.
        let config: GyroConfig =
            serde_json::from_str(r#"{"enabled": true, "sensitivity": 2.0}"#).unwrap();
        assert_eq!(config.momentum, GyroMomentum::default());
        assert_eq!(config.trigger_dampening, TriggerDampening::Off);
        assert!((config.dampening_amount - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_profile_rejects_out_of_range_momentum_and_dampening() {
        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                momentum: GyroMomentum {
                    enabled: true,
                    friction: 99.0,
                },
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());

        let profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                trigger_dampening: TriggerDampening::BothTriggersFullPull,
                dampening_amount: 2.0,
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
                orientation: GyroOrientation::WorldSpace,
                sensitivity: 2.5,
                invert_x: true,
                invert_y: false,
                smoothing: false,
                momentum: GyroMomentum {
                    enabled: true,
                    friction: 4.0,
                },
                trigger_dampening: TriggerDampening::RightTriggerSoftPull,
                rotate_output: 90.0,
                dots_per_360: 8000.0,
                stick: GyroStickSettings {
                    power_curve: 2.0,
                    deadzone_dps: 1.5,
                    ..GyroStickSettings::default()
                },
                dampening_amount: 0.75,
            },
            ..InputProfile::default()
        };
        let decoded = InputProfile::from_json(&serde_json::to_string(&profile).unwrap()).unwrap();
        assert_eq!(decoded, profile);
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
                                kind: ActivatorKind::DoublePress { window_ms: 280 },
                                outputs: vec![OutputAction::Keyboard { keycode: 32 }],
                                activation: Activation::Always,
                                settings: ActivatorSettings::default(),
                            },
                        ],
                        ..InputMapping::new(InputSource::Button(GamepadButton::A))
                    },
                    InputMapping {
                        source: InputSource::Axis(GamepadAxis::LeftX),
                        mode: Some(SourceMode::Joystick(JoystickSettings {
                            output: StickOutput::Left,
                            processing: StickProcessing {
                                deadzone: StickDeadzone::Custom,
                                deadzone_inner: 0.12,
                                deadzone_outer: 0.94,
                                curve: 1.4,
                                ..StickProcessing::default()
                            },
                        })),
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
    fn test_old_joystick_mode_json_gets_steam_defaults() {
        // Profiles written before the stick rework carry no deadzone source,
        // per-axis sensitivity, or rotation; they must load as raw
        // passthrough instead of silently keeping a hidden deadzone.
        let profile = InputProfile::from_json(
            r#"{"name":"old","action_sets":[{"name":"Default","inputs":[
                {"source":{"axis":"left_x"},
                 "mode":{"joystick":{"output":"left","deadzone_inner":0.1,"deadzone_outer":0.95,"curve":1.0}}}
            ]}]}"#,
        )
        .unwrap();
        let Some(SourceMode::Joystick(settings)) = profile.action_sets[0].inputs[0].mode.as_ref()
        else {
            panic!("expected a joystick mode");
        };
        assert_eq!(settings.processing.deadzone, StickDeadzone::None);
        assert_eq!(settings.processing.output_axis, StickOutputAxis::Both);
        assert_eq!(settings.processing.rotation, 0.0);
        assert!((settings.processing.sensitivity_x - 1.0).abs() < f32::EPSILON);
        assert!((settings.processing.sensitivity_y - 1.0).abs() < f32::EPSILON);
        assert!(!settings.processing.invert_x && !settings.processing.invert_y);
    }

    #[test]
    fn test_joystick_settings_reject_out_of_range_rotation_and_sensitivity() {
        let build = |settings: JoystickSettings| InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping {
                    mode: Some(SourceMode::Joystick(settings)),
                    ..InputMapping::new(InputSource::Axis(GamepadAxis::LeftX))
                }],
            }],
            ..InputProfile::default()
        };
        assert!(build(JoystickSettings {
            processing: StickProcessing {
                rotation: 400.0,
                ..StickProcessing::default()
            },
            ..JoystickSettings::default()
        })
        .validate()
        .is_err());
        assert!(build(JoystickSettings {
            processing: StickProcessing {
                sensitivity_x: 20.0,
                ..StickProcessing::default()
            },
            ..JoystickSettings::default()
        })
        .validate()
        .is_err());
        assert!(build(JoystickSettings::default()).validate().is_ok());
    }

    #[test]
    fn test_from_json_collapses_legacy_per_axis_stick_mappings() {
        // The per-axis editor wrote one mapping per stick axis; loading
        // merges each Y half into its X counterpart.
        let profile = InputProfile::from_json(
            r#"{"name":"old","action_sets":[{"name":"Default","inputs":[
                {"source":{"axis":"left_x"},
                 "mode":{"joystick":{"output":"left","curve":1.0}}},
                {"source":{"axis":"left_y"},
                 "mode":{"joystick":{"output":"left","curve":2.0}}},
                {"source":{"axis":"right_y"},
                 "mode":{"joystick":{"output":"right","curve":3.0}}}
            ]}]}"#,
        )
        .unwrap();
        let inputs = &profile.action_sets[0].inputs;
        assert_eq!(inputs.len(), 2);
        assert!(inputs.iter().all(|input| matches!(
            input.source,
            InputSource::Axis(GamepadAxis::LeftX) | InputSource::Axis(GamepadAxis::RightX)
        )));
        // The X half's own mode wins over the Y half's.
        let Some(SourceMode::Joystick(settings)) = inputs
            .iter()
            .find(|input| input.source == InputSource::Axis(GamepadAxis::LeftX))
            .and_then(|input| input.mode.as_ref())
        else {
            panic!("expected a joystick mode on the left stick");
        };
        assert!((settings.processing.curve - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_from_json_keeps_stick_y_half_carrying_activators() {
        let profile = InputProfile::from_json(
            r#"{"name":"old","action_sets":[{"name":"Default","inputs":[
                {"source":{"axis":"right_x"},
                 "mode":{"joystick":{"output":"right","curve":1.0}}},
                {"source":{"axis":"right_y"},
                 "mode":{"joystick":{"output":"right","curve":2.0}},
                 "activators":[{"kind":"full_press","outputs":[{"gamepad_button":"a"}]}]}
            ]}]}"#,
        )
        .unwrap();
        let inputs = &profile.action_sets[0].inputs;
        assert_eq!(inputs.len(), 2);
        let y_half = inputs
            .iter()
            .find(|input| input.source == InputSource::Axis(GamepadAxis::RightY))
            .unwrap();
        assert!(!y_half.activators.is_empty());
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
        let error = empty_activators.validate().unwrap_err();
        // The message must name the input in plain language and say what to
        // do about it — "Default input 21: needs at least one activator"
        // was unreadable.
        assert!(error.contains("Default"), "{error}");
        assert!(error.contains("A (bottom face button)"), "{error}");
        assert!(error.contains("activator"), "{error}");
        assert!(error.contains("controller editor"), "{error}");
    }

    #[test]
    fn test_analog_input_without_mode_is_a_valid_none_behavior() {
        // Steam's "None": an unbound axis is inert by choice, not an error.
        // This is what blocked setting trigger behavior to None before.
        let unbound_trigger = InputProfile {
            action_sets: vec![ActionSet {
                name: "Default".to_string(),
                inputs: vec![InputMapping::new(InputSource::Axis(
                    GamepadAxis::LeftTrigger,
                ))],
            }],
            ..InputProfile::default()
        };
        assert!(unbound_trigger.validate().is_ok());
    }

    #[test]
    fn test_source_display_names_are_plain_language() {
        assert_eq!(
            GamepadButton::LeftTrigger.display_name(),
            "left trigger"
        );
        assert_eq!(
            GamepadAxis::RightY.display_name(),
            "right stick vertical"
        );
        assert_eq!(
            InputSource::AxisDirection {
                axis: GamepadAxis::LeftX,
                direction: AxisDirection::Negative,
            }
            .display_name(),
            "left stick horizontal (pushed left/up)"
        );
    }

    #[test]
    fn test_keyboard_and_mouse_helpers_see_activator_outputs() {
        let profile = InputProfile {
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
