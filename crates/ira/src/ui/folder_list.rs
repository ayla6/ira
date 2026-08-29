//! Reusable editor for a user-configured list of folders.
//!
//! Used for the PC games folders and ROM roots in Settings. Each folder is
//! an `ActionRow` in the group's boxed list showing its free space plus
//! change/remove buttons, and a `ButtonRow` appends new folders. The live
//! `Vec<String>` is shared with the caller so the Save handler can read the
//! final list back.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::gio;
use gtk4::glib;

use super::css::CSS_FLAT;
use super::disk_space;
use super::helpers::{esc, set_initial_folder};

/// Owns one editable folder list inside a `PreferencesGroup`.
/// Cheap to clone; every row callback captures the same shared state.
#[derive(Clone)]
pub(super) struct FolderListWidgets {
    /// The preferences group this list renders into, for the page to append.
    pub(super) group: adw::PreferencesGroup,
    rows: Rc<RefCell<Vec<adw::ActionRow>>>,
    add_row: adw::ButtonRow,
    pub(super) folders: Rc<RefCell<Vec<String>>>,
    select_label: String,
}

impl FolderListWidgets {
    /// Snapshot of the current folder list, primary first.
    pub(super) fn get(&self) -> Vec<String> {
        self.folders.borrow().clone()
    }

    fn rebuild(&self) {
        for row in self.rows.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
        for index in 0..self.folders.borrow().len() {
            let row = self.build_folder_row(index);
            self.group.add(&row);
            self.rows.borrow_mut().push(row);
        }
        // Re-append so the add row stays below the folder rows after every
        // rebuild; group.add() always inserts at the end of the list.
        self.group.remove(&self.add_row);
        self.group.add(&self.add_row);
    }

    fn build_folder_row(&self, index: usize) -> adw::ActionRow {
        let folder = self.folders.borrow()[index].clone();
        let path = PathBuf::from(&folder);

        let row = adw::ActionRow::new();
        row.set_title(&esc(&folder));
        if let Some(free) = disk_space::available_bytes(&path) {
            row.set_subtitle(&crate::tr!("{} free").replacen(
                "{}",
                &disk_space::format_size(free),
                1,
            ));
        }

        row.add_suffix(&self.build_change_button(index));
        row.add_suffix(&self.build_remove_button(index));
        row
    }

    fn build_change_button(&self, index: usize) -> gtk4::Button {
        let btn = gtk4::Button::from_icon_name("document-edit-symbolic");
        btn.add_css_class(CSS_FLAT);
        btn.set_valign(gtk4::Align::Center);
        btn.set_tooltip_text(Some(&crate::tr!("Change folder")));

        let ctx = self.clone();
        btn.connect_clicked(move |_| {
            ctx.pick_folder(
                ctx.folders.borrow().get(index).cloned(),
                move |selected_widgets, path| {
                    let mut folders = selected_widgets.folders.borrow_mut();
                    if index < folders.len() {
                        folders[index] = path.to_string_lossy().into_owned();
                    }
                    drop(folders);
                    selected_widgets.rebuild();
                },
            );
        });
        btn
    }

    fn build_remove_button(&self, index: usize) -> gtk4::Button {
        let btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        btn.add_css_class(CSS_FLAT);
        btn.set_valign(gtk4::Align::Center);
        btn.set_tooltip_text(Some(&crate::tr!("Remove folder")));

        let ctx = self.clone();
        btn.connect_clicked(move |_| {
            let mut folders = ctx.folders.borrow_mut();
            if index < folders.len() {
                folders.remove(index);
            }
            drop(folders);
            ctx.rebuild();
        });
        btn
    }

    /// Open the folder picker; `on_pick` receives the shared state and the
    /// chosen path, and must mutate and re-render.
    fn pick_folder(
        &self,
        initial: Option<String>,
        on_pick: impl Fn(&FolderListWidgets, &Path) + 'static,
    ) {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title(&self.select_label);
        if let Some(p) = initial {
            set_initial_folder(&dialog, &p);
        }
        let ctx = self.clone();
        let cb = move |result: Result<gio::File, glib::Error>| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    on_pick(&ctx, &path);
                }
            }
        };
        // File dialogs must be transient over a real window; the group
        // resolves to the settings dialog at click time.
        match super::helpers::hosting_window(&self.group) {
            Some(window) => dialog.select_folder(Some(&window), None::<&gio::Cancellable>, cb),
            None => eprintln!("Cannot open folder picker without a parent window"),
        }
    }
}

pub(super) fn build_folder_list(
    title: &str,
    add_label: &str,
    select_label: &str,
    initial: Vec<String>,
) -> FolderListWidgets {
    let group = adw::PreferencesGroup::new();
    group.set_title(title);

    let add_row = adw::ButtonRow::new();
    add_row.set_title(add_label);
    add_row.set_start_icon_name(Some("list-add-symbolic"));
    group.add(&add_row);

    let widgets = FolderListWidgets {
        group: group.clone(),
        rows: Rc::new(RefCell::new(Vec::new())),
        add_row,
        folders: Rc::new(RefCell::new(initial)),
        select_label: select_label.to_string(),
    };

    widgets.rebuild();

    let clicked_ctx = widgets.clone();
    widgets.add_row.connect_activated(move |_| {
        clicked_ctx.pick_folder(None, |picked_widgets, path| {
            picked_widgets
                .folders
                .borrow_mut()
                .push(path.to_string_lossy().into_owned());
            picked_widgets.rebuild();
        });
    });

    widgets
}
