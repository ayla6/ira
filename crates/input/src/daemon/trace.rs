use std::time::{Duration, Instant};

use crate::{InputEvent, OutputEvent};

pub(crate) struct TraceState {
    enabled: bool,
    last_report: Instant,
    gyro: [f32; 3],
    mouse: [f32; 2],
}

impl TraceState {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_report: Instant::now(),
            gyro: [0.0; 3],
            mouse: [0.0; 2],
        }
    }

    pub(crate) fn record_input(&mut self, event: InputEvent) {
        if self.enabled {
            eprintln!(
                "ira-input: input source={:?} value={:.3}",
                event.source, event.value
            );
        }
    }

    pub(crate) fn record_gyro(&mut self, gyro: [f32; 3]) {
        self.gyro = gyro;
    }

    /// Per-sample diagnostics for drift hunting: the integration delta, the
    /// raw sensor reading, and the rates the mapper receives.
    pub(crate) fn record_sample(&self, dt: f32, gyro: [f32; 3], rates: [f32; 3]) {
        if !self.enabled {
            return;
        }
        eprintln!(
            "ira-input: sample dt={dt:.5} gyro=({:.4}, {:.4}, {:.4}) \
             rates=({:.1}, {:.1}, {:.1}) dps",
            gyro[0], gyro[1], gyro[2], rates[0], rates[1], rates[2]
        );
    }

    pub(crate) fn record_output(&mut self, output: &OutputEvent) {
        if self.enabled
            && !matches!(
                output,
                OutputEvent::GamepadAxis { .. } | OutputEvent::MouseMotion { .. }
            )
        {
            eprintln!("ira-input: output {output:?}");
        }
        if let OutputEvent::MouseMotion { axis, value } = output {
            match axis {
                crate::MouseAxis::X => self.mouse[0] += value,
                crate::MouseAxis::Y => self.mouse[1] += value,
                crate::MouseAxis::Wheel | crate::MouseAxis::WheelX => {}
            }
        }
    }

    pub(crate) fn flush(&mut self) {
        if !self.enabled || self.last_report.elapsed() < Duration::from_millis(250) {
            return;
        }
        if self.gyro.iter().any(|value| value.abs() > 0.001)
            || self.mouse.iter().any(|value| value.abs() > 0.01)
        {
            eprintln!(
                "ira-input: trace gyro=({:.3}, {:.3}, {:.3}) mouse_delta=({:.2}, {:.2})",
                self.gyro[0], self.gyro[1], self.gyro[2], self.mouse[0], self.mouse[1],
            );
        }
        self.last_report = Instant::now();
        self.gyro = [0.0; 3];
        self.mouse = [0.0; 2];
    }
}
