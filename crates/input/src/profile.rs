use serde::{Deserialize, Serialize};

pub const PROFILE_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroAxis {
    X,
    Y,
    Z,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroMode {
    #[default]
    Rate,
    HoldLast,
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

impl GyroCalibration {
    pub fn axis_value(self, axis: GyroAxis) -> f32 {
        match axis {
            GyroAxis::X => self.x,
            GyroAxis::Y => self.y,
            GyroAxis::Z => self.z,
        }
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
    Gyro(GyroAxis),
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
            InputSource::Gyro(_) => InputCategory::Gyro,
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
    RecenterGyro,
}

impl OutputAction {
    pub fn is_xinput_compatible(&self) -> bool {
        match self {
            Self::GamepadButton(button) => button.is_xinput(),
            Self::GamepadAxis(_) | Self::RecenterGyro => true,
            Self::Keyboard { .. } | Self::MouseButton(_) | Self::MouseAxis(_) => false,
        }
    }

    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::GamepadButton(button) if !button.is_xinput())
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecenterMode {
    #[default]
    Never,
    OnEnable,
    OnDisable,
    OnEnableOrDisable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub source: InputSource,
    pub output: OutputAction,
    #[serde(default)]
    pub gyro_mode: GyroMode,
    #[serde(default)]
    pub activation: Activation,
    #[serde(default)]
    pub transform: AxisTransform,
    #[serde(default)]
    pub recenter: RecenterMode,
}

impl Binding {
    pub fn new(source: InputSource, output: OutputAction) -> Self {
        Self {
            source,
            output,
            gyro_mode: GyroMode::default(),
            activation: Activation::Always,
            transform: AxisTransform::default(),
            recenter: RecenterMode::Never,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputProfile {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default = "default_profile_name")]
    pub name: String,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub gyro_calibration: GyroCalibration,
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
            bindings: Vec::new(),
            gyro_calibration: GyroCalibration::default(),
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
            if !binding.output.is_supported() {
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
            if binding.recenter != RecenterMode::Never
                && !matches!(binding.source, InputSource::Gyro(_))
            {
                return Err(format!(
                    "binding {index}: recentering requires a gyro source"
                ));
            }
        }
        Ok(())
    }

    pub fn default_gamepad() -> Self {
        Self::default_gamepad_controls()
    }

    pub fn default_gamepad_for_buttons(supported_buttons: &[GamepadButton]) -> Self {
        let mut profile = Self::default_gamepad_controls();
        profile.bindings.retain(|binding| {
            matches!(binding.source, InputSource::Axis(_) | InputSource::Gyro(_))
                || matches!(
                    binding.source,
                    InputSource::Button(button) if supported_buttons.contains(&button)
                )
        });
        profile
    }

    fn default_gamepad_controls() -> Self {
        let buttons = [
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
        let axes = [
            GamepadAxis::LeftX,
            GamepadAxis::LeftY,
            GamepadAxis::RightX,
            GamepadAxis::RightY,
            GamepadAxis::LeftTrigger,
            GamepadAxis::RightTrigger,
        ];
        let mut bindings = Vec::with_capacity(buttons.len() + axes.len());
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
        Self {
            name: String::new(),
            bindings,
            ..Self::default()
        }
    }

    pub fn keyboard_keycodes(&self) -> Vec<u16> {
        self.bindings
            .iter()
            .filter_map(|binding| match binding.output {
                OutputAction::Keyboard { keycode } => Some(keycode),
                _ => None,
            })
            .collect()
    }

    pub fn uses_mouse(&self) -> bool {
        self.bindings.iter().any(|binding| {
            matches!(
                binding.output,
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
        assert_eq!(
            InputSource::Gyro(GyroAxis::Z).category(),
            InputCategory::Gyro
        );
    }

    #[test]
    fn test_profile_rejects_invalid_transform() {
        let mut profile = InputProfile::default();
        profile.bindings.push(Binding {
            source: InputSource::Gyro(GyroAxis::X),
            output: OutputAction::GamepadAxis(GamepadAxis::RightX),
            activation: Activation::Always,
            gyro_mode: GyroMode::default(),
            transform: AxisTransform {
                dead_zone: 1.0,
                ..AxisTransform::default()
            },
            recenter: RecenterMode::Never,
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
    fn test_profile_serde_roundtrip_preserves_custom_gyro_chord() {
        let profile = InputProfile {
            name: "My gyro layout".to_string(),
            bindings: vec![Binding {
                source: InputSource::Gyro(GyroAxis::Z),
                output: OutputAction::GamepadAxis(GamepadAxis::RightX),
                activation: Activation::Chord {
                    sources: vec![
                        InputSource::Button(GamepadButton::Guide),
                        InputSource::Button(GamepadButton::Paddle1),
                    ],
                    mode: ChordMode::Toggle,
                },
                gyro_mode: GyroMode::default(),
                transform: AxisTransform {
                    dead_zone: 0.08,
                    sensitivity: 2.5,
                    exponent: 1.2,
                    invert: true,
                },
                recenter: RecenterMode::OnEnableOrDisable,
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
                source: InputSource::Gyro(GyroAxis::X),
                output: OutputAction::GamepadAxis(GamepadAxis::RightX),
                activation: Activation::Chord {
                    sources: Vec::new(),
                    mode: ChordMode::Hold,
                },
                gyro_mode: GyroMode::default(),
                transform: AxisTransform::default(),
                recenter: RecenterMode::Never,
            }],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn test_profile_rejects_recentering_non_gyro_binding() {
        let mut binding = Binding::new(
            InputSource::Button(GamepadButton::A),
            OutputAction::GamepadButton(GamepadButton::B),
        );
        binding.recenter = RecenterMode::OnEnable;
        let profile = InputProfile {
            bindings: vec![binding],
            ..InputProfile::default()
        };
        assert!(profile.validate().is_err());
    }
}
