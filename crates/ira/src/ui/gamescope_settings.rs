use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::wine_config_helpers::make_revert_btn;

pub(super) const UPSCALE_VALUES: [&str; 5] = ["linear", "fsr", "nis", "integer", "nearest"];

pub(super) struct GamescopeDefaults {
    pub flags: String,
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub upscaling: String,
}

pub(super) struct GamescopeOverride {
    pub flags: String,
    pub w: Option<u32>,
    pub h: Option<u32>,
    pub fps: Option<u32>,
    pub upscaling: Option<String>,
}

pub(super) struct GamescopeWidgets {
    pub flags: adw::EntryRow,
    pub w: gtk4::SpinButton,
    pub h: gtk4::SpinButton,
    pub fps: gtk4::SpinButton,
    pub upscaling: adw::ComboRow,
    pub w_state: Option<Rc<RefCell<Option<u32>>>>,
    pub h_state: Option<Rc<RefCell<Option<u32>>>>,
    pub fps_state: Option<Rc<RefCell<Option<u32>>>>,
    pub upscaling_state: Option<Rc<RefCell<Option<String>>>>,
}

fn make_spin_row(
    title: &str, subtitle: &str,
    default_val: u32, override_val: Option<u32>,
    min: f64, max: f64,
) -> (adw::ActionRow, gtk4::SpinButton, Option<Rc<RefCell<Option<u32>>>>) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    let val = override_val.unwrap_or(default_val);
    let adj = gtk4::Adjustment::new(val as f64, min, max, 1.0, 10.0, 0.0);
    let spin = gtk4::SpinButton::new(Some(&adj), 1.0, 0);
    spin.set_valign(gtk4::Align::Center);

    if let Some(ov) = override_val {
        let state: Rc<RefCell<Option<u32>>> = Rc::new(RefCell::new(Some(ov)));
        let revert_btn = make_revert_btn();
        let rev = Rc::new(RefCell::new(false));
        {
            let state_c = state.clone();
            let btn_c = revert_btn.clone();
            let rev_c = rev.clone();
            spin.connect_value_changed(move |s| {
                if *rev_c.borrow() { return; }
                *state_c.borrow_mut() = Some(s.value() as u32);
                btn_c.set_visible(true);
            });
        }
        {
            let state_c = state.clone();
            let btn_c = revert_btn.clone();
            let spin_c = spin.clone();
            let rev_c = rev.clone();
            let d = default_val as f64;
            revert_btn.connect_clicked(move |_| {
                *rev_c.borrow_mut() = true;
                spin_c.set_value(d);
                *rev_c.borrow_mut() = false;
                *state_c.borrow_mut() = None;
                btn_c.set_visible(false);
            });
        }
        row.add_suffix(&revert_btn);
        row.add_suffix(&spin);
        (row, spin, Some(state))
    } else {
        row.add_suffix(&spin);
        (row, spin, None)
    }
}

fn make_upscaling_row(
    default_upscaling: &str,
    override_val: Option<&str>,
) -> (adw::ComboRow, Option<Rc<RefCell<Option<String>>>>) {
    let model = gtk4::StringList::new(&["Linear", "FSR", "NIS", "Integer", "Nearest"]);
    let row = adw::ComboRow::new();
    row.set_title("Upscaling method");
    row.set_model(Some(&model));

    let selected = if let Some(ov) = override_val {
        UPSCALE_VALUES.iter().position(|&v| v == ov).unwrap_or(0)
    } else {
        UPSCALE_VALUES.iter().position(|&v| v == default_upscaling).unwrap_or(0)
    } as u32;
    row.set_selected(selected);

    if let Some(ov) = override_val {
        let state: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(Some(ov.to_string())));
        let revert_btn = make_revert_btn();
        let rev = Rc::new(RefCell::new(false));
        row.add_suffix(&revert_btn);
        {
            let state_c = state.clone();
            let btn_c = revert_btn.clone();
            let rev_c = rev.clone();
            row.connect_selected_item_notify(move |r| {
                if *rev_c.borrow() { return; }
                let idx = r.selected() as usize;
                let v = UPSCALE_VALUES.get(idx).copied().unwrap_or("linear");
                *state_c.borrow_mut() = Some(v.to_string());
                btn_c.set_visible(true);
            });
        }
        {
            let state_c = state.clone();
            let btn_c = revert_btn.clone();
            let rev_c = rev.clone();
            let default_idx = UPSCALE_VALUES.iter().position(|&v| v == default_upscaling).unwrap_or(0) as u32;
            let row_c = row.clone();
            revert_btn.connect_clicked(move |_| {
                *rev_c.borrow_mut() = true;
                row_c.set_selected(default_idx);
                *rev_c.borrow_mut() = false;
                *state_c.borrow_mut() = None;
                btn_c.set_visible(false);
            });
        }
        (row, Some(state))
    } else {
        (row, None)
    }
}

pub(super) fn add_gamescope_rows(
    expander: &adw::ExpanderRow,
    defaults: &GamescopeDefaults,
    override_vals: Option<&GamescopeOverride>,
) -> GamescopeWidgets {
    let flags_text = override_vals.map(|o| o.flags.as_str()).unwrap_or(&defaults.flags);
    let flags = adw::EntryRow::new();
    flags.set_title("Gamescope flags");
    flags.set_text(flags_text);
    expander.add_row(&flags);

    let (w_row, w_spin, w_state) = make_spin_row(
        "Resolution width", "0 = auto",
        defaults.w, override_vals.and_then(|o| o.w),
        0.0, 16384.0,
    );
    expander.add_row(&w_row);

    let (h_row, h_spin, h_state) = make_spin_row(
        "Resolution height", "0 = auto",
        defaults.h, override_vals.and_then(|o| o.h),
        0.0, 16384.0,
    );
    expander.add_row(&h_row);

    let (fps_row, fps_spin, fps_state) = make_spin_row(
        "FPS limit", "0 = no limit",
        defaults.fps, override_vals.and_then(|o| o.fps),
        0.0, 360.0,
    );
    expander.add_row(&fps_row);

    let (upscaling_row, upscaling_state) = make_upscaling_row(
        &defaults.upscaling,
        override_vals.and_then(|o| o.upscaling.as_deref()),
    );
    expander.add_row(&upscaling_row);

    GamescopeWidgets {
        flags,
        w: w_spin,
        h: h_spin,
        fps: fps_spin,
        upscaling: upscaling_row,
        w_state,
        h_state,
        fps_state,
        upscaling_state,
    }
}
