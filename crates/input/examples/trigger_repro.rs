//! Debug: feed trigger input through a profile and show what the DSU shadow
//! would broadcast. Usage: trigger_repro <profile.json>

use ira_input::{GamepadAxis, GamepadButton, InputSource, PadState};
use ira_input::{InputEvent, MappingEngine};

fn main() {
    let path = std::env::args().nth(1).expect("profile path");
    let raw = std::fs::read_to_string(&path).unwrap();
    let profile = ira_input::InputProfile::from_json(&raw).unwrap();
    println!("backend = {:?}", profile.backend);
    let mut engine = MappingEngine::new(profile).unwrap();
    let mut pad = PadState::default();
    let feed = |engine: &mut MappingEngine, pad: &mut PadState, label: &str, source, value| {
        let outputs = engine.process(InputEvent {
            source,
            value,
            timestamp_us: 0,
        });
        for out in &outputs {
            println!("  {label} -> {out:?}");
            pad.apply_output(out);
        }
        if outputs.is_empty() {
            println!("  {label} -> (no outputs)");
        }
    };
    feed(
        &mut engine,
        &mut pad,
        "button L press",
        InputSource::Button(GamepadButton::LeftTrigger),
        1.0,
    );
    feed(
        &mut engine,
        &mut pad,
        "button L release",
        InputSource::Button(GamepadButton::LeftTrigger),
        0.0,
    );
    for v in [0.0f32, 0.5, 1.0, 0.0] {
        feed(
            &mut engine,
            &mut pad,
            &format!("axis L {v}"),
            InputSource::Axis(GamepadAxis::LeftTrigger),
            v,
        );
    }
    println!(
        "final pad: l2={} r2={} state2-bit0={}",
        pad.l2,
        pad.r2,
        pad.l2 > 0.0
    );
}
