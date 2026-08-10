use super::widget::{Event, Rect};

fn major_axis_distance(direction: Event, current: Rect, candidate: Rect) -> Option<f32> {
    match direction {
        Event::NavDown => {
            if candidate.y >= current.y + current.height {
                Some(candidate.y - (current.y + current.height))
            } else {
                None
            }
        }
        Event::NavUp => {
            if candidate.y + candidate.height <= current.y {
                Some(current.y - (candidate.y + candidate.height))
            } else {
                None
            }
        }
        Event::NavRight => {
            if candidate.x >= current.x + current.width {
                Some(candidate.x - (current.x + current.width))
            } else {
                None
            }
        }
        Event::NavLeft => {
            if candidate.x + candidate.width <= current.x {
                Some(current.x - (candidate.x + candidate.width))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn minor_axis_distance(current: Rect, candidate: Rect) -> f32 {
    if current.x + current.width > candidate.x && candidate.x + candidate.width > current.x {
        0.0
    } else if candidate.x >= current.x + current.width {
        candidate.x - (current.x + current.width)
    } else {
        current.x - (candidate.x + candidate.width)
    }
}

fn minor_axis_distance_v(current: Rect, candidate: Rect) -> f32 {
    if current.y + current.height > candidate.y && candidate.y + candidate.height > current.y {
        0.0
    } else if candidate.y >= current.y + current.height {
        candidate.y - (current.y + current.height)
    } else {
        current.y - (candidate.y + candidate.height)
    }
}

pub fn navigate(direction: Event, current: Rect, candidates: &[Rect]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;

    for (i, c) in candidates.iter().enumerate() {
        let major = match major_axis_distance(direction, current, *c) {
            Some(d) => d,
            None => continue,
        };

        let minor = match direction {
            Event::NavDown | Event::NavUp => minor_axis_distance(current, *c),
            Event::NavLeft | Event::NavRight => minor_axis_distance_v(current, *c),
            _ => return None,
        };

        let score = major + minor * 2.0;

        if best.is_none_or(|(_, s)| score < s) {
            best = Some((i, score));
        }
    }

    best.map(|(i, _)| i)
}
