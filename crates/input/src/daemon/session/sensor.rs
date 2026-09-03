use std::time::{SystemTime, UNIX_EPOCH};

use crate::{GyroProcessor, InputProfile, MappingEngine, SensorSample};

use super::outputs::{emit_outputs, OutputTargets};
use super::super::TraceState;

/// Sensor-side state advanced by the tick loop.
pub(crate) struct SensorPipeline {
    /// Whether the pad currently has any motion source (kernel IMU, SDL,
    /// or Switch-protocol takeover) — the hub reports transitions.
    pub(crate) motion_available: bool,
    pub(crate) gyro_processor: GyroProcessor,
    pub(crate) last_sensor_us: Option<u64>,
    pub(crate) motion: Option<crate::MotionServer>,
    pub(crate) motion_device: Option<crate::VirtualMotionSensor>,
    pub(crate) ds4_hid: Option<crate::Ds4UhidDevice>,
    pub(crate) dualsense_hid: Option<crate::DualsenseUhidDevice>,
    pub(crate) imu_hid: Option<crate::ImuUhidDevice>,
    pub(crate) switch_pro_hid: Option<crate::SwitchProUhidDevice>,
    /// Whether a gyro sensor was available at startup. Gyroless DSU
    /// sessions still stream whole-controller frames with zeroed motion.
    pub(crate) ever_had_sensor: bool,
    /// Last motion timestamp sent over cemuhook. Cemu drops any sample
    /// whose timestamp does not advance and force-resets on >10s backwards
    /// jumps, so outbound stamps are clamped to move forward on one clock.
    pub(crate) last_dsu_ts: u64,
}

/// Whether the tick loop must run for this session state. Ticks drive
/// continuous outputs (mouse motion, gyro axes, cemuhook frames), so they
/// are needed whenever any of those consumers exists.
pub(crate) fn tick_needed_for(pipeline: &SensorPipeline, mapper: &MappingEngine) -> bool {
    pipeline.motion_alive()
        || mapper.has_continuous_outputs()
        || mapper.profile().backend == crate::VirtualGamepadBackend::Dsu
        || pipeline.motion.is_some()
        || pipeline.ds4_hid.is_some()
}

pub(crate) fn open_motion_node(
    backend: crate::VirtualGamepadBackend,
) -> Option<crate::VirtualMotionSensor> {
    match crate::VirtualMotionSensor::create(backend) {
        Ok(device) => {
            eprintln!("ira-input: native gyro exposed through evdev motion node (SDL sensors)");
            Some(device)
        }
        Err(error) => {
            eprintln!("ira-input: failed to create native motion node: {error}");
            None
        }
    }
}

/// Answers pending kernel events on every live uhid twin without
/// streaming a state report. Runs unconditionally each daemon pass so
/// handshakes and probes survive idle or paused stretches.
impl SensorPipeline {
    /// Whether any motion source is currently live.
    pub(crate) fn motion_alive(&self) -> bool {
        self.motion_available
    }
}

pub(crate) fn service_twin_events(pipeline: &mut SensorPipeline) {
    if pipeline.switch_pro_hid.is_some() {
        if let Err(error) = pipeline.switch_pro_hid.as_mut().unwrap().service() {
            eprintln!("ira-input: virtual Switch Pro stopped: {error}");
            pipeline.switch_pro_hid = None;
        }
    }
    if pipeline.ds4_hid.is_some() {
        if let Err(error) = pipeline.ds4_hid.as_mut().unwrap().service() {
            eprintln!("ira-input: virtual DS4 stopped: {error}");
            pipeline.ds4_hid = None;
        }
    }
    if pipeline.dualsense_hid.is_some() {
        if let Err(error) = pipeline.dualsense_hid.as_mut().unwrap().service() {
            eprintln!("ira-input: virtual DualSense stopped: {error}");
            pipeline.dualsense_hid = None;
        }
    }
}

/// Creates the flatpak-visible motion half of a native twin: SDL pairs it
/// with the pad by matching EVIOCGUNIQ serials — the same pairing the DS4
/// flavor uses, and the gyro path that works everywhere today.
pub(crate) fn spawn_paired_imu(uniq: &str) -> Option<crate::ImuUhidDevice> {
    match crate::ImuUhidDevice::create(uniq) {
        Ok(device) => {
            eprintln!("ira-input: paired virtual IMU exposed over evdev");
            Some(device)
        }
        Err(error) => {
            eprintln!("ira-input: failed to create virtual IMU: {error}");
            None
        }
    }
}

pub(crate) fn process_tick(
    pipeline: &mut SensorPipeline,
    mapper: &mut MappingEngine,
    targets: OutputTargets<'_>,
    trace: &mut TraceState,
    run: bool,
    mut readings: Vec<SensorSample>,
) -> Result<(), String> {
    if !run {
        return Ok(());
    }
    let tick_now = now_us();
    // The raw sensor readings, kept unconverted so each consumer can request
    // its own frame below. The hub forwards every physical sample; each is
    // integrated with its own time delta — dropping one would stretch the
    // next window and fabricate rotation out of micro-motion. A tick
    // without fresh samples still carries state forward.
    let mut latest: Option<([f32; 3], [f32; 3], u64)> = None;
    for sample in readings.drain(..) {
        let dt = pipeline
            .last_sensor_us
            .map(|last| sample.timestamp_us.saturating_sub(last) as f32 / 1_000_000.0)
            .unwrap_or(1.0 / 250.0)
            .clamp(0.0005, 0.05);
        pipeline.last_sensor_us = Some(sample.timestamp_us);
        trace.record_gyro(sample.gyro);
        if let Some(motion_device) = pipeline.motion_device.as_mut() {
            // Native passthrough: SDL-based emulators read the same
            // unfiltered sensor straight from the virtual pad.
            if let Err(error) = motion_device
                .emit_sample(sample.gyro, sample.accel.unwrap_or([0.0, 0.0, 0.0]))
            {
                eprintln!("ira-input: native motion node failed: {error}");
            }
        }
        // Raw passthrough: emulators consuming the DSU stream get
        // the unfiltered sensor, independent of the mapping
        // profile's gyro processing.
        latest = Some((
            sample.gyro,
            sample.accel.unwrap_or([0.0, 0.0, 0.0]),
            sample.timestamp_us,
        ));
        let rates = pipeline
            .gyro_processor
            .process(sample.gyro, sample.accel, dt);
        let bias = pipeline.gyro_processor.bias();
        trace.record_sample(
            dt,
            sample.gyro,
            sample.accel,
            [bias.x, bias.y, bias.z],
            [rates.yaw, rates.pitch],
        );
        mapper.update_gyro(rates, tick_now);
        // Laser Pointer position deltas ride alongside the rates: the
        // processor accumulates angles, the mapper emits them directly.
        if mapper.profile().gyro.orientation == crate::GyroOrientation::LaserPointer {
            let (yaw_delta, pitch_delta) = pipeline.gyro_processor.take_laser_deltas();
            mapper.update_gyro_position(yaw_delta, pitch_delta);
        }
    }
    // One sensor reading feeds every consumer, each in its own frame: the
    // virtual DS4 and the cemuhook stream both carry our SDL frame with
    // wire yaw/roll negated where their respective ingestion paths apply
    // [gx, -gy, -gz], the Switch Pro pre-shuffles into the Nintendo device
    // frame, and ticks without a fresh sensor sample still carry state
    // with zeroed motion.
    let zero = ([0.0f32; 3], [0.0f32; 3], now_us());
    // Native motion is not a raw relay: the profile's gyro shaping applies
    // to the rates written on the wire. Sensitivity scales them and the
    // invert flags flip SDL's yaw (index 1) and pitch (index 0); only the
    // orientation math stays out — the game reads the sensor axes as the
    // device reports them. One multiply per sample, nothing added to the
    // per-sample path.
    let shaped = native_gyro_shaping(mapper.profile());
    if pipeline.ds4_hid.is_some() {
        let (gyro, accel, timestamp_us) = latest.unwrap_or(zero);
        let hid_frame = crate::sensor_to_motion(shaped(gyro), accel, timestamp_us);
        let ds4 = pipeline.ds4_hid.as_mut().unwrap();
        match ds4.send_state(targets.pad, &hid_frame) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("ira-input: virtual DS4 stopped: {error}");
                pipeline.ds4_hid = None;
            }
        }
    }
    if pipeline.dualsense_hid.is_some() {
        // SDL's PS5 driver passes axes through untouched like its DS4 one,
        // so the source SDL frame goes on the wire verbatim.
        let (gyro, accel, timestamp_us) = latest.unwrap_or(zero);
        let hid_frame = crate::sensor_to_motion(shaped(gyro), accel, timestamp_us);
        let dualsense = pipeline.dualsense_hid.as_mut().unwrap();
        match dualsense.send_state(targets.pad, &hid_frame) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("ira-input: virtual DualSense stopped: {error}");
                pipeline.dualsense_hid = None;
            }
        }
    }
    if pipeline.switch_pro_hid.is_some() {
        let (gyro, accel, _) = latest.unwrap_or(zero);
        const GRAVITY: f32 = 9.80665;
        let accel_g = [accel[0] / GRAVITY, accel[1] / GRAVITY, accel[2] / GRAVITY];
        let gyro_dps = shaped([gyro[0].to_degrees(), gyro[1].to_degrees(), gyro[2].to_degrees()]);
        let switch_pro = pipeline.switch_pro_hid.as_mut().unwrap();
        if let Err(error) = switch_pro.tick(targets.pad, accel_g, gyro_dps) {
            eprintln!("ira-input: virtual Switch Pro stopped: {error}");
            pipeline.switch_pro_hid = None;
        }
    }
    if pipeline.imu_hid.is_some() {
        let (gyro, accel, _) = latest.unwrap_or(zero);
        const GRAVITY: f32 = 9.80665;
        let accel_g = [accel[0] / GRAVITY, accel[1] / GRAVITY, accel[2] / GRAVITY];
        let gyro_dps = shaped([gyro[0].to_degrees(), gyro[1].to_degrees(), gyro[2].to_degrees()]);
        let imu = pipeline.imu_hid.as_mut().unwrap();
        if let Err(error) = imu.send_sample(accel_g, gyro_dps) {
            eprintln!("ira-input: virtual IMU stopped: {error}");
            pipeline.imu_hid = None;
        }
    }
    if let Some(motion) = pipeline.motion.as_mut() {
        // Frames go out per fresh sensor sample, not per tick: Cemu
        // integrates wire timestamps as sample deltas, so duplicating a
        // sample across ticks double-counts its rotation and zeroed
        // frames between samples read as instant freefall. Gyroless
        // sessions (or a dead sensor) still stream whole-controller
        // frames with zeroed motion at tick rate.
        if should_send_dsu_frame(
            latest.is_some(),
            pipeline.motion_available,
            pipeline.ever_had_sensor,
        ) {
            // Stamp with the sensor sample's own clock when available and
            // clamp forward otherwise: Cemu drops non-advancing timestamps
            // and resets on >10s backwards jumps, so fallback frames must
            // never travel backwards or collide with sample stamps.
            let (gyro, accel, sample_ts) = latest.take().unwrap_or(([0.0; 3], [0.0; 3], 0));
            let ts = if sample_ts > pipeline.last_dsu_ts {
                sample_ts
            } else {
                pipeline.last_dsu_ts + 1
            };
            pipeline.last_dsu_ts = ts;
            let dsu_frame = crate::sensor_to_dsu_frame(gyro, accel, ts);
            motion.poll_clients(true);
            if mapper.profile().backend == crate::VirtualGamepadBackend::Dsu {
                motion.send_sample(&dsu_frame, targets.pad);
            } else {
                // Companion stream next to a real virtual pad: the game
                // already reads that pad's buttons, so the stream's own
                // state stays neutral and only motion carries data.
                motion.send_motion_only(&dsu_frame);
            }
        }
    }
    let outputs = mapper.tick(tick_now);
    emit_outputs(outputs, targets, trace)
}

/// Whether a cemuhook frame should leave this tick: always for a fresh
/// sensor sample; also at tick rate when no gyro ever existed or the
/// sensor just died, so whole-controller state keeps flowing.
fn should_send_dsu_frame(fresh_sample: bool, sensor_alive: bool, ever_had_sensor: bool) -> bool {
    fresh_sample || !sensor_alive || !ever_had_sensor
}

/// The rate shaping applied to motion written on the native transports:
/// sensitivity and invert flags when the profile routes the gyro to native
/// motion sensors, identity otherwise.
pub(crate) fn native_gyro_shaping(
    profile: &InputProfile,
) -> impl Fn([f32; 3]) -> [f32; 3] + Send + 'static {
    let gyro = &profile.gyro;
    let active = gyro.enabled && gyro.output == crate::GyroOutput::NativeMotion;
    let scale = if active { gyro.sensitivity } else { 1.0 };
    let inverts = active.then_some((gyro.invert_x, gyro.invert_y));
    move |mut rates: [f32; 3]| {
        for rate in &mut rates {
            *rate *= scale;
        }
        if let Some((invert_x, invert_y)) = inverts {
            if invert_x {
                rates[1] = -rates[1]; // yaw: horizontal output
            }
            if invert_y {
                rates[0] = -rates[0]; // pitch: vertical output
            }
        }
        rates
    }
}

pub(crate) fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dsu_frames_follow_samples_not_ticks() {
        // Fresh samples always stream; ticks without one stay silent so the
        // same rotation is never integrated twice.
        assert!(should_send_dsu_frame(true, true, true));
        assert!(!should_send_dsu_frame(false, true, true));
        // A gyroless session keeps streaming whole-controller frames.
        assert!(should_send_dsu_frame(false, false, false));
        // A sensor dying mid-session must not freeze the buttons either.
        assert!(should_send_dsu_frame(false, false, true));
    }

    #[test]
    fn test_native_gyro_shaping_scales_and_inverts_only_when_native() {
        use crate::{GyroConfig, GyroOutput};
        let mut profile = InputProfile {
            gyro: GyroConfig {
                enabled: true,
                output: GyroOutput::NativeMotion,
                sensitivity: 2.0,
                invert_x: true,
                invert_y: true,
                ..GyroConfig::default()
            },
            ..InputProfile::default()
        };
        let shape = native_gyro_shaping(&profile);
        // Pitch (index 0) and yaw (index 1) scale and flip; roll (index 2)
        // only scales.
        let shaped = shape([1.0, -3.0, 5.0]);
        assert_eq!(shaped, [-2.0, 6.0, 10.0]);

        // Any other output leaves the sensor untouched.
        profile.gyro.output = GyroOutput::Mouse;
        let shape = native_gyro_shaping(&profile);
        assert_eq!(shape([1.0, -3.0, 5.0]), [1.0, -3.0, 5.0]);
        // A disabled native gyro is identity too.
        profile.gyro.output = GyroOutput::NativeMotion;
        profile.gyro.enabled = false;
        let shape = native_gyro_shaping(&profile);
        assert_eq!(shape([1.0, -3.0, 5.0]), [1.0, -3.0, 5.0]);
    }
}