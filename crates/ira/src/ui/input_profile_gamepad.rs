use super::input_monitor_dialog::MonitorValues;
use adw::prelude::*;
use ira_input::GamepadButton;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy)]
struct MonitorColors {
    foreground: gtk4::gdk::RGBA,
    accent: gtk4::gdk::RGBA,
}

fn set_cairo_color(cr: &gtk4::cairo::Context, color: gtk4::gdk::RGBA) {
    cr.set_source_rgba(
        color.red() as f64,
        color.green() as f64,
        color.blue() as f64,
        color.alpha() as f64,
    );
}

const DESIGN_WIDTH: f64 = 640.0;
const DESIGN_HEIGHT: f64 = 410.0;
const ACTIVE_THRESHOLD: f32 = 0.015;

pub(super) fn set_draw_func(drawing: &gtk4::DrawingArea, values: Rc<RefCell<MonitorValues>>) {
    drawing.set_draw_func(move |area, cr, width, height| {
        let values = values.borrow();
        let colors = MonitorColors {
            foreground: area.color(),
            accent: adw::StyleManager::default().accent_color().to_rgba(),
        };
        let scale = (width as f64 / DESIGN_WIDTH)
            .min(height as f64 / DESIGN_HEIGHT)
            .min(1.65);
        let offset_x = (width as f64 - DESIGN_WIDTH * scale) / 2.0;
        let offset_y = (height as f64 - DESIGN_HEIGHT * scale) / 2.0;
        let _ = cr.save();
        cr.translate(offset_x, offset_y);
        cr.scale(scale, scale);
        draw_gamepad(cr, &values, colors);
        let _ = cr.restore();
    });
}

fn draw_gamepad(cr: &gtk4::cairo::Context, values: &MonitorValues, colors: MonitorColors) {
    draw_body(cr, colors);
    draw_trigger(cr, 108.0, values.output_axes[4], "LT", colors);
    draw_trigger(cr, 454.0, values.output_axes[5], "RT", colors);
    draw_bumper(
        cr,
        166.0,
        pressed(values, GamepadButton::LeftShoulder),
        "LB",
        colors,
    );
    draw_bumper(
        cr,
        382.0,
        pressed(values, GamepadButton::RightShoulder),
        "RB",
        colors,
    );

    draw_stick(
        cr,
        190.0,
        151.0,
        values.output_axes[0],
        values.output_axes[1],
        pressed(values, GamepadButton::LeftStick),
        colors,
    );
    draw_dpad(cr, 249.0, 235.0, values, colors);
    draw_stick(
        cr,
        391.0,
        235.0,
        values.output_axes[2],
        values.output_axes[3],
        pressed(values, GamepadButton::RightStick),
        colors,
    );
    draw_face_cluster(cr, values, colors);
    draw_center_controls(cr, values, colors);
    draw_gyro_telemetry(cr, values, colors);
}

fn draw_body(cr: &gtk4::cairo::Context, colors: MonitorColors) {
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.22);
    controller_path(cr, 5.0);
    let _ = cr.fill();

    set_cairo_color(cr, colors.foreground.with_alpha(0.105));
    controller_path(cr, 0.0);
    let _ = cr.fill_preserve();
    set_cairo_color(cr, colors.foreground.with_alpha(0.32));
    cr.set_line_width(1.6);
    let _ = cr.stroke();

    set_cairo_color(cr, colors.foreground.with_alpha(0.055));
    cr.move_to(173.0, 78.0);
    cr.curve_to(230.0, 91.0, 268.0, 96.0, 320.0, 96.0);
    cr.curve_to(372.0, 96.0, 410.0, 91.0, 467.0, 78.0);
    cr.curve_to(449.0, 112.0, 442.0, 157.0, 449.0, 202.0);
    cr.curve_to(422.0, 202.0, 404.0, 207.0, 390.0, 219.0);
    cr.curve_to(345.0, 230.0, 295.0, 230.0, 250.0, 219.0);
    cr.curve_to(236.0, 207.0, 218.0, 202.0, 191.0, 202.0);
    cr.curve_to(198.0, 157.0, 191.0, 112.0, 173.0, 78.0);
    cr.close_path();
    let _ = cr.fill();

    set_cairo_color(cr, colors.foreground.with_alpha(0.12));
    cr.set_line_width(1.0);
    cr.move_to(232.0, 292.0);
    cr.curve_to(273.0, 306.0, 367.0, 306.0, 408.0, 292.0);
    let _ = cr.stroke();
}

fn controller_path(cr: &gtk4::cairo::Context, offset_y: f64) {
    cr.move_to(174.0, 66.0 + offset_y);
    cr.curve_to(
        139.0,
        62.0 + offset_y,
        103.0,
        77.0 + offset_y,
        82.0,
        108.0 + offset_y,
    );
    cr.curve_to(
        61.0,
        139.0 + offset_y,
        58.0,
        194.0 + offset_y,
        70.0,
        239.0 + offset_y,
    );
    cr.line_to(89.0, 302.0 + offset_y);
    cr.curve_to(
        96.0,
        328.0 + offset_y,
        124.0,
        340.0 + offset_y,
        145.0,
        326.0 + offset_y,
    );
    cr.curve_to(
        162.0,
        315.0 + offset_y,
        171.0,
        288.0 + offset_y,
        184.0,
        263.0 + offset_y,
    );
    cr.curve_to(
        198.0,
        236.0 + offset_y,
        214.0,
        222.0 + offset_y,
        238.0,
        220.0 + offset_y,
    );
    cr.curve_to(
        264.0,
        218.0 + offset_y,
        284.0,
        230.0 + offset_y,
        320.0,
        230.0 + offset_y,
    );
    cr.curve_to(
        356.0,
        230.0 + offset_y,
        376.0,
        218.0 + offset_y,
        402.0,
        220.0 + offset_y,
    );
    cr.curve_to(
        426.0,
        222.0 + offset_y,
        442.0,
        236.0 + offset_y,
        456.0,
        263.0 + offset_y,
    );
    cr.curve_to(
        469.0,
        288.0 + offset_y,
        478.0,
        315.0 + offset_y,
        495.0,
        326.0 + offset_y,
    );
    cr.curve_to(
        516.0,
        340.0 + offset_y,
        544.0,
        328.0 + offset_y,
        551.0,
        302.0 + offset_y,
    );
    cr.line_to(570.0, 239.0 + offset_y);
    cr.curve_to(
        582.0,
        194.0 + offset_y,
        579.0,
        139.0 + offset_y,
        558.0,
        108.0 + offset_y,
    );
    cr.curve_to(
        537.0,
        77.0 + offset_y,
        501.0,
        62.0 + offset_y,
        466.0,
        66.0 + offset_y,
    );
    cr.curve_to(
        423.0,
        72.0 + offset_y,
        381.0,
        84.0 + offset_y,
        320.0,
        84.0 + offset_y,
    );
    cr.curve_to(
        259.0,
        84.0 + offset_y,
        217.0,
        72.0 + offset_y,
        174.0,
        66.0 + offset_y,
    );
    cr.close_path();
}

fn draw_trigger(cr: &gtk4::cairo::Context, x: f64, value: f32, label: &str, colors: MonitorColors) {
    let value = value.clamp(0.0, 1.0) as f64;
    set_cairo_color(cr, colors.foreground.with_alpha(0.1));
    rounded_rect(cr, x, 53.0, 78.0, 18.0, 9.0);
    let _ = cr.fill_preserve();
    set_cairo_color(cr, colors.foreground.with_alpha(0.28));
    let _ = cr.stroke();
    if value > ACTIVE_THRESHOLD as f64 {
        set_cairo_color(cr, colors.accent);
        rounded_rect(cr, x + 2.0, 55.0, 74.0 * value, 14.0, 7.0);
        let _ = cr.fill();
    }
    draw_text(cr, x + 39.0, 62.0, label, 10.0, colors.foreground, 0.5);
}

fn draw_bumper(
    cr: &gtk4::cairo::Context,
    x: f64,
    active: bool,
    label: &str,
    colors: MonitorColors,
) {
    set_cairo_color(cr, control_color(active, colors));
    rounded_rect(cr, x, 71.0, 92.0, 22.0, 11.0);
    let _ = cr.fill_preserve();
    set_cairo_color(cr, colors.foreground.with_alpha(0.32));
    let _ = cr.stroke();
    draw_text(cr, x + 46.0, 82.0, label, 10.0, colors.foreground, 0.5);
}

fn draw_stick(
    cr: &gtk4::cairo::Context,
    cx: f64,
    cy: f64,
    x_value: f32,
    y_value: f32,
    clicked: bool,
    colors: MonitorColors,
) {
    set_cairo_color(cr, colors.foreground.with_alpha(0.075));
    cr.arc(cx, cy, 43.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill_preserve();
    set_cairo_color(cr, colors.foreground.with_alpha(0.25));
    cr.set_line_width(1.3);
    let _ = cr.stroke();
    set_cairo_color(cr, colors.foreground.with_alpha(0.13));
    cr.move_to(cx - 25.0, cy);
    cr.line_to(cx + 25.0, cy);
    cr.move_to(cx, cy - 25.0);
    cr.line_to(cx, cy + 25.0);
    let _ = cr.stroke();

    let x = x_value.clamp(-1.0, 1.0);
    let y = y_value.clamp(-1.0, 1.0);
    let active = x.abs() > ACTIVE_THRESHOLD || y.abs() > ACTIVE_THRESHOLD;
    set_cairo_color(
        cr,
        if active || clicked {
            colors.accent
        } else {
            colors.foreground.with_alpha(0.4)
        },
    );
    cr.arc(
        cx + x as f64 * 20.0,
        cy - y as f64 * 20.0,
        15.0,
        0.0,
        std::f64::consts::TAU,
    );
    let _ = cr.fill();
    if clicked {
        set_cairo_color(cr, colors.foreground.with_alpha(0.8));
        cr.set_line_width(2.0);
        cr.arc(cx, cy, 34.0, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
    }
}

fn draw_dpad(
    cr: &gtk4::cairo::Context,
    cx: f64,
    cy: f64,
    values: &MonitorValues,
    colors: MonitorColors,
) {
    set_cairo_color(cr, colors.foreground.with_alpha(0.14));
    rounded_rect(cr, cx - 12.0, cy - 40.0, 24.0, 80.0, 6.0);
    let _ = cr.fill();
    rounded_rect(cr, cx - 40.0, cy - 12.0, 80.0, 24.0, 6.0);
    let _ = cr.fill();
    draw_dpad_arm(
        cr,
        cx - 12.0,
        cy - 40.0,
        24.0,
        30.0,
        pressed(values, GamepadButton::DpadUp),
        colors,
    );
    draw_dpad_arm(
        cr,
        cx - 12.0,
        cy + 10.0,
        24.0,
        30.0,
        pressed(values, GamepadButton::DpadDown),
        colors,
    );
    draw_dpad_arm(
        cr,
        cx - 40.0,
        cy - 12.0,
        30.0,
        24.0,
        pressed(values, GamepadButton::DpadLeft),
        colors,
    );
    draw_dpad_arm(
        cr,
        cx + 10.0,
        cy - 12.0,
        30.0,
        24.0,
        pressed(values, GamepadButton::DpadRight),
        colors,
    );
    set_cairo_color(cr, colors.foreground.with_alpha(0.22));
    cr.arc(cx, cy, 5.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

fn draw_dpad_arm(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    active: bool,
    colors: MonitorColors,
) {
    if active {
        set_cairo_color(cr, colors.accent);
        rounded_rect(cr, x, y, width, height, 6.0);
        let _ = cr.fill();
    }
}

fn draw_face_cluster(cr: &gtk4::cairo::Context, values: &MonitorValues, colors: MonitorColors) {
    draw_face_button(
        cr,
        474.0,
        112.0,
        "Y",
        pressed(values, GamepadButton::Y),
        colors,
    );
    draw_face_button(
        cr,
        443.0,
        143.0,
        "X",
        pressed(values, GamepadButton::X),
        colors,
    );
    draw_face_button(
        cr,
        505.0,
        143.0,
        "B",
        pressed(values, GamepadButton::B),
        colors,
    );
    draw_face_button(
        cr,
        474.0,
        174.0,
        "A",
        pressed(values, GamepadButton::A),
        colors,
    );
}

fn draw_face_button(
    cr: &gtk4::cairo::Context,
    cx: f64,
    cy: f64,
    label: &str,
    active: bool,
    colors: MonitorColors,
) {
    set_cairo_color(cr, control_color(active, colors));
    cr.arc(cx, cy, 17.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill_preserve();
    set_cairo_color(cr, colors.foreground.with_alpha(0.32));
    cr.set_line_width(1.2);
    let _ = cr.stroke();
    draw_text(cr, cx, cy, label, 11.0, colors.foreground, 0.5);
}

fn draw_center_controls(cr: &gtk4::cairo::Context, values: &MonitorValues, colors: MonitorColors) {
    draw_center_button(
        cr,
        283.0,
        "VIEW",
        pressed(values, GamepadButton::Back),
        colors,
    );
    draw_center_button(
        cr,
        357.0,
        "MENU",
        pressed(values, GamepadButton::Start),
        colors,
    );
    let guide = pressed(values, GamepadButton::Guide);
    set_cairo_color(cr, control_color(guide, colors));
    cr.arc(320.0, 139.0, 15.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill_preserve();
    set_cairo_color(cr, colors.foreground.with_alpha(0.35));
    let _ = cr.stroke();
    set_cairo_color(cr, colors.foreground.with_alpha(0.65));
    cr.arc(320.0, 139.0, 4.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

fn draw_center_button(
    cr: &gtk4::cairo::Context,
    cx: f64,
    label: &str,
    active: bool,
    colors: MonitorColors,
) {
    set_cairo_color(cr, control_color(active, colors));
    rounded_rect(cr, cx - 14.0, 163.0, 28.0, 12.0, 6.0);
    let _ = cr.fill_preserve();
    set_cairo_color(cr, colors.foreground.with_alpha(0.3));
    let _ = cr.stroke();
    draw_text(
        cr,
        cx,
        187.0,
        label,
        7.5,
        colors.foreground.with_alpha(0.55),
        0.5,
    );
}

fn draw_gyro_telemetry(cr: &gtk4::cairo::Context, values: &MonitorValues, colors: MonitorColors) {
    draw_text(
        cr,
        320.0,
        365.0,
        "GYRO INPUT",
        8.0,
        colors.foreground.with_alpha(0.45),
        0.5,
    );
    for (index, value) in values.gyro.iter().enumerate() {
        let value = (value / 10.0).clamp(-1.0, 1.0) as f64;
        let x = 235.0 + index as f64 * 61.0;
        set_cairo_color(cr, colors.foreground.with_alpha(0.1));
        rounded_rect(cr, x, 380.0, 50.0, 7.0, 3.5);
        let _ = cr.fill();
        if value.abs() > 0.01 {
            set_cairo_color(cr, colors.accent);
            let start = if value >= 0.0 {
                x + 25.0
            } else {
                x + 25.0 + value * 25.0
            };
            rounded_rect(cr, start, 380.0, value.abs() * 25.0, 7.0, 3.5);
            let _ = cr.fill();
        }
        draw_text(
            cr,
            x + 25.0,
            399.0,
            ["PITCH", "YAW", "ROLL"][index],
            7.0,
            colors.foreground.with_alpha(0.42),
            0.5,
        );
    }
}

fn pressed(values: &MonitorValues, button: GamepadButton) -> bool {
    values.output_buttons.contains(&button)
}

fn control_color(active: bool, colors: MonitorColors) -> gtk4::gdk::RGBA {
    if active {
        colors.accent
    } else {
        colors.foreground.with_alpha(0.11)
    }
}

fn draw_text(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    text: &str,
    size: f64,
    color: gtk4::gdk::RGBA,
    anchor: f64,
) {
    set_cairo_color(cr, color);
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Bold,
    );
    cr.set_font_size(size);
    let Ok(extents) = cr.text_extents(text) else {
        return;
    };
    cr.move_to(
        x - extents.width() * anchor,
        y - extents.y_bearing() - extents.height() / 2.0,
    );
    let _ = cr.show_text(text);
}

fn rounded_rect(cr: &gtk4::cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let half = std::f64::consts::PI / 2.0;
    cr.move_to(x + radius, y);
    cr.arc(x + width - radius, y + radius, radius, -half, 0.0);
    cr.arc(x + width - radius, y + height - radius, radius, 0.0, half);
    cr.arc(
        x + radius,
        y + height - radius,
        radius,
        half,
        std::f64::consts::PI,
    );
    cr.arc(x + radius, y + radius, radius, std::f64::consts::PI, -half);
    cr.close_path();
}
