use adw::prelude::*;
use std::{cell::Cell, rc::Rc};

pub(super) fn build_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&crate::tr!("Desktop integration"));
    group.set_description(Some(&crate::tr!("Search installed games from GNOME without individual shortcuts. Enable or remove the integration here.")));
    let row = adw::SwitchRow::new();
    row.set_title(&crate::tr!("Desktop search"));
    row.set_subtitle(&crate::tr!("Use your existing Ira launcher for GNOME search. Your app manager owns the launcher. Disable to remove search integration while keeping game shortcuts."));
    row.set_sensitive(false);
    let updating = Rc::new(Cell::new(true));
    row.connect_active_notify({
        let updating = updating.clone();
        move |row| {
            if !updating.get() {
                toggle(row, updating.clone());
            }
        }
    });
    let weak = row.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(crate::desktop::integration::status_for_user)
            .await
            .map_err(|_| "Desktop integration worker panicked".to_string())
            .and_then(|result| result);
        if let Some(row) = weak.upgrade() {
            match result {
                Ok(status) => apply_status(&row, status),
                Err(error) => {
                    eprintln!("Read desktop integration state: {error}");
                    super::helpers::toast_on_root(&row, &error);
                }
            }
            updating.set(false);
            row.set_sensitive(true);
        }
    });
    group.add(&row);
    group
}

fn toggle(row: &adw::SwitchRow, updating: Rc<Cell<bool>>) {
    let enabled = row.is_active();
    row.set_sensitive(false);
    let weak = row.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(move || {
            if enabled {
                crate::desktop::integration::install_for_user()?;
            } else {
                crate::desktop::integration::uninstall_for_user()?;
            }
            crate::desktop::integration::status_for_user()
        })
        .await
        .map_err(|_| "Desktop integration worker panicked".to_string())
        .and_then(|result| result);
        if let Some(row) = weak.upgrade() {
            updating.set(true);
            match result {
                Ok(status) => apply_status(&row, status),
                Err(error) => {
                    eprintln!("Change desktop integration: {error}");
                    row.set_active(!enabled);
                    super::helpers::toast_on_root(&row, &error);
                }
            }
            updating.set(false);
            row.set_sensitive(true);
        }
    });
}

fn apply_status(row: &adw::SwitchRow, status: crate::desktop::integration::IntegrationStatus) {
    row.set_active(status.enabled);
    row.set_subtitle(&if status.requires_login {
        crate::tr!("Installed, but this GNOME session cannot discover the provider. Desktop search is not active. Your app manager's launcher is unchanged.")
    } else {
        crate::tr!("Use your existing Ira launcher for GNOME search. Your app manager owns the launcher. Disable to remove search integration while keeping game shortcuts.")
    });
}
