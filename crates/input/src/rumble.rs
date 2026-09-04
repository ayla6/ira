//! Rumble passthrough: effects a game plays on the virtual pad are replayed
//! on the physical controller — through the kernel's evdev force-feedback
//! interface where the pad has one, or a vendor output-report protocol
//! written straight to its hidraw node where it does not (8BitDo pads in
//! DInput mode expose no evdev FF; SDL rumbles them the hidraw way, and so
//! do we). The 8BitDo wire format itself lives in hid_8bitdo.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use evdev::{Device, FFEffect, FFEffectCode, FFEffectData, FFEffectKind, FFReplay, FFTrigger};

use crate::hid_8bitdo::{
    enable_8bitdo_reports, needs_enable_handshake, rumble_report_8bitdo, VENDOR_8BITDO,
};

/// One rumble request for a physical pad. Magnitudes use the kernel's
/// 0..=65535 scale: strong is the heavy left motor, weak the light right one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleCommand {
    pub strong: u16,
    pub weak: u16,
    /// How long the motors run before the backend stops them.
    pub duration_ms: u16,
}

/// Translates an uploaded effect into a rumble command; non-rumble effects
/// (constant force on wheels, springs) have no gamepad equivalent and are
/// ignored.
pub(crate) fn rumble_command_from_effect(data: &FFEffectData) -> Option<RumbleCommand> {
    let FFEffectKind::Rumble {
        strong_magnitude,
        weak_magnitude,
    } = data.kind
    else {
        return None;
    };
    Some(RumbleCommand {
        strong: strong_magnitude,
        weak: weak_magnitude,
        duration_ms: data.replay.length,
    })
}

/// Effects longer than this are clamped so an infinite-length upload cannot
/// leave the motors running forever when the game crashes or stops updating.
const MAX_DURATION_MS: u16 = 5_000;
/// Short blips still need to be long enough to spin a real motor up.
const MIN_DURATION_MS: u16 = 20;

enum Backend {
    /// Kernel force feedback: the kernel stops the effect on its own timer.
    Evdev {
        device: Box<Device>,
        effect: Option<FFEffect>,
    },
    /// Vendor output-report protocol: packets go straight to the pad's
    /// hidraw node, so this side must silence the motors itself once the
    /// deadline set by `play` passes.
    Hidraw {
        file: File,
        stop_at: Option<Instant>,
    },
}

/// Plays rumble commands on one physical controller. `None`-like states are
/// expressed by the `open` constructors returning an error string once, so
/// callers log the reason exactly once per session.
pub struct PhysicalRumble {
    backend: Backend,
}

impl PhysicalRumble {
    /// Opens the force-feedback side of a controller. Fails when the node
    /// cannot be reopened read-write or declares no rumble capability (most
    /// wheels and generic HID devices without FF descriptors).
    pub fn open(path: &Path) -> Result<Self, String> {
        let device = Device::open(path)
            .map_err(|error| format!("rumble: cannot open {}: {error}", path.display()))?;
        let supports_rumble = device
            .supported_ff()
            .is_some_and(|effects| effects.contains(FFEffectCode::FF_RUMBLE));
        if !supports_rumble {
            return Err(format!(
                "rumble: {} reports no rumble capability",
                path.display()
            ));
        }
        Ok(Self {
            backend: Backend::Evdev {
                device: Box::new(device),
                effect: None,
            },
        })
    }

    /// Opens the vendor hidraw rumble path for a pad whose evdev nodes
    /// declare no force feedback. `event_path` is the pad's event node; the
    /// matching hidraw node is located through sysfs so only this pad's HID
    /// device is written, never a neighbouring controller's.
    pub fn open_vendor_hidraw(
        event_path: &Path,
        vendor: u16,
        product: u16,
    ) -> Result<Self, String> {
        if vendor != VENDOR_8BITDO {
            return Err(format!(
                "rumble: no vendor hidraw protocol for {vendor:#06x}"
            ));
        }
        let mut last_error = format!(
            "rumble: no hidraw node beside {} (is the pad connected through its dongle?)",
            event_path.display()
        );
        for node in sibling_hidraw_nodes(event_path) {
            match OpenOptions::new().write(true).open(&node) {
                Ok(mut file) => {
                    if needs_enable_handshake(product) {
                        enable_8bitdo_reports(&mut file);
                    }
                    return Ok(Self {
                        backend: Backend::Hidraw { file, stop_at: None },
                    });
                }
                Err(error) => {
                    last_error =
                        format!("rumble: cannot open {}: {error}", node.display());
                }
            }
        }
        Err(last_error)
    }

    /// Runs the motors. Evdev re-uploads keep the same effect id, so
    /// repeated commands are a cheap ioctl plus one event write; hidraw is
    /// one output report whose self-timed deadline is refreshed.
    pub fn play(&mut self, command: RumbleCommand) {
        let duration = command.duration_ms.clamp(MIN_DURATION_MS, MAX_DURATION_MS);
        match &mut self.backend {
            Backend::Evdev { device, effect } => {
                let data = FFEffectData {
                    direction: 0x4000,
                    trigger: FFTrigger::default(),
                    replay: FFReplay {
                        length: duration,
                        delay: 0,
                    },
                    kind: FFEffectKind::Rumble {
                        strong_magnitude: command.strong,
                        weak_magnitude: command.weak,
                    },
                };
                let result = match effect.as_mut() {
                    Some(effect) => effect.update(data).and_then(|()| effect.play(1)),
                    None => match device.upload_ff_effect(data) {
                        Ok(uploaded) => {
                            *effect = Some(uploaded);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    },
                };
                if let Err(error) = result {
                    // A disconnected or revoked device keeps failing until the
                    // caller rebuilds us after reconnect; report but never panic.
                    eprintln!("ira-input: rumble playback failed: {error}");
                }
            }
            Backend::Hidraw { file, stop_at } => {
                let packet = rumble_report_8bitdo(command.strong, command.weak);
                if let Err(error) = file.write_all(&packet) {
                    eprintln!("ira-input: rumble playback failed: {error}");
                    return;
                }
                *stop_at = Some(Instant::now() + Duration::from_millis(u64::from(duration)));
            }
        }
    }

    /// Stops the motors immediately (pause, disconnect, shutdown).
    pub fn stop(&mut self) {
        match &mut self.backend {
            Backend::Evdev { effect, .. } => {
                if let Some(error) = effect.as_mut().and_then(|effect| effect.stop().err()) {
                    eprintln!("ira-input: rumble stop failed: {error}");
                }
            }
            Backend::Hidraw { file, stop_at } => {
                if stop_at.is_some() {
                    silence_hidraw(file, stop_at);
                }
            }
        }
    }

    /// Advances self-timed rumble: the vendor hidraw protocol has no kernel
    /// timer, so the deadline each `play` sets must be checked here or the
    /// motors keep running after a game stops replaying its effect.
    pub fn service(&mut self) {
        let Backend::Hidraw { file, stop_at } = &mut self.backend else {
            return;
        };
        if stop_at.is_some_and(|deadline| Instant::now() >= deadline) {
            silence_hidraw(file, stop_at);
        }
    }
}

/// Writes the all-zero motor report and clears the deadline.
fn silence_hidraw(file: &mut File, stop_at: &mut Option<Instant>) {
    if let Err(error) = file.write_all(&rumble_report_8bitdo(0, 0)) {
        eprintln!("ira-input: rumble stop failed: {error}");
    }
    *stop_at = None;
}

/// Resolves the /dev/hidraw nodes of the HID device backing an evdev event
/// node: /sys/class/input/eventN/device links into the HID device's input
/// tree, and the HID device's own sysfs directory carries a hidraw/
/// subdirectory naming its character device.
pub(crate) fn sibling_hidraw_nodes(event_path: &Path) -> Vec<PathBuf> {
    let Some(stem) = event_path.file_name() else {
        return Vec::new();
    };
    let Ok(device) =
        std::fs::canonicalize(Path::new("/sys/class/input").join(stem).join("device"))
    else {
        return Vec::new();
    };
    let mut current = Some(device.as_path());
    for _ in 0..8 {
        let Some(dir) = current else {
            break;
        };
        let nodes: Vec<PathBuf> = std::fs::read_dir(dir.join("hidraw"))
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        name.starts_with("hidraw")
                            .then(|| PathBuf::from("/dev").join(name))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !nodes.is_empty() {
            return nodes;
        }
        current = dir.parent();
    }
    Vec::new()
}

impl std::fmt::Debug for PhysicalRumble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let uploaded = match &self.backend {
            Backend::Evdev { effect, .. } => effect.is_some(),
            Backend::Hidraw { .. } => true,
        };
        f.debug_struct("PhysicalRumble")
            .field("uploaded", &uploaded)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{rumble_command_from_effect, RumbleCommand, MAX_DURATION_MS, MIN_DURATION_MS};
    use evdev::{FFEffectData, FFEffectKind, FFReplay, FFTrigger};

    #[test]
    fn test_rumble_command_fields_round_trip() {
        let command = RumbleCommand {
            strong: 40_000,
            weak: 8_000,
            duration_ms: 250,
        };
        assert_eq!(command.strong, 40_000);
        assert_eq!(command.weak, 8_000);
        assert_eq!(command.duration_ms, 250);
    }

    #[test]
    fn test_rumble_command_from_rumble_effect() {
        let data = FFEffectData {
            direction: 0x4000,
            trigger: FFTrigger::default(),
            replay: FFReplay {
                length: 300,
                delay: 0,
            },
            kind: FFEffectKind::Rumble {
                strong_magnitude: 60_000,
                weak_magnitude: 5_000,
            },
        };
        let command = rumble_command_from_effect(&data).expect("rumble effects convert");
        assert_eq!(command.strong, 60_000);
        assert_eq!(command.weak, 5_000);
        assert_eq!(command.duration_ms, 300);
    }

    #[test]
    fn test_rumble_command_ignores_non_rumble_effects() {
        let data = FFEffectData {
            direction: 0x4000,
            trigger: FFTrigger::default(),
            replay: FFReplay::default(),
            kind: FFEffectKind::Constant {
                level: 20_000,
                envelope: evdev::FFEnvelope {
                    attack_length: 0,
                    attack_level: 0,
                    fade_length: 0,
                    fade_level: 0,
                },
            },
        };
        assert!(rumble_command_from_effect(&data).is_none());
    }

    #[test]
    fn test_duration_clamp_bounds_are_sane() {
        const { assert!(MIN_DURATION_MS > 0) };
        const { assert!(MAX_DURATION_MS >= MIN_DURATION_MS) };
    }
}