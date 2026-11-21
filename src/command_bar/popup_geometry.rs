use egui::{pos2, vec2, Pos2, Rect, Vec2};

pub(super) const POPUP_MARGIN: f32 = 4.0;

fn finite_axis_bounds(first: f32, second: f32) -> (f64, f64) {
    match (first.is_finite(), second.is_finite()) {
        (true, true) => {
            let first = f64::from(first);
            let second = f64::from(second);
            (first.min(second), first.max(second))
        }
        (true, false) => {
            let value = f64::from(first);
            (value, value)
        }
        (false, true) => {
            let value = f64::from(second);
            (value, value)
        }
        (false, false) => (0.0, 0.0),
    }
}

fn constrained_axis_size(start: f32, end: f32, desired: f32) -> f32 {
    let (start, end) = finite_axis_bounds(start, end);
    let extent = (end - start).max(0.0);
    let margin = f64::from(POPUP_MARGIN).min(extent / 2.0);
    (f64::from(desired).min((extent - margin * 2.0).max(0.0))) as f32
}

pub(super) fn constrained_size(screen: Rect, desired: Vec2) -> Vec2 {
    vec2(
        constrained_axis_size(screen.left(), screen.right(), desired.x),
        constrained_axis_size(screen.top(), screen.bottom(), desired.y),
    )
}

fn axis_position(
    anchor: f32,
    size: f32,
    screen_start: f32,
    screen_end: f32,
    before_anchor: bool,
) -> f32 {
    let (screen_start, screen_end) = finite_axis_bounds(screen_start, screen_end);
    let extent = (screen_end - screen_start).max(0.0);
    let margin = f64::from(POPUP_MARGIN).min(extent / 2.0);
    let size = if size.is_finite() {
        f64::from(size.max(0.0)).min((extent - margin * 2.0).max(0.0))
    } else {
        0.0
    };
    let minimum = screen_start + margin;
    let maximum = (screen_end - margin - size).max(minimum);
    let fallback = maximum;
    let anchor = if anchor.is_finite() {
        f64::from(anchor)
    } else {
        fallback + if before_anchor { size } else { 0.0 }
    };
    let desired = if before_anchor {
        anchor - size - f64::from(POPUP_MARGIN)
    } else {
        anchor - size
    };
    desired.clamp(minimum, maximum) as f32
}

pub(super) fn above_button(button: Rect, size: Vec2, screen: Rect) -> Pos2 {
    pos2(
        axis_position(button.right(), size.x, screen.left(), screen.right(), false),
        axis_position(button.top(), size.y, screen.top(), screen.bottom(), true),
    )
}
