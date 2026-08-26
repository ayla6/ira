//! Rumble passthrough: effects a game plays on the virtual pad are replayed
//! on the physical controller through its evdev force-feedback interface.
//!
//! The physical node gets its own read-write handle so the reading side is
//! untouched; one two-motor effect stays uploaded and is updated in place,
//! which is what the kernel's EVIOCSFF round-trip makes cheap.

use std::path::Path;

use evdev::{Device, FFEffect, FFEffectCode, FFEffectData, FFEffectKind, FFReplay, FFTrigger};

/// One rumble request for a physical pad. Magnitudes use the kernel's
/// 0..=65535 scale: strong is the heavy left motor, weak the light right one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleCommand {
    pub strong: u16,
    pub weak: u16,
    /// How long the motors run before the kernel stops them.
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

/// Plays rumble commands on one physical controller. `None`-like states are
/// expressed by [`PhysicalRumble::open`] returning an error string once, so
/// callers log the reason exactly once per session.
pub struct PhysicalRumble {
    device: Device,
    effect: Option<FFEffect>,
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
            device,
            effect: None,
        })
    }

    /// Runs the motors. Re-uploads keep the same effect id, so repeated
    /// commands are a cheap ioctl plus one event write.
    pub fn play(&mut self, command: RumbleCommand) {
        let duration = command.duration_ms.clamp(MIN_DURATION_MS, MAX_DURATION_MS);
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
        let result = match self.effect.as_mut() {
            Some(effect) => effect.update(data).and_then(|()| effect.play(1)),
            None => match self.device.upload_ff_effect(data) {
                Ok(effect) => {
                    self.effect = Some(effect);
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

    /// Stops the motors immediately (pause, disconnect, shutdown).
    pub fn stop(&mut self) {
        if let Some(error) = self.effect.as_mut().and_then(|effect| effect.stop().err()) {
            eprintln!("ira-input: rumble stop failed: {error}");
        }
    }
}

impl std::fmt::Debug for PhysicalRumble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicalRumble")
            .field("uploaded", &self.effect.is_some())
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
