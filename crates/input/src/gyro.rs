//! Player-space gyro processing.
//!
//! Raw IMU gyro reports angular velocity around the *sensor's* axes, so a
//! naive "local axis → mouse axis" mapping only works while the controller is
//! held in exactly one orientation. Rotating the pad mixes yaw into pitch and
//! back. The fix (used by JoyShockMapper and Steam Input) is to decompose the
//! rotation relative to gravity, which the accelerometer provides:
//!
//! * yaw is the component of ω around the gravity axis — turning left/right
//!   reads identically no matter how the pad is rolled or pitched;
//! * pitch is the remaining rotation projected onto the controller's "right"
//!   axis (local X with the gravity component removed).
//!
//! The processor also tracks gyro bias continuously: when the incoming samples
//! stay quiet long enough, their mean is folded into the bias estimate so a
//! resting controller stops drifting as its temperature changes.

use crate::profile::{GyroCalibration, GyroOrientation};

/// Samples kept for stillness detection. At a 250 Hz report rate this covers
/// ~1 s; faster controllers converge sooner.
const STILLNESS_WINDOW: usize = 250;
/// Per-axis sample variance below which the controller counts as at rest
/// (0.01 rad/s standard deviation ≈ 0.57 °/s, above typical sensor noise but
/// far below intentional motion).
const STILLNESS_VARIANCE: f32 = 0.0001;
/// Continuous stillness required before the bias estimate starts adapting.
const STILLNESS_SETTLE: f32 = 0.5;
/// A still window whose mean exceeds this (rad/s) is slow deliberate motion,
/// not bias, and must not be absorbed. Real biases sit far below this.
const BIAS_MEAN_LIMIT: f32 = 0.25;
/// How strongly each still sample nudges the bias toward the window mean.
const BIAS_ADAPT_RATE: f32 = 0.05;
/// Accelerometer magnitude (in g) below which the reading is shake/free-fall
/// garbage rather than a usable gravity direction.
const MIN_GRAVITY_G: f32 = 0.3;
/// Time constant of the gravity low-pass, in seconds. Fast enough to track
/// slow re-orientation, slow enough that aiming shakes don't tilt it.
const GRAVITY_LOWPASS_TC: f32 = 0.12;
/// Adaptive smoothing: angular speed (rad/s) below which smoothing is heaviest
/// and above which it disappears entirely.
const SMOOTH_LOW: f32 = 0.05;
const SMOOTH_HIGH: f32 = 0.5;
const SMOOTH_MAX_ALPHA: f32 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GyroRates {
    /// Rotation around the gravity axis in rad/s; positive turns right.
    pub yaw: f32,
    /// Rotation around the controller's right axis in rad/s; positive tilts
    /// the far edge of the pad upward (aim up).
    pub pitch: f32,
    /// False when no usable accelerometer data exists and the rates fall back
    /// to raw controller-space axes.
    pub gravity_locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GyroProcessingOptions {
    /// Blend neighbouring samples at low angular speed, pass flicks through
    /// untouched.
    pub smoothing: bool,
    /// Fold stillness-window means into the bias estimate over time.
    pub auto_calibrate: bool,
    /// Which rotation axes feed horizontal/vertical output.
    pub orientation: GyroOrientation,
}

impl Default for GyroProcessingOptions {
    fn default() -> Self {
        Self {
            smoothing: true,
            auto_calibrate: true,
            orientation: GyroOrientation::PlayerSpace,
        }
    }
}

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    const X: Self = Self {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    const Z: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    fn from_array(array: [f32; 3]) -> Self {
        Self {
            x: array[0],
            y: array[1],
            z: array[2],
        }
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn scale(self, factor: f32) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
            z: self.z * factor,
        }
    }

    fn magnitude(self) -> f32 {
        self.dot(self).sqrt()
    }

    fn normalized(self) -> Option<Self> {
        let magnitude = self.magnitude();
        if magnitude < 1.0e-6 || !magnitude.is_finite() {
            None
        } else {
            Some(self.scale(1.0 / magnitude))
        }
    }

    fn mean(samples: &[Self]) -> Self {
        let count = samples.len() as f32;
        samples
            .iter()
            .fold(Self::ZERO, |acc, sample| Self::sum(acc, *sample))
            .scale(1.0 / count)
    }

    fn sum(acc: Self, sample: Self) -> Self {
        Self {
            x: acc.x + sample.x,
            y: acc.y + sample.y,
            z: acc.z + sample.z,
        }
    }
}

struct StillnessTracker {
    samples: Vec<Vec3>,
    /// Seconds of continuous stillness accumulated so far.
    settled: f32,
}

impl StillnessTracker {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(STILLNESS_WINDOW),
            settled: 0.0,
        }
    }

    fn push(&mut self, sample: Vec3, dt: f32) {
        if self.samples.len() == STILLNESS_WINDOW {
            self.samples.remove(0);
        }
        self.samples.push(sample);
        if self.is_still() {
            self.settled += dt;
        } else {
            self.settled = 0.0;
        }
    }

    fn is_still(&self) -> bool {
        if self.samples.len() < 16 {
            return false;
        }
        let mean = Vec3::mean(&self.samples);
        self.samples.iter().all(|sample| {
            (sample.x - mean.x).powi(2) < STILLNESS_VARIANCE
                && (sample.y - mean.y).powi(2) < STILLNESS_VARIANCE
                && (sample.z - mean.z).powi(2) < STILLNESS_VARIANCE
        })
    }

    /// Window mean once the controller has been still long enough for it to
    /// represent bias rather than motion.
    fn bias_candidate(&self) -> Option<Vec3> {
        if self.settled < STILLNESS_SETTLE {
            return None;
        }
        let mean = Vec3::mean(&self.samples);
        (mean.magnitude() <= BIAS_MEAN_LIMIT).then_some(mean)
    }
}

pub struct GyroProcessor {
    bias: Vec3,
    gravity: Option<Vec3>,
    stillness: StillnessTracker,
    options: GyroProcessingOptions,
    smoothed_yaw: f32,
    smoothed_pitch: f32,
    /// World-anchored right axis for World Space: the previous right vector
    /// re-projected onto the current horizontal plane, so vertical output
    /// does not follow the controller's own roll.
    world_right: Option<Vec3>,
}

impl GyroProcessor {
    pub fn new(initial_bias: GyroCalibration, options: GyroProcessingOptions) -> Self {
        Self {
            bias: Vec3::from_array([initial_bias.x, initial_bias.y, initial_bias.z]),
            gravity: None,
            stillness: StillnessTracker::new(),
            options,
            smoothed_yaw: 0.0,
            smoothed_pitch: 0.0,
            world_right: None,
        }
    }

    /// Current bias estimate, including everything auto-calibration has
    /// learned; persisting this as the profile calibration shortens the next
    /// session's settling time.
    pub fn bias(&self) -> GyroCalibration {
        GyroCalibration {
            x: self.bias.x,
            y: self.bias.y,
            z: self.bias.z,
        }
    }

    pub fn process(&mut self, gyro: [f32; 3], accel: Option<[f32; 3]>, dt: f32) -> GyroRates {
        let gyro = Vec3::from_array(gyro);
        self.track_bias(gyro, dt);
        let rotation = Vec3 {
            x: gyro.x - self.bias.x,
            y: gyro.y - self.bias.y,
            z: gyro.z - self.bias.z,
        };
        let (mut yaw, mut pitch, gravity_locked) = match self.options.orientation {
            GyroOrientation::Yaw => (rotation.y, rotation.x, false),
            GyroOrientation::Roll => (rotation.z, rotation.x, false),
            GyroOrientation::YawPlusRoll => (rotation.y + rotation.z, rotation.x, false),
            GyroOrientation::PlayerSpace | GyroOrientation::WorldSpace => {
                match accel {
                    Some(accel) => {
                        self.update_gravity(Vec3::from_array(accel), dt);
                        match self.gravity.and_then(|gravity| gravity.normalized()) {
                            Some(gravity) => {
                                let (yaw, pitch, locked) = if self.options.orientation
                                    == GyroOrientation::WorldSpace
                                {
                                    self.world_space(rotation, gravity)
                                } else {
                                    Self::player_space(rotation, gravity)
                                };
                                (yaw, pitch, locked)
                            }
                            None => Self::controller_space(rotation),
                        }
                    }
                    None => Self::controller_space(rotation),
                }
            }
        };
        if self.options.smoothing {
            let speed = (yaw * yaw + pitch * pitch).sqrt();
            let blend = ((speed - SMOOTH_LOW) / (SMOOTH_HIGH - SMOOTH_LOW)).clamp(0.0, 1.0);
            let alpha = SMOOTH_MAX_ALPHA * (1.0 - blend);
            self.smoothed_yaw = yaw * (1.0 - alpha) + self.smoothed_yaw * alpha;
            self.smoothed_pitch = pitch * (1.0 - alpha) + self.smoothed_pitch * alpha;
            yaw = self.smoothed_yaw;
            pitch = self.smoothed_pitch;
        } else {
            self.smoothed_yaw = yaw;
            self.smoothed_pitch = pitch;
        }
        GyroRates {
            yaw,
            pitch,
            gravity_locked,
        }
    }

    fn track_bias(&mut self, sample: Vec3, dt: f32) {
        if !self.options.auto_calibrate {
            return;
        }
        self.stillness.push(sample, dt);
        if let Some(candidate) = self.stillness.bias_candidate() {
            self.bias = Vec3 {
                x: self.bias.x + (candidate.x - self.bias.x) * BIAS_ADAPT_RATE,
                y: self.bias.y + (candidate.y - self.bias.y) * BIAS_ADAPT_RATE,
                z: self.bias.z + (candidate.z - self.bias.z) * BIAS_ADAPT_RATE,
            };
        }
    }

    fn update_gravity(&mut self, accel: Vec3, dt: f32) {
        let magnitude = accel.magnitude();
        if !magnitude.is_finite() || !(MIN_GRAVITY_G..=2.5).contains(&magnitude) {
            // Free fall or violent shake: keep the last known direction.
            return;
        }
        let Some(direction) = accel.normalized() else {
            return;
        };
        self.gravity = Some(match self.gravity {
            // Exponential smoothing with a time constant independent of the
            // sample rate: alpha = dt / (tc + dt).
            Some(previous) => {
                let alpha = dt / (GRAVITY_LOWPASS_TC + dt);
                Vec3 {
                    x: previous.x + (direction.x - previous.x) * alpha,
                    y: previous.y + (direction.y - previous.y) * alpha,
                    z: previous.z + (direction.z - previous.z) * alpha,
                }
            }
            None => direction,
        });
    }

    /// World Space: like Player Space for yaw, but the pitch axis is a
    /// world-anchored right vector carried across updates, so rolling the
    /// controller on its side does not redirect vertical output.
    fn world_space(&mut self, rotation: Vec3, gravity: Vec3) -> (f32, f32, bool) {
        let yaw = rotation.dot(gravity);
        let right = match self.world_right {
            Some(previous) => Self::horizontal_axis(gravity, previous),
            None => Self::horizontal_axis(gravity, Vec3::X),
        }
        .or_else(|| Self::horizontal_axis(gravity, Vec3::Z));
        let perpendicular = Vec3 {
            x: rotation.x - gravity.x * yaw,
            y: rotation.y - gravity.y * yaw,
            z: rotation.z - gravity.z * yaw,
        };
        let pitch = right
            .map(|right| {
                self.world_right = Some(right);
                perpendicular.dot(right)
            })
            .unwrap_or_else(|| {
                self.world_right = None;
                perpendicular.x
            });
        (yaw, pitch, true)
    }

    /// Decomposition relative to gravity. Axis-free: works for any hold angle
    /// because both extracted rates derive from ĝ rather than raw local axes.
    fn player_space(rotation: Vec3, gravity: Vec3) -> (f32, f32, bool) {
        let yaw = rotation.dot(gravity);
        let perpendicular = Vec3 {
            x: rotation.x - gravity.x * yaw,
            y: rotation.y - gravity.y * yaw,
            z: rotation.z - gravity.z * yaw,
        };
        // The controller's right axis is local X; remove its gravity
        // component so it stays horizontal. When the pad is held with X
        // vertical (rare) that projection degenerates and local Z takes over.
        let right = Self::horizontal_axis(gravity, Vec3::X)
            .or_else(|| Self::horizontal_axis(gravity, Vec3::Z));
        let pitch = right
            .map(|right| perpendicular.dot(right))
            .unwrap_or(perpendicular.x);
        (yaw, pitch, true)
    }

    fn horizontal_axis(gravity: Vec3, axis: Vec3) -> Option<Vec3> {
        let projection = axis.dot(gravity);
        Vec3 {
            x: axis.x - gravity.x * projection,
            y: axis.y - gravity.y * projection,
            z: axis.z - gravity.z * projection,
        }
        .normalized()
    }

    /// Fallback for controllers whose accelerometer is unavailable: raw
    /// controller-space rates, correct only near the orientation in which
    /// they were tuned.
    fn controller_space(rotation: Vec3) -> (f32, f32, bool) {
        (rotation.z, rotation.x, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 250.0;

    fn processor() -> GyroProcessor {
        GyroProcessor::new(GyroCalibration::default(), GyroProcessingOptions::default())
    }

    /// Feed gravity long enough for the low-pass to converge, then the
    /// rotation long enough for the adaptive smoothing to reach steady state,
    /// and return the final rates.
    fn sample(processor: &mut GyroProcessor, rotation: [f32; 3], gravity: [f32; 3]) -> GyroRates {
        for _ in 0..200 {
            processor.process([0.0; 3], Some(gravity), DT);
        }
        let mut rates = GyroRates {
            yaw: 0.0,
            pitch: 0.0,
            gravity_locked: false,
        };
        for _ in 0..50 {
            rates = processor.process(rotation, Some(gravity), DT);
        }
        rates
    }

    #[test]
    fn test_player_space_yaw_around_gravity_flat() {
        let mut processor = processor();
        let rates = sample(&mut processor, [0.0, 0.0, 0.5], [0.0, 0.0, 1.0]);
        assert!((rates.yaw - 0.5).abs() < 0.001, "yaw: {}", rates.yaw);
        assert!(rates.pitch.abs() < 0.001, "pitch: {}", rates.pitch);
        assert!(rates.gravity_locked);
    }

    #[test]
    fn test_player_space_yaw_survives_45_degree_roll() {
        let mut processor = processor();
        // Controller rolled 45° around its long axis: gravity now has equal
        // parts Y and Z. Pure rotation around gravity must still read as pure
        // yaw with no pitch bleed.
        let inv_sqrt2 = 1.0 / std::f32::consts::SQRT_2;
        let gravity = [0.0, inv_sqrt2, inv_sqrt2];
        let rotation = [0.0, 0.5 * inv_sqrt2, 0.5 * inv_sqrt2];
        let rates = sample(&mut processor, rotation, gravity);
        assert!((rates.yaw - 0.5).abs() < 0.001, "yaw: {}", rates.yaw);
        assert!(rates.pitch.abs() < 0.001, "pitch: {}", rates.pitch);
    }

    #[test]
    fn test_player_space_pitch_while_flat() {
        let mut processor = processor();
        // Nodding the far edge of a flat controller up: rotation around local
        // X with gravity along Z.
        let rates = sample(&mut processor, [0.3, 0.0, 0.0], [0.0, 0.0, 1.0]);
        assert!((rates.pitch - 0.3).abs() < 0.001, "pitch: {}", rates.pitch);
        assert!(rates.yaw.abs() < 0.001, "yaw: {}", rates.yaw);
    }

    #[test]
    fn test_player_space_fallback_axis_when_controller_vertical() {
        let mut processor = processor();
        // Controller held vertically (local X points along gravity): the X
        // projection degenerates, so a rotation around local Z — now a
        // horizontal axis — must still register through the Z fallback.
        let rates = sample(&mut processor, [0.0, 0.0, 0.4], [1.0, 0.0, 0.0]);
        assert!(rates.yaw.abs() < 0.001, "yaw: {}", rates.yaw);
        assert!(
            (rates.pitch.abs() - 0.4).abs() < 0.001,
            "pitch: {}",
            rates.pitch
        );
        assert!(rates.gravity_locked);
    }

    #[test]
    fn test_seed_bias_is_subtracted() {
        let bias = GyroCalibration {
            x: 0.01,
            y: -0.02,
            z: 0.03,
        };
        let mut processor = GyroProcessor::new(
            bias,
            GyroProcessingOptions {
                auto_calibrate: false,
                ..GyroProcessingOptions::default()
            },
        );
        let rates = sample(&mut processor, [0.01, -0.02, 0.53], [0.0, 0.0, 1.0]);
        assert!((rates.yaw - 0.5).abs() < 0.001, "yaw: {}", rates.yaw);
    }

    #[test]
    fn test_auto_calibration_converges_to_constant_bias() {
        let mut processor = GyroProcessor::new(
            GyroCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        // Controller resting with a hidden Z bias of 0.2 rad/s.
        for _ in 0..(250 * 4) {
            processor.process([0.0, 0.0, 0.2], Some([0.0, 0.0, 1.0]), DT);
        }
        assert!(processor.bias().z > 0.15, "bias: {:?}", processor.bias());
        let rates = processor.process([0.0, 0.0, 0.2], Some([0.0, 0.0, 1.0]), DT);
        assert!(rates.yaw.abs() < 0.05, "yaw after cal: {}", rates.yaw);
    }

    #[test]
    fn test_auto_calibration_ignores_sustained_rotation() {
        let mut processor = GyroProcessor::new(
            GyroCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        // A slow constant turn has zero variance but is deliberate motion
        // (0.5 rad/s ≈ 29 °/s); it must never be folded into the bias.
        for _ in 0..(250 * 4) {
            processor.process([0.0, 0.0, 0.5], Some([0.0, 0.0, 1.0]), DT);
        }
        assert!(processor.bias().z.abs() < 0.01, "bias: {:?}", processor.bias());
    }

    #[test]
    fn test_auto_calibration_ignores_changing_motion() {
        let mut processor = GyroProcessor::new(
            GyroCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        for i in 0..(250 * 4) {
            let value = ((i % 250) as f32 / 250.0) * 1.5;
            processor.process([0.0, 0.0, value], Some([0.0, 0.0, 1.0]), DT);
        }
        assert!(
            processor.bias().z.abs() < 0.01,
            "bias: {:?}",
            processor.bias()
        );
    }

    #[test]
    fn test_missing_accelerometer_falls_back_to_controller_space() {
        let mut processor = processor();
        let rates = processor.process([0.2, 9.9, 0.7], None, DT);
        assert!((rates.yaw - 0.7).abs() < 0.001);
        assert!((rates.pitch - 0.2).abs() < 0.001);
        assert!(!rates.gravity_locked);
    }

    #[test]
    fn test_freefall_accelerometer_keeps_last_gravity() {
        let mut processor = processor();
        sample(&mut processor, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        let rates = processor.process([0.0, 0.0, 0.5], Some([0.0, 0.0, 0.0]), DT);
        assert!(rates.gravity_locked);
        assert!((rates.yaw - 0.5).abs() < 0.01, "yaw: {}", rates.yaw);
    }

    #[test]
    fn test_smoothing_damps_slow_noise_and_passes_flicks() {
        let options = GyroProcessingOptions {
            smoothing: true,
            auto_calibrate: false,
            ..GyroProcessingOptions::default()
        };
        let mut slow = GyroProcessor::new(GyroCalibration::default(), options);
        let mut fast = GyroProcessor::new(GyroCalibration::default(), options);
        let mut raw = GyroProcessor::new(
            GyroCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                auto_calibrate: false,
                orientation: GyroOrientation::PlayerSpace,
            },
        );
        for i in 0..200 {
            // Hand tremor at slow aiming speed: alternates ±0.02 rad/s.
            let noise = if i % 2 == 0 { 0.02 } else { -0.02 };
            slow.process([0.0, 0.0, noise], Some([0.0, 0.0, 1.0]), DT);
            fast.process([0.0, 0.0, 2.0], Some([0.0, 0.0, 1.0]), DT);
            raw.process([0.0, 0.0, 2.0], Some([0.0, 0.0, 1.0]), DT);
        }
        let damped = slow.process([0.0, 0.0, 0.02], Some([0.0, 0.0, 1.0]), DT).yaw;
        assert!(damped.abs() < 0.01, "damped yaw: {damped}");
        let flick = fast.process([0.0, 0.0, 2.0], Some([0.0, 0.0, 1.0]), DT).yaw;
        let raw_flick = raw.process([0.0, 0.0, 2.0], Some([0.0, 0.0, 1.0]), DT).yaw;
        assert!((flick - raw_flick).abs() < 0.01, "flick: {flick} vs {raw_flick}");
    }

    #[test]
    fn test_local_orientations_use_controller_axes() {
        // Yaw preset: rotation about the controller's own vertical axis.
        let mut processor = GyroProcessor::new(
            GyroCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                orientation: GyroOrientation::Yaw,
                ..GyroProcessingOptions::default()
            },
        );
        let rates = processor.process([0.0, 0.4, 0.0], Some([0.0, 1.0, 0.0]), DT);
        assert!((rates.yaw - 0.4).abs() < 0.001);
        assert!((rates.pitch - 0.0).abs() < 0.001);

        // Roll preset: lean around the forward axis drives horizontal.
        let mut processor = GyroProcessor::new(
            GyroCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                orientation: GyroOrientation::Roll,
                ..GyroProcessingOptions::default()
            },
        );
        let rates = processor.process([0.0, 0.0, 0.6], None, DT);
        assert!((rates.yaw - 0.6).abs() < 0.001);

        // Yaw + Roll sums both.
        let mut processor = GyroProcessor::new(
            GyroCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                orientation: GyroOrientation::YawPlusRoll,
                ..GyroProcessingOptions::default()
            },
        );
        let rates = processor.process([0.0, 0.2, 0.3], None, DT);
        assert!((rates.yaw - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_bias_accessor_round_trips_into_new_processor() {
        let bias = GyroCalibration {
            x: 0.05,
            y: 0.0,
            z: -0.05,
        };
        let mut processor = GyroProcessor::new(
            bias,
            GyroProcessingOptions {
                auto_calibrate: false,
                ..GyroProcessingOptions::default()
            },
        );
        for _ in 0..10 {
            processor.process([0.0, 0.0, 0.0], Some([0.0, 0.0, 1.0]), DT);
        }
        assert_eq!(processor.bias(), bias);
    }
}
