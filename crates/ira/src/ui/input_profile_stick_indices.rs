//! Bidirectional enum ↔ picker-index conversions for the stick sheet's
//! option rows, plus shared row formatting.

use ira_input::{ResponseAxisStyle, StickDeadzone, StickOutput, StickOutputAxis};

pub(super) fn deadzone_source_index(deadzone: StickDeadzone) -> usize {
    match deadzone {
        StickDeadzone::None => 0,
        StickDeadzone::Controller => 1,
        StickDeadzone::Custom => 2,
    }
}

pub(super) fn deadzone_from_index(index: usize) -> StickDeadzone {
    match index {
        1 => StickDeadzone::Controller,
        2 => StickDeadzone::Custom,
        _ => StickDeadzone::None,
    }
}

pub(super) fn output_index(output: StickOutput) -> usize {
    match output {
        StickOutput::Left => 0,
        StickOutput::Right => 1,
    }
}

pub(super) fn output_from_index(index: usize) -> StickOutput {
    match index {
        1 => StickOutput::Right,
        _ => StickOutput::Left,
    }
}

pub(super) fn output_axis_index(axis: StickOutputAxis) -> usize {
    match axis {
        StickOutputAxis::Both => 0,
        StickOutputAxis::Horizontal => 1,
        StickOutputAxis::Vertical => 2,
    }
}

pub(super) fn output_axis_from_index(index: usize) -> StickOutputAxis {
    match index {
        1 => StickOutputAxis::Horizontal,
        2 => StickOutputAxis::Vertical,
        _ => StickOutputAxis::Both,
    }
}

pub(super) fn axis_style_index(style: ResponseAxisStyle) -> usize {
    match style {
        ResponseAxisStyle::Distance => 0,
        ResponseAxisStyle::PerAxis => 1,
    }
}

pub(super) fn axis_style_from_index(index: usize) -> ResponseAxisStyle {
    match index {
        1 => ResponseAxisStyle::PerAxis,
        _ => ResponseAxisStyle::Distance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deadzone_source_round_trips_through_picker_index() {
        for deadzone in [
            StickDeadzone::None,
            StickDeadzone::Controller,
            StickDeadzone::Custom,
        ] {
            assert_eq!(
                deadzone_from_index(deadzone_source_index(deadzone)),
                deadzone
            );
        }
    }

    #[test]
    fn test_output_axis_round_trips_through_picker_index() {
        for axis in [
            StickOutputAxis::Both,
            StickOutputAxis::Horizontal,
            StickOutputAxis::Vertical,
        ] {
            assert_eq!(output_axis_from_index(output_axis_index(axis)), axis);
        }
        assert_eq!(
            output_from_index(output_index(StickOutput::Right)),
            StickOutput::Right
        );
        assert_eq!(
            output_from_index(output_index(StickOutput::Left)),
            StickOutput::Left
        );
    }

    #[test]
    fn test_axis_style_round_trips_through_picker_index() {
        for style in [ResponseAxisStyle::Distance, ResponseAxisStyle::PerAxis] {
            assert_eq!(axis_style_from_index(axis_style_index(style)), style);
        }
    }

}
