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

use crate::profile::{ControllerCalibration, GyroOrientation};

/// Wall-clock span of the stillness window, in seconds. Time-based, not
/// sample-count-based: several pads report motion at only 20-40 Hz, and a
/// 250-deep FIFO would hold 6-12 s of history there, pushing old motion
/// samples into the window for so long that stillness is never detected
/// and the bias never converges.
const STILLNESS_WINDOW_SECONDS: f32 = 1.2;
/// Minimum samples before stillness is even considered, so a handful of
/// quiet readings right after motion cannot pose as rest.
const STILLNESS_MIN_SAMPLES: usize = 16;
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
// Fusion and conversion tuning follows JibbSmart's GamepadMotionHelpers
// (the library behind JoyShockMapper and Steam's 2023 gyro overhaul),
// pinned at 39b578a in references/JoyShockMapper.
/// Half-life of the smoothed accelerometer used to detect shakiness
/// (linear acceleration), in seconds.
const SHAKE_HALF_TIME: f32 = 0.25;
/// Shakiness (in g) below which the gravity estimate is corrected at the
/// full still speed, and above which it eases toward the shaky speed.
const SHAKE_MIN: f32 = 0.01;
const SHAKE_MAX: f32 = 0.4;
/// Gravity correction speeds in g/s: fast while steady, barely crawling
/// while shaky.
const CORRECTION_STILL_SPEED: f32 = 1.0;
const CORRECTION_SHAKY_SPEED: f32 = 0.1;
/// The correction is additionally capped relative to the gyro rate so it
/// can never fight real motion, with a small floor.
const CORRECTION_GYRO_FACTOR: f32 = 0.1;
const CORRECTION_MIN_SPEED: f32 = 0.01;
/// The correction eases into the cap as the estimate closes on the
/// accelerometer direction (difference in g).
const CLOSE_ENOUGH_MIN: f32 = 0.05;
const CLOSE_ENOUGH_MAX: f32 = 0.25;
/// Player space's yaw relax factor: when the pad rolls steeply the
/// gravity-projected yaw shrinks; the relax factor restores up to 41% of
/// it, capped at the full horizontal gyro magnitude.
const YAW_RELAX_FACTOR: f32 = 1.41;
/// World space's side-reduction threshold: past this flatness/upness the
/// gravity-projected pitch fades out instead of flipping.
const SIDE_REDUCTION_THRESHOLD: f32 = 0.125;
/// Adaptive smoothing: angular speed (rad/s) below which smoothing is heaviest
/// and above which it disappears entirely.
const SMOOTH_LOW: f32 = 0.05;
const SMOOTH_HIGH: f32 = 0.5;
const SMOOTH_MAX_ALPHA: f32 = 0.8;
/// Standard gravity: sensor producers report the accelerometer in m/s²
/// while the gravity filters work in g.
const GRAVITY_MS2: f32 = 9.80665;

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

    fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    fn plus(self, other: Self) -> Self {
        Self::sum(self, other)
    }

    fn minus(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }

    fn lerp(self, other: Self, factor: f32) -> Self {
        self.plus(other.minus(self).scale(factor))
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
    /// (reading, age in seconds); entries older than the window are evicted
    /// so the detector spans fixed wall-clock time at any sample rate.
    samples: Vec<(Vec3, f32)>,
    /// Seconds of continuous stillness accumulated so far.
    settled: f32,
}

impl StillnessTracker {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
            settled: 0.0,
        }
    }

    fn push(&mut self, sample: Vec3, dt: f32) {
        for entry in &mut self.samples {
            entry.1 += dt;
        }
        while self
            .samples
            .first()
            .is_some_and(|first| first.1 > STILLNESS_WINDOW_SECONDS)
        {
            self.samples.remove(0);
        }
        self.samples.push((sample, 0.0));
        if self.is_still() {
            self.settled += dt;
        } else {
            self.settled = 0.0;
        }
    }

    fn is_still(&self) -> bool {
        if self.samples.len() < STILLNESS_MIN_SAMPLES {
            return false;
        }
        let readings = self.samples.iter().map(|(sample, _)| *sample).collect::<Vec<_>>();
        let mean = Vec3::mean(&readings);
        readings.iter().all(|sample| {
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
        let readings = self.samples.iter().map(|(sample, _)| *sample).collect::<Vec<_>>();
        let mean = Vec3::mean(&readings);
        (mean.magnitude() <= BIAS_MEAN_LIMIT).then_some(mean)
    }
}

pub struct GyroProcessor {
    bias: Vec3,
    /// Gravity estimate: the up direction in the controller frame, carried
    /// by the gyro rotation and corrected toward the accelerometer.
    gravity: Option<Vec3>,
    /// Shakiness detection: smoothed accelerometer and its worst recent
    /// deviation (both in g). Shakiness means linear acceleration, and the
    /// fusion slows its correction to a crawl so tremors never tilt it.
    smooth_accel: Vec3,
    shakiness: f32,
    stillness: StillnessTracker,
    options: GyroProcessingOptions,
    smoothed_yaw: f32,
    smoothed_pitch: f32,
    /// Laser Pointer position tracking: yaw integrated around the vertical
    /// and pitch measured from gravity, accumulated as angle deltas since
    /// the last drain. Position-based: holding still holds the cursor
    /// still — nothing here can slide.
    laser_yaw_pending: f32,
    laser_pitch_pending: f32,
    laser_pitch_prev: Option<f32>,
}

impl GyroProcessor {
    pub fn new(initial_bias: ControllerCalibration, options: GyroProcessingOptions) -> Self {
        Self {
            bias: Vec3::from_array([initial_bias.x, initial_bias.y, initial_bias.z]),
            gravity: None,
            smooth_accel: Vec3::ZERO,
            shakiness: 0.0,
            stillness: StillnessTracker::new(),
            options,
            smoothed_yaw: 0.0,
            smoothed_pitch: 0.0,
            laser_yaw_pending: 0.0,
            laser_pitch_pending: 0.0,
            laser_pitch_prev: None,
        }
    }

    /// Current bias estimate, including everything auto-calibration has
    /// learned; persisting this as the profile calibration shortens the next
    /// session's settling time.
    /// Accumulated Laser Pointer angle deltas (yaw, pitch in radians)
    /// since the last drain. The mapper emits these directly as cursor
    /// position deltas, bypassing the rate pipeline entirely.
    pub fn take_laser_deltas(&mut self) -> (f32, f32) {
        (
            std::mem::take(&mut self.laser_yaw_pending),
            std::mem::take(&mut self.laser_pitch_pending),
        )
    }

    pub fn bias(&self) -> ControllerCalibration {
        ControllerCalibration {
            x: self.bias.x,
            y: self.bias.y,
            z: self.bias.z,
            stick_deadzone_left: 0.0,
            stick_deadzone_right: 0.0,
            nintendo_layout: false,
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
        self.update_gravity(rotation, accel, dt);
        let (mut yaw, mut pitch, gravity_locked) = match self.options.orientation {
            GyroOrientation::Local => (rotation.y, rotation.x, false),
            GyroOrientation::Yaw => (rotation.y, rotation.x, false),
            GyroOrientation::Roll => (rotation.z, rotation.x, false),
            GyroOrientation::YawPlusRoll => (rotation.y + rotation.z, rotation.x, false),
            GyroOrientation::LaserPointer => match self.gravity.and_then(|up| up.normalized()) {
                Some(up) => {
                            // Laser Pointer, in Steam Input's sense: an
                            // imaginary ray out of the controller drives
                            // the cursor by POSITION. Yaw is unobservable
                            // from an IMU alone, so the ray's horizontal
                            // angle integrates the gravity-projected gyro
                            // rate; the vertical angle is absolute from
                            // gravity. Deltas accumulate here and are
                            // drained straight into cursor movement —
                            // never through the rate pipeline, whose
                            // smoothing turned every hand settle into a
                            // slide. Turn sign is empirically inverted
                            // for the SDL frame (turn right = negative
                            // rotation around up = cursor right).
                            let yaw_rate = -rotation.dot(up);
                            self.laser_yaw_pending += yaw_rate * dt;
                            // Elevation of the pointing axis: SDL's +Z is
                            // toward the user, so the nose is -Z and a
                            // nose-up hold reads up.z negative. atan2
                            // rather than asin: asin's derivative blows
                            // up near ±1, turning accelerometer noise
                            // into huge angle swings.
                            let elevation =
                                (-up.z).atan2((up.x * up.x + up.y * up.y).sqrt());
                            if let Some(previous) = self.laser_pitch_prev {
                                self.laser_pitch_pending += elevation - previous;
                            }
                        self.laser_pitch_prev = Some(elevation);
                        (yaw_rate, 0.0, true)
                    }
                    None => Self::controller_space(rotation),
                },
            GyroOrientation::PlayerSpace => match self.gravity.and_then(|gravity| gravity.normalized())
            {
                Some(gravity) => Self::player_space(rotation, gravity),
                None => Self::controller_space(rotation),
            },
            GyroOrientation::WorldSpace => match self.gravity.and_then(|gravity| gravity.normalized())
            {
                Some(gravity) => self.world_space(rotation, gravity),
                None => Self::controller_space(rotation),
            },
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

    /// Accelerometer input in the unit the sensor layer delivers. NOTE: SDL3
    /// (and every SensorSample producer) reports m/s², so on real hardware
    /// the magnitude gate below rejects every reading and the
    /// gravity-referenced orientations degrade to controller-space axes —
    /// the behavior every tuned profile to date was built on. Normalizing
    /// units at the sensor boundary is deliberate follow-up work: doing it
    /// here silently changes what existing profiles feel like.
    /// Accelerometer input in m/s², the unit every sensor producer
    /// delivers; the gravity filters work in g, so convert once, here.
    /// Without this the magnitude gate below rejected every real reading,
    /// gravity never locked, and the gravity-referenced orientations
    /// silently degraded to raw controller axes (roll-as-yaw on a tilted
    /// hold).
    /// Gravity estimate (up direction in the controller frame), fused per
    /// JibbSmart's GamepadMotionHelpers: the estimate is carried by the
    /// calibrated gyro rotation, then corrected toward the accelerometer at
    /// a speed that drops when the accelerometer is shaky (linear
    /// acceleration) and is capped relative to the gyro rate — a still hand
    /// pins it in place while motion is carried gyro-clean. This is what
    /// keeps the Laser Pointer's absolute vertical from jittering with
    /// every tremor.
    fn update_gravity(&mut self, rotation: Vec3, accel: Option<[f32; 3]>, dt: f32) {
        if let Some(up) = self.gravity.as_mut() {
            // carry the estimate against the measured rotation: a
            // world-fixed direction in a rotating body frame rotates
            // opposite the body
            *up = up.minus(rotation.cross(*up).scale(dt));
        }
        let Some(sample) = accel else {
            return;
        };
        let accel = Vec3::from_array(sample).scale(1.0 / GRAVITY_MS2);
        let magnitude = accel.magnitude();
        if !magnitude.is_finite() {
            return;
        }
        // shake/free-fall garbage cannot correct the estimate
        let Some(direction) = (MIN_GRAVITY_G..=2.5)
            .contains(&magnitude)
            .then(|| accel.normalized())
            .flatten()
        else {
            return;
        };
        let Some(mut up) = self.gravity else {
            self.gravity = Some(direction);
            return;
        };
        // Shakiness: how far the raw accelerometer strays from its smoothed
        // self, with a 0.25 s-half-life memory.
        let smooth_factor = (-dt / SHAKE_HALF_TIME).exp2();
        self.shakiness *= smooth_factor;
        let wobble = accel.minus(self.smooth_accel).magnitude();
        if wobble > self.shakiness {
            self.shakiness = wobble;
        }
        self.smooth_accel = accel.lerp(self.smooth_accel, smooth_factor);
        // Correction speed: fast while steady, crawling while shaky, and
        // capped relative to the gyro rate so it never fights real motion.
        let blend = ((self.shakiness - SHAKE_MIN) / (SHAKE_MAX - SHAKE_MIN)).clamp(0.0, 1.0);
        let mut speed =
            CORRECTION_STILL_SPEED + (CORRECTION_SHAKY_SPEED - CORRECTION_STILL_SPEED) * blend;
        let cap = (rotation.magnitude() * CORRECTION_GYRO_FACTOR).max(CORRECTION_MIN_SPEED);
        let to_target = direction.minus(up);
        if speed > cap {
            let close_enough = ((to_target.magnitude() - CLOSE_ENOUGH_MIN)
                / (CLOSE_ENOUGH_MAX - CLOSE_ENOUGH_MIN))
                .clamp(0.0, 1.0);
            speed = cap + (speed - cap) * close_enough;
        }
        if let Some(step_direction) = to_target.normalized() {
            let step = step_direction.scale(speed * dt);
            up = if step.magnitude() < to_target.magnitude() {
                up.plus(step)
            } else {
                direction
            };
        }
        self.gravity = Some(up.normalized().unwrap_or(up));
    }

    /// World Space, per GamepadMotionHelpers: horizontal is the full
    /// gravity-referenced turn rate; the pitch axis is the pad's local X
    /// projected onto the horizontal plane, faded out by the side
    /// reduction as the pad stands on its side (where pitch would flip).
    fn world_space(&self, rotation: Vec3, gravity: Vec3) -> (f32, f32, bool) {
        let yaw = -(gravity.dot(rotation));
        let pitch_axis = Vec3 {
            x: 1.0 - gravity.x * gravity.x,
            y: -gravity.y * gravity.x,
            z: -gravity.z * gravity.x,
        };
        let flatness = gravity.y.abs().max(gravity.z.abs());
        let side_reduction =
            ((flatness - SIDE_REDUCTION_THRESHOLD) / SIDE_REDUCTION_THRESHOLD).clamp(0.0, 1.0);
        let pitch = pitch_axis
            .normalized()
            .map(|axis| axis.dot(rotation) * side_reduction)
            .unwrap_or(0.0);
        (yaw, pitch, true)
    }

    /// Player Space, per GamepadMotionHelpers: vertical is the raw local
    /// pitch — accelerometer error can never touch it — and horizontal is
    /// the turn rate around the vertical projected onto the pad's own
    /// horizontal plane, with a relax factor that keeps steep rolls from
    /// collapsing the reading (capped at the full horizontal magnitude).
    fn player_space(rotation: Vec3, gravity: Vec3) -> (f32, f32, bool) {
        let world_yaw = -(gravity.y * rotation.y + gravity.z * rotation.z);
        let sign = if world_yaw < 0.0 { -1.0 } else { 1.0 };
        let horizontal = (rotation.y * rotation.y + rotation.z * rotation.z).sqrt();
        let yaw = sign * (world_yaw.abs() * YAW_RELAX_FACTOR).min(horizontal);
        (yaw, rotation.x, true)
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

    const G_TO_MS2: f32 = 9.80665;

    fn processor() -> GyroProcessor {
        GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions::default(),
        )
    }

    /// Feed gravity long enough for the fusion to converge, then the
    /// rotation long enough for the adaptive smoothing to reach steady
    /// state, and return the final rates. Gravity is given in g; the
    /// processor consumes m/s².
    fn sample(processor: &mut GyroProcessor, rotation: [f32; 3], gravity: [f32; 3]) -> GyroRates {
        let gravity = [gravity[0] * G_TO_MS2, gravity[1] * G_TO_MS2, gravity[2] * G_TO_MS2];
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
        // Rotation +0.5 around the vertical is a turn left: the validated
        // SDL-frame sign reads it negative so the cursor moves left.
        assert!((rates.yaw + 0.5).abs() < 0.001, "yaw: {}", rates.yaw);
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
        assert!((rates.yaw + 0.5).abs() < 0.001, "yaw: {}", rates.yaw);
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
    fn test_player_space_vertical_is_local_pitch() {
        let mut processor = processor();
        // Controller held with X vertical: gravity has no component in the
        // pad's horizontal plane, so the yaw legitimately reads zero (the
        // degenerate hold) — while the vertical stays raw local pitch,
        // untouched by where gravity points.
        let rates = sample(&mut processor, [0.4, 0.4, 0.0], [1.0, 0.0, 0.0]);
        assert!(rates.yaw.abs() < 0.01, "yaw: {}", rates.yaw);
        assert!((rates.pitch - 0.4).abs() < 0.001, "pitch: {}", rates.pitch);
        assert!(rates.gravity_locked);
    }

    #[test]
    fn test_seed_bias_is_subtracted() {
        let bias = ControllerCalibration {
            x: 0.01,
            y: -0.02,
            z: 0.03,
            stick_deadzone_left: 0.0,
            stick_deadzone_right: 0.0,
            nintendo_layout: false,
        };
        let mut processor = GyroProcessor::new(
            bias,
            GyroProcessingOptions {
                auto_calibrate: false,
                ..GyroProcessingOptions::default()
            },
        );
        let rates = sample(&mut processor, [0.01, -0.02, 0.53], [0.0, 0.0, 1.0]);
        assert!((rates.yaw + 0.5).abs() < 0.001, "yaw: {}", rates.yaw);
    }

    #[test]
    fn test_auto_calibration_converges_to_constant_bias() {
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        // Controller resting with a hidden Z bias of 0.2 rad/s.
        for _ in 0..(250 * 4) {
            processor.process([0.0, 0.0, 0.2], Some([0.0, 0.0, G_TO_MS2]), DT);
        }
        assert!(processor.bias().z > 0.15, "bias: {:?}", processor.bias());
        let rates = processor.process([0.0, 0.0, 0.2], Some([0.0, 0.0, G_TO_MS2]), DT);
        assert!(rates.yaw.abs() < 0.05, "yaw after cal: {}", rates.yaw);
    }

    #[test]
    fn test_auto_calibration_converges_at_sparse_report_rate() {
        // A 20 Hz pad resting with a hidden Z bias: the stillness window is
        // wall-clock based, so a few seconds of rest must flush the motion
        // history and converge the bias — not 250 samples' worth of time.
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        // 2 s of motion first, so the window holds moving samples.
        for _ in 0..40 {
            processor.process([0.0, 0.0, 0.5], Some([0.0, 0.0, G_TO_MS2]), 0.05);
        }
        // 5 s of stillness at 20 Hz.
        for _ in 0..100 {
            processor.process([0.0, 0.0, 0.05], Some([0.0, 0.0, G_TO_MS2]), 0.05);
        }
        assert!(
            processor.bias().z > 0.02 && processor.bias().z < 0.051,
            "bias: {:?}",
            processor.bias()
        );
    }

    #[test]
    fn test_auto_calibration_ignores_sustained_rotation() {
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        // A slow constant turn has zero variance but is deliberate motion
        // (0.5 rad/s ≈ 29 °/s); it must never be folded into the bias.
        for _ in 0..(250 * 4) {
            processor.process([0.0, 0.0, 0.5], Some([0.0, 0.0, G_TO_MS2]), DT);
        }
        assert!(
            processor.bias().z.abs() < 0.01,
            "bias: {:?}",
            processor.bias()
        );
    }

    #[test]
    fn test_auto_calibration_ignores_changing_motion() {
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        for i in 0..(250 * 4) {
            let value = ((i % 250) as f32 / 250.0) * 1.5;
            processor.process([0.0, 0.0, value], Some([0.0, 0.0, G_TO_MS2]), DT);
        }
        assert!(
            processor.bias().z.abs() < 0.01,
            "bias: {:?}",
            processor.bias()
        );
    }

    #[test]
    fn test_player_space_locks_gravity_from_real_sdl_accelerometer() {
        // A real 8BitDo Ultimate 2 at rest through SDL3 reads ~9.7 m/s²
        // (SI units). The gravity pipeline converts, so the reading must
        // lock and player-space decomposition must engage — before the
        // conversion this magnitude failed the 0.3..=2.5 g gate and every
        // orientation silently degraded to controller-space axes.
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                ..GyroProcessingOptions::default()
            },
        );
        // Tilted hold: up ≈ (0.0, 0.69, -0.72) g. Turning the pad right
        // is a negative rotation around up: -0.5 rad/s * up.
        let up_ms2 = [0.0, 0.69 * G_TO_MS2, -0.72 * G_TO_MS2];
        let rotation = [0.0, -0.345, 0.36];
        let mut rates = GyroRates::default();
        for _ in 0..200 {
            rates = processor.process(rotation, Some(up_ms2), DT);
        }
        assert!(rates.gravity_locked, "m/s² readings must lock gravity");
        // Player-space yaw reads the full turn rate regardless of tilt.
        assert!(
            (rates.yaw.abs() - 0.5).abs() < 0.01,
            "yaw: {}",
            rates.yaw
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
        assert!((rates.yaw + 0.5).abs() < 0.01, "yaw: {}", rates.yaw);
    }

    #[test]
    fn test_smoothing_damps_slow_noise_and_passes_flicks() {
        let options = GyroProcessingOptions {
            smoothing: true,
            auto_calibrate: false,
            ..GyroProcessingOptions::default()
        };
        let mut slow = GyroProcessor::new(ControllerCalibration::default(), options);
        let mut fast = GyroProcessor::new(ControllerCalibration::default(), options);
        let mut raw = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                auto_calibrate: false,
                orientation: GyroOrientation::PlayerSpace,
            },
        );
        for i in 0..200 {
            // Hand tremor at slow aiming speed: alternates ±0.02 rad/s.
            let noise = if i % 2 == 0 { 0.02 } else { -0.02 };
            slow.process([0.0, 0.0, noise], Some([0.0, 0.0, G_TO_MS2]), DT);
            fast.process([0.0, 0.0, 2.0], Some([0.0, 0.0, G_TO_MS2]), DT);
            raw.process([0.0, 0.0, 2.0], Some([0.0, 0.0, G_TO_MS2]), DT);
        }
        let damped = slow
            .process([0.0, 0.0, 0.02], Some([0.0, 0.0, G_TO_MS2]), DT)
            .yaw;
        assert!(damped.abs() < 0.01, "damped yaw: {damped}");
        let flick = fast.process([0.0, 0.0, 2.0], Some([0.0, 0.0, G_TO_MS2]), DT).yaw;
        let raw_flick = raw.process([0.0, 0.0, 2.0], Some([0.0, 0.0, G_TO_MS2]), DT).yaw;
        assert!(
            (flick - raw_flick).abs() < 0.01,
            "flick: {flick} vs {raw_flick}"
        );
    }

    #[test]
    fn test_local_orientations_use_controller_axes() {
        // Passthrough (Local): raw axes, gravity data ignored entirely.
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                orientation: GyroOrientation::Local,
                ..GyroProcessingOptions::default()
            },
        );
        let rates = processor.process([0.1, 0.4, -0.2], Some([0.0, G_TO_MS2, 0.0]), DT);
        assert!((rates.yaw - 0.4).abs() < 0.001);
        assert!((rates.pitch - 0.1).abs() < 0.001);
        assert!(!rates.gravity_locked);

        // Yaw preset: rotation about the controller's own vertical axis.
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                orientation: GyroOrientation::Yaw,
                ..GyroProcessingOptions::default()
            },
        );
        let rates = processor.process([0.0, 0.4, 0.0], Some([0.0, G_TO_MS2, 0.0]), DT);
        assert!((rates.yaw - 0.4).abs() < 0.001);
        assert!((rates.pitch - 0.0).abs() < 0.001);

        // Roll preset: lean around the forward axis drives horizontal.
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
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
            ControllerCalibration::default(),
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
    fn test_laser_pointer_accumulates_turn_position() {
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                auto_calibrate: false,
                orientation: GyroOrientation::LaserPointer,
            },
        );
        // Hold with up ≈ (0, 0.69, -0.72) g and turn right (negative
        // rotation around up) at ≈0.5 rad/s for 1 s: the drained yaw
        // position delta must be +0.5 rad (cursor right).
        let up_ms2 = [0.0, 0.69 * G_TO_MS2, -0.72 * G_TO_MS2];
        let rotation = [0.0, -0.345, 0.36];
        for _ in 0..250 {
            processor.process(rotation, Some(up_ms2), DT);
        }
        let (yaw, pitch) = processor.take_laser_deltas();
        assert!((yaw - 0.5).abs() < 0.02, "yaw delta: {yaw}");
        assert!(pitch.abs() < 0.001);
    }

    #[test]
    fn test_laser_pointer_accumulates_elevation_position() {
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                auto_calibrate: false,
                orientation: GyroOrientation::LaserPointer,
            },
        );
        // Pointing the nose up by θ puts world-up at (0, cos θ, -sin θ) in
        // the pad frame (+Z is toward the user; the nose is -Z). A real
        // sweep carries gyro signal: rotation +0.5 rad/s about the pitch
        // axis with the accel direction following the same rotation. The
        // fusion must track it: drained pitch delta ≈ the 30° sweep.
        let elevation_at =
            |tilt: f32| [0.0, tilt.cos() * G_TO_MS2, -tilt.sin() * G_TO_MS2];
        for _ in 0..200 {
            processor.process([0.0; 3], Some(elevation_at((30.0f32).to_radians())), DT);
        }
        processor.take_laser_deltas();
        let sweep = (30.0f32).to_radians();
        for step in 0..250 {
            let tilt = (30.0f32).to_radians() + sweep * (step as f32 * DT);
            processor.process([0.5, 0.0, 0.0], Some(elevation_at(tilt)), DT);
        }
        let (yaw, pitch) = processor.take_laser_deltas();
        assert!(yaw.abs() < 0.001, "yaw delta: {yaw}");
        assert!((pitch - sweep).abs() < 0.06, "pitch delta: {pitch}");
    }

    #[test]
    fn test_laser_pointer_ignores_accelerometer_noise() {
        // Hand tremor shakes the accelerometer without any real rotation:
        // the fusion's shakiness gate must keep the elevation — and so the
        // cursor — essentially still. Before the fusion this jitter slid
        // the cursor continuously.
        let mut processor = GyroProcessor::new(
            ControllerCalibration::default(),
            GyroProcessingOptions {
                smoothing: false,
                auto_calibrate: false,
                orientation: GyroOrientation::LaserPointer,
            },
        );
        for _ in 0..200 {
            processor.process([0.0; 3], Some([0.0, 0.0, G_TO_MS2]), DT);
        }
        processor.take_laser_deltas();
        let mut worst = 0.0f32;
        for step in 0..400 {
            let wobble = if step % 2 == 0 { 0.3 } else { -0.3 };
            let rates = processor.process(
                [0.0; 3],
                Some([wobble * G_TO_MS2, 0.0, G_TO_MS2]),
                DT,
            );
            worst = worst.max(rates.pitch.abs());
        }
        let (_, pitch) = processor.take_laser_deltas();
        assert!(pitch.abs() < 0.05, "elevation drifted: {pitch}");
        assert!(worst < 0.5, "per-sample jitter rate: {worst}");
    }

    #[test]
    fn test_bias_accessor_round_trips_into_new_processor() {
        let bias = ControllerCalibration {
            x: 0.05,
            y: 0.0,
            z: -0.05,
            stick_deadzone_left: 0.0,
            stick_deadzone_right: 0.0,
            nintendo_layout: false,
        };
        let mut processor = GyroProcessor::new(
            bias,
            GyroProcessingOptions {
                auto_calibrate: false,
                ..GyroProcessingOptions::default()
            },
        );
        for _ in 0..10 {
            processor.process([0.0, 0.0, 0.0], Some([0.0, 0.0, G_TO_MS2]), DT);
        }
        assert_eq!(processor.bias(), bias);
    }
}
