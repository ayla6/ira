use std::cell::RefCell;
use std::rc::Rc;

type VariantImagesSectionResult = (Option<Rc<RefCell<String>>>, Option<gtk4::Adjustment>, Option<Rc<std::cell::Cell<bool>>>);
use adw::prelude::*;
use ira_models::{AssetType, GameVariant};
use super::helpers;
use super::settings_dialog;
use super::state::SharedState;
use super::css::*;

pub(super) struct VarW {
    pub(super) id: Option<i64>,
    pub(super) name: adw::EntryRow,
    pub(super) exe: adw::EntryRow,
    pub(super) wd: adw::EntryRow,
    pub(super) args: adw::EntryRow,
    pub(super) pre_launch: adw::EntryRow,
    pub(super) custom_images: adw::SwitchRow,
    pub(super) show_as_entry: adw::SwitchRow,
    pub(super) count_playtime: adw::SwitchRow,
    pub(super) logo_position: Rc<RefCell<String>>,
    pub(super) logo_size: gtk4::Adjustment,
    pub(super) logo_modified: Rc<std::cell::Cell<bool>>,
    pub(super) group: adw::PreferencesGroup,
}

fn move_variant_card(
    group: &adw::PreferencesGroup,
    delta: isize,
    var_widgets: &Rc<RefCell<Vec<VarW>>>,
    container: &gtk4::Box,
) {
    let widgets = var_widgets.borrow();
    let idx = widgets.iter().position(|w| w.group == *group);
    if let Some(i) = idx {
        let new_i = i as isize + delta;
        if new_i < 0 || new_i >= widgets.len() as isize { return; }
        let new_i = new_i as usize;
        drop(widgets);
        let mut widgets = var_widgets.borrow_mut();
        widgets.swap(i, new_i);
        let sibling = if new_i == 0 { None } else { Some(widgets[new_i - 1].group.clone()) };
        drop(widgets);
        match &sibling {
            None => container.reorder_child_after(group, None::<&gtk4::Widget>),
            Some(sib) => container.reorder_child_after(group, Some(sib)),
        }
    }
}

fn build_variant_name_entry_buttons(
    name_entry: &adw::EntryRow,
    group: &adw::PreferencesGroup,
    var_widgets: &Rc<RefCell<Vec<VarW>>>,
    container: &gtk4::Box,
    main_win: adw::Window,
) {
    let up_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    up_btn.set_tooltip_text(Some("Move up"));
    up_btn.set_valign(gtk4::Align::Center);
    let group_up = group.clone();
    let vw_up = var_widgets.clone();
    let container_up = container.clone();
    up_btn.connect_clicked(move |_| {
        move_variant_card(&group_up, -1, &vw_up, &container_up);
    });
    name_entry.add_suffix(&up_btn);

    let down_btn = gtk4::Button::from_icon_name("go-down-symbolic");
    down_btn.set_tooltip_text(Some("Move down"));
    down_btn.set_valign(gtk4::Align::Center);
    let group_down = group.clone();
    let vw_down = var_widgets.clone();
    let container_down = container.clone();
    down_btn.connect_clicked(move |_| {
        move_variant_card(&group_down, 1, &vw_down, &container_down);
    });
    name_entry.add_suffix(&down_btn);

    let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    del_btn.set_tooltip_text(Some("Delete variant"));
    del_btn.set_valign(gtk4::Align::Center);
    del_btn.add_css_class(CSS_ERROR);
    let container_c = container.clone();
    let group_c = group.clone();
    let name_entry_c = name_entry.clone();
    del_btn.connect_clicked(move |_| {
        let variant_name = name_entry_c.text().to_string();
        let dialog = adw::AlertDialog::new(
            Some("Delete variant?"),
            Some(&format!("\"{}\" will be removed. Save to apply changes.", variant_name)),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let container_cc = container_c.clone();
        let group_cc = group_c.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                container_cc.remove(&group_cc);
            }
        });
        dialog.present(Some(&main_win));
    });
    name_entry.add_suffix(&del_btn);
}

fn build_variant_images_and_logo_section(
    group: &adw::PreferencesGroup,
    images_expander: &adw::ExpanderRow,
    custom_images_row: &adw::SwitchRow,
    v: &GameVariant,
    state: &SharedState,
    db_id: i64,
    win: &adw::Window,
) -> VariantImagesSectionResult {
    let mut logo_position_cell: Option<Rc<RefCell<String>>> = None;
    let mut logo_size_adj_cell: Option<gtk4::Adjustment> = None;
    let mut logo_modified_cell: Option<Rc<std::cell::Cell<bool>>> = None;

    {
        let images_expander_c = images_expander.clone();
        custom_images_row.connect_notify_local(Some("active"), move |row, _| {
            images_expander_c.set_enable_expansion(row.is_active());
        });
    }

    if v.id > 0 {
        let state_c = state.clone();
        let db_id_c = db_id;
        let variant_id = v.id;
        let parent_win = win.clone();
        let save_dir = state.borrow().save_dir.clone();
        let db = state.borrow().db.clone();
        if let Ok(Some(entry)) = ira_db::find_by_db_id(&db, db_id_c) {
            let image_dir = ira_parser::entry_data_dir(&save_dir, &entry);
            let var_dir = image_dir.join(format!("variant-{}", variant_id));
            let _ = std::fs::create_dir_all(&var_dir);
            for at in AssetType::all() {
                let row = super::image_manager::build_image_section_for_dir(
                    super::image_manager::VariantImageSectionParams {
                        target_dir: &var_dir,
                        label: at.display_name(),
                        file_base: at.file_base(),
                        asset_type: at.as_str(),
                        dimensions: at.sgdb_dimensions(),
                        max_h: match at {
                            AssetType::Icon => 48,
                            AssetType::Hero => 64,
                            AssetType::Grid => 64,
                            AssetType::Header => 48,
                            AssetType::Logo => 64,
                        },
                        state: &state_c, entry: &entry, parent_win: &parent_win,
                    },
                );
                images_expander.add_row(&row);
            }

            let mut var_game = crate::Game::default();
            ira_parser::populate_image_paths(&var_dir, &mut var_game);
            if var_game.hero_image_path.is_empty() || var_game.logo_path.is_empty() {
                if let Some(base) = state.borrow().games.iter()
                    .find(|g| g.db_id == db_id && g.variant_id.is_none())
                {
                    if var_game.hero_image_path.is_empty() {
                        var_game.hero_image_path = base.hero_image_path.clone();
                    }
                    if var_game.logo_path.is_empty() {
                        var_game.logo_path = base.logo_path.clone();
                    }
                }
            }
            var_game.logo_position = v.logo_position.clone();
            var_game.logo_size = v.logo_size;
            if let Some((logo_ui, logo_pos, logo_size_adj, logo_mod)) = super::game_logo::build_game_logo_page(&var_game, true, None) {
                let logo_expander = adw::ExpanderRow::new();
                logo_expander.set_title("Logo position");
                logo_expander.set_subtitle(if v.logo_position.is_empty() { "Inherited from base game" } else { "" });
                logo_expander.set_enable_expansion(true);
                logo_expander.set_visible(custom_images_row.is_active());
                let logo_row = adw::ActionRow::new();
                logo_row.set_activatable(false);
                logo_row.set_child(Some(&logo_ui));
                logo_expander.add_row(&logo_row);
                group.add(&logo_expander);
                {
                    let logo_exp_c = logo_expander.clone();
                    custom_images_row.connect_notify_local(Some("active"), move |row, _| {
                        logo_exp_c.set_visible(row.is_active());
                    });
                }
                logo_position_cell = Some(logo_pos);
                logo_size_adj_cell = Some(logo_size_adj);
                logo_modified_cell = Some(logo_mod);
            }
        }
    }

    (logo_position_cell, logo_size_adj_cell, logo_modified_cell)
}

fn build_variant_card(
    v: GameVariant,
    var_widgets: &Rc<RefCell<Vec<VarW>>>,
    container: &gtk4::Box,
    state: &SharedState,
    db_id: i64,
    win: &adw::Window,
) -> VarW {
    let group = adw::PreferencesGroup::new();

    let name_entry = adw::EntryRow::new();
    name_entry.set_title("Variant name");
    name_entry.set_text(&v.name);

    let main_win = win.clone();
    build_variant_name_entry_buttons(&name_entry, &group, var_widgets, container, main_win);

    group.add(&name_entry);

    let exe_entry = adw::EntryRow::new();
    exe_entry.set_title("Executable");
    exe_entry.set_text(&v.exe);
    let browse = helpers::make_browse_button(
        None,
        "Select variant executable",
        false,
        Some(("Executable", &["application/x-executable"])),
        helpers::entry_path_closure(&exe_entry),
        {
            let entry = exe_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    exe_entry.add_suffix(&browse);
    group.add(&exe_entry);

    let args_entry = adw::EntryRow::new();
    args_entry.set_title("Arguments");
    args_entry.set_text(&v.args);
    group.add(&args_entry);

    let wd_entry = adw::EntryRow::new();
    wd_entry.set_title("Working directory");
    wd_entry.set_text(&v.working_dir);
    let wd_browse = helpers::make_browse_button(
        None,
        "Select working directory",
        true,
        None,
        helpers::entry_path_closure(&wd_entry),
        {
            let entry = wd_entry.clone();
            move |path| entry.set_text(&path.to_string_lossy())
        },
    );
    wd_entry.add_suffix(&wd_browse);
    group.add(&wd_entry);

    let pre_launch_entry = adw::EntryRow::new();
    pre_launch_entry.set_title("Run before game");
    pre_launch_entry.set_text(&v.pre_launch);
    pre_launch_entry.set_tooltip_text(Some("Shell command to run before launching. If it fails, the game will not launch."));
    group.add(&pre_launch_entry);

    let custom_images_row = adw::SwitchRow::new();
    custom_images_row.set_title("Custom images");
    custom_images_row.set_subtitle("Use custom hero and logo for this variant");
    custom_images_row.set_active(v.custom_images || v.show_as_entry);
    group.add(&custom_images_row);

    let show_as_entry_row = adw::SwitchRow::new();
    show_as_entry_row.set_title("Show as separate entry");
    show_as_entry_row.set_subtitle("Appears in the grid as its own game entry");
    show_as_entry_row.set_active(v.show_as_entry);
    group.add(&show_as_entry_row);

    {
        let custom_images_c = custom_images_row.clone();
        show_as_entry_row.connect_notify_local(Some("active"), move |row, _| {
            if row.is_active() {
                custom_images_c.set_active(true);
                custom_images_c.set_sensitive(false);
            } else {
                custom_images_c.set_sensitive(true);
            }
        });
    }

    if v.show_as_entry {
        custom_images_row.set_sensitive(false);
    }

    let count_playtime_row = adw::SwitchRow::new();
    count_playtime_row.set_title("Count playtime");
    count_playtime_row.set_subtitle("Track playtime for this variant (disable for modding tools)");
    count_playtime_row.set_active(v.count_playtime);
    group.add(&count_playtime_row);

    let images_expander = adw::ExpanderRow::new();
    images_expander.set_title("Manage images");
    images_expander.set_enable_expansion(custom_images_row.is_active() && v.id > 0);
    if v.id == 0 {
        images_expander.set_subtitle("Save the variant first");
    }
    group.add(&images_expander);

    let (logo_position_cell, logo_size_adj_cell, logo_modified_cell) = build_variant_images_and_logo_section(
        &group, &images_expander, &custom_images_row, &v, state, db_id, win,
    );

    container.append(&group);

    VarW {
        id: if v.id > 0 { Some(v.id) } else { None },
        name: name_entry,
        exe: exe_entry,
        wd: wd_entry,
        args: args_entry,
        pre_launch: pre_launch_entry,
        custom_images: custom_images_row,
        show_as_entry: show_as_entry_row,
        count_playtime: count_playtime_row,
        logo_position: logo_position_cell.unwrap_or_else(|| Rc::new(RefCell::new(v.logo_position.clone()))),
        logo_size: logo_size_adj_cell.unwrap_or_else(|| gtk4::Adjustment::new(v.logo_size as f64, 5.0, 100.0, 1.0, 5.0, 0.0)),
        logo_modified: logo_modified_cell.unwrap_or_else(|| Rc::new(std::cell::Cell::new(false))),
        group,
    }
}

pub(super) fn build_variants_page(
    state: &SharedState,
    db_id: i64,
    game_kind: ira_models::GameKind,
    _has_config: bool,
    sidebar: &gtk4::ListBox,
    stack: &gtk4::Stack,
    win: &adw::Window,
) -> Rc<RefCell<Vec<VarW>>> {
    let variants: Vec<GameVariant> = ira_db::get_variants(&state.borrow().db, db_id).unwrap_or_default();
    let variant_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.set_margin_start(12);
    header.set_margin_end(12);
    let title = gtk4::Label::new(Some("Variants"));
    title.set_halign(gtk4::Align::Start);
    title.set_hexpand(true);
    title.add_css_class(CSS_HEADING);
    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some("Add variant"));
    add_btn.set_valign(gtk4::Align::Center);
    add_btn.add_css_class(CSS_FLAT);
    header.append(&title);
    header.append(&add_btn);
    variant_page.append(&header);

    let variant_container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    variant_container.set_margin_start(12);
    variant_container.set_margin_end(12);
    variant_page.append(&variant_container);

    let var_widgets: Rc<RefCell<Vec<VarW>>> = Rc::new(RefCell::new(Vec::new()));

    for v in &variants {
        let vw = build_variant_card(v.clone(), &var_widgets, &variant_container, state, db_id, win);
        var_widgets.borrow_mut().push(vw);
    }

    {
        let var_widgets_c = var_widgets.clone();
        let container_c = variant_container.clone();
        let state_c = state.clone();
        let win_c = win.clone();
        add_btn.connect_clicked(move |_| {
            let vw = build_variant_card(
                GameVariant { game_id: db_id, ..Default::default() },
                &var_widgets_c, &container_c, &state_c, db_id, &win_c,
            );
            var_widgets_c.borrow_mut().push(vw);
        });
    }

    let variant_scroll = gtk4::ScrolledWindow::new();
    variant_scroll.set_child(Some(&variant_page));
    variant_scroll.set_vexpand(true);
    variant_scroll.set_hexpand(true);
    if game_kind != ira_models::GameKind::Steam && game_kind != ira_models::GameKind::Ps4 && game_kind != ira_models::GameKind::Ps3 && game_kind != ira_models::GameKind::Retro {
        sidebar.append(&settings_dialog::sidebar_separator());
        sidebar.append(&settings_dialog::settings_sidebar_row("application-x-executable-symbolic", "Variants", "variants"));
        stack.add_named(&variant_scroll, Some("variants"));
    }

    var_widgets
}

pub(super) fn save_variants(db: &ira_db::DbConn, db_id: i64, var_widgets: &Rc<RefCell<Vec<VarW>>>) {
    let widgets = var_widgets.borrow();

    for vw in widgets.iter() {
        if vw.group.parent().is_none() {
            if let Some(id) = vw.id {
                if let Err(e) = ira_db::delete_variant(db, id) {
                    eprintln!("Failed to delete variant {}: {}", id, e);
                }
            }
        }
    }

    let mut ordered_ids: Vec<i64> = Vec::new();
    for vw in widgets.iter() {
        if vw.group.parent().is_none() { continue; }
        let name = vw.name.text().to_string();
        if name.is_empty() { continue; }

        let variant = ira_models::GameVariant {
            id: vw.id.unwrap_or(0),
            game_id: db_id,
            name,
            exe: vw.exe.text().to_string(),
            working_dir: vw.wd.text().to_string(),
            args: vw.args.text().to_string(),
            env_vars: Vec::new(),
            sort_order: 0,
            pre_launch: vw.pre_launch.text().to_string(),
            custom_images: vw.custom_images.is_active(),
            show_as_entry: vw.show_as_entry.is_active(),
            count_playtime: vw.count_playtime.is_active(),
            logo_position: if vw.logo_modified.get() { vw.logo_position.borrow().clone() } else { String::new() },
            logo_size: if vw.logo_modified.get() { vw.logo_size.value() as i32 } else { 0 },
            ..Default::default()
        };

        if let Some(id) = vw.id {
            if let Err(e) = ira_db::update_variant(db, &variant) {
                eprintln!("Failed to update variant {}: {}", id, e);
            }
            ordered_ids.push(id);
        } else {
            match ira_db::add_variant(db, &variant) {
                Ok(new_id) => ordered_ids.push(new_id),
                Err(e) => eprintln!("Failed to add variant: {}", e),
            }
        }
    }

    if ordered_ids.len() > 1 {
        if let Err(e) = ira_db::reorder_variants(db, &ordered_ids) {
            eprintln!("Failed to reorder variants: {}", e);
        }
    }
}
