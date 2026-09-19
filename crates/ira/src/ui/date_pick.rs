//! The date editor: a menu button whose popover pairs a typed entry with
//! a calendar. The two stay in step — typing moves the calendar, picking
//! a day fills the entry — so either way of answering lands the same
//! date. Used by the release-date row and the auto-group rule editor.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// The Apply/Clear callbacks, shared between the buttons and the owner.
type ActionSlot = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

pub(crate) struct DatePick {
    button: gtk4::MenuButton,
    calendar: gtk4::Calendar,
    typed: Rc<RefCell<Option<String>>>,
    on_apply: ActionSlot,
    on_clear: ActionSlot,
}

impl DatePick {
    /// `initial` is the stored shape — ISO date or bare year — shown in
    /// the entry and preset on the calendar.
    pub fn new(initial: &str) -> Self {
        let calendar = gtk4::Calendar::new();
        let timestamp = ira_db::scraper_release_timestamp(initial);
        if timestamp > 0 {
            if let Some(date) = chrono::DateTime::from_timestamp(timestamp, 0) {
                use chrono::Datelike;
                if let Ok(preset) = glib::DateTime::from_utc(
                    date.year(),
                    date.month() as i32,
                    date.day() as i32,
                    0,
                    0,
                    0.0,
                ) {
                    calendar.select_day(&preset);
                }
            }
        }

        // The normalized typed date while it is still being typed: `Some`
        // only while the entry holds a shape the parser accepts.
        let typed: Rc<RefCell<Option<String>>> = Default::default();
        let entry = gtk4::Entry::new();
        entry.set_placeholder_text(Some(&crate::tr!(
            "1998-08-24, 24.08.1998, Aug 24 1998, 1998"
        )));
        entry.set_tooltip_text(Some(&crate::tr!(
            "Type the date — the calendar follows along"
        )));
        entry.set_text(initial);

        {
            let typed = typed.clone();
            let calendar = calendar.clone();
            // Every keystroke re-parses: a good shape selects its day in
            // the calendar (the feedback that says the date was
            // understood), a bad one flags the entry red.
            entry.connect_changed(move |entry| {
                let text = entry.text().trim().to_string();
                match super::edit_game_scraper::parse_typed_date(&text) {
                    Some(parsed) => {
                        entry.remove_css_class("error");
                        *typed.borrow_mut() = Some(parsed.stored);
                        if let Ok(preset) = glib::DateTime::from_utc(
                            parsed.year,
                            parsed.month as i32,
                            parsed.day as i32,
                            0,
                            0,
                            0.0,
                        ) {
                            calendar.select_day(&preset);
                        }
                    }
                    None if text.is_empty() => {
                        entry.remove_css_class("error");
                        *typed.borrow_mut() = None;
                    }
                    None => {
                        entry.add_css_class("error");
                        *typed.borrow_mut() = None;
                    }
                }
            });
        }
        {
            // Picking a day fills the entry — unless the entry is what
            // moved the calendar, whose echoed text would fight the
            // typist.
            let entry = entry.clone();
            let typed = typed.clone();
            calendar.connect_day_selected(move |calendar| {
                let iso = calendar.date().format("%Y-%m-%d").map(|s| s.to_string());
                let Ok(iso) = iso else { return };
                if super::edit_game_scraper::parse_typed_date(&entry.text())
                    .is_some_and(|parsed| parsed.stored == iso)
                {
                    return;
                }
                *typed.borrow_mut() = Some(iso.clone());
                entry.set_text(&iso);
                entry.remove_css_class("error");
            });
        }

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(8);
        content.set_margin_end(8);
        content.append(&entry);
        content.append(&calendar);
        let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let clear_btn = gtk4::Button::with_label(&crate::tr!("Clear"));
        clear_btn.add_css_class(super::css::CSS_FLAT);
        clear_btn.set_halign(gtk4::Align::Start);
        clear_btn.set_hexpand(true);
        let apply_btn = gtk4::Button::with_label(&crate::tr!("Apply"));
        apply_btn.add_css_class(super::css::CSS_SUGGESTED_ACTION);
        apply_btn.set_halign(gtk4::Align::End);
        buttons.append(&clear_btn);
        buttons.append(&apply_btn);
        content.append(&buttons);
        let popover = gtk4::Popover::new();
        popover.set_child(Some(&content));

        let button = gtk4::MenuButton::new();
        button.set_icon_name("x-office-calendar-symbolic");
        button.add_css_class(super::css::CSS_FLAT);
        button.set_valign(gtk4::Align::Center);
        button.set_popover(Some(&popover));

        let on_apply: ActionSlot = Rc::new(RefCell::new(None));
        let on_clear: ActionSlot = Rc::new(RefCell::new(None));
        {
            let on_apply = on_apply.clone();
            apply_btn.connect_clicked(move |_| {
                if let Some(f) = on_apply.borrow().as_ref() {
                    f();
                }
            });
        }
        {
            // Clear empties the typed date (the calendar keeps its page)
            // and then tells the owner what clearing means.
            let on_clear = on_clear.clone();
            let entry = entry.clone();
            let typed = typed.clone();
            clear_btn.connect_clicked(move |_| {
                entry.set_text("");
                *typed.borrow_mut() = None;
                if let Some(f) = on_clear.borrow().as_ref() {
                    f();
                }
            });
        }

        Self {
            button,
            calendar,
            typed,
            on_apply,
            on_clear,
        }
    }

    pub fn button(&self) -> &gtk4::MenuButton {
        &self.button
    }

    pub fn popover(&self) -> gtk4::Popover {
        self.button
            .popover()
            .and_downcast::<gtk4::Popover>()
            .expect("DatePick popover")
    }

    /// The Apply button's action. The popover stays open unless the
    /// callback closes it.
    pub fn on_apply(&self, f: impl Fn() + 'static) {
        *self.on_apply.borrow_mut() = Some(Rc::new(f));
    }

    /// The Clear button's action — the entry is already emptied by then.
    pub fn on_clear(&self, f: impl Fn() + 'static) {
        *self.on_clear.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_tooltip(&self, text: &str) {
        self.button.set_tooltip_text(Some(text));
    }

    /// The valid typed date, if any.
    pub fn typed(&self) -> Option<String> {
        self.typed.borrow().clone()
    }

    /// The typed date, else the day the calendar currently shows.
    pub fn date(&self) -> Option<String> {
        self.typed().or_else(|| {
            self.calendar
                .date()
                .format("%Y-%m-%d")
                .ok()
                .map(|s| s.to_string())
        })
    }
}
