//! The Identity page's remaining per-field editors — the age-rating and
//! synopsis dialogs behind their rows' pen buttons. (Companies, genres
//! and families live in the unified entity dialog.) They only mutate
//! the scraper slot's draft and ask the Identity page to repaint —
//! persistence is the game settings' Save, as for every edit on these
//! pages.

use adw::prelude::*;

use super::css::*;
use super::edit_game_scraper::{edit_field, refresh_rows, ScraperSlot};
use super::helpers::{clear_children, status_row};
use super::state::SharedState;
use crate::Game;
use ira_models::ScraperClassification;

/// The age-rating editor: a two-page dialog like the entity pickers, but
/// with nothing to search — the root page lists the stored ratings with
/// remove buttons (their marks as prefixes), and the add page picks a
/// board and then one of its known values from two combo rows. Adding
/// keeps the page up so several ratings can go in.
pub(super) fn show_classifications_edit_dialog(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Age ratings"));
    dialog.set_content_width(460);
    dialog.set_content_height(320);

    // List page: every stored rating with its remove button.
    let list_toolbar = adw::ToolbarView::new();
    let list_header = adw::HeaderBar::new();
    let list_title = gtk4::Label::new(Some(&crate::tr!("Age ratings")));
    list_title.add_css_class("heading");
    list_header.set_title_widget(Some(&list_title));
    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class(CSS_FLAT);
    add_btn.set_tooltip_text(Some(&crate::tr!("Add")));
    list_header.pack_start(&add_btn);
    list_toolbar.add_top_bar(&list_header);
    let (list_scrolled, list) = super::helpers::clamped_boxed_list(460);
    list_toolbar.set_content(Some(&list_scrolled));

    // Add page: board and rating as two pickers over the hardcoded
    // value sets. There is no source to search, so no query entry.
    let boards: Vec<&ira_models::ratings::RatingBoard> =
        ira_models::ratings::BOARDS.iter().collect();
    let add_toolbar = adw::ToolbarView::new();
    let add_header = adw::HeaderBar::new();
    let add_title = gtk4::Label::new(Some(&crate::tr!("Add a rating")));
    add_title.add_css_class("heading");
    add_header.set_title_widget(Some(&add_title));
    let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    back_btn.add_css_class(CSS_FLAT);
    back_btn.set_tooltip_text(Some(&crate::tr!("Back")));
    add_header.pack_start(&back_btn);
    add_toolbar.add_top_bar(&add_header);
    let add_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    add_box.set_margin_top(12);
    add_box.set_margin_bottom(12);
    add_box.set_margin_start(12);
    add_box.set_margin_end(12);
    let pickers = gtk4::ListBox::new();
    pickers.set_selection_mode(gtk4::SelectionMode::None);
    pickers.add_css_class(CSS_BOXED_LIST);
    let board_combo = adw::ComboRow::new();
    board_combo.set_title(&crate::tr!("Board"));
    board_combo.set_model(Some(&super::helpers::string_list_from(
        &boards
            .iter()
            .map(|board| format!("{} · {}", board.name, board.region))
            .collect::<Vec<_>>(),
    )));
    let value_combo = adw::ComboRow::new();
    value_combo.set_title(&crate::tr!("Rating"));
    fill_value_combo(&value_combo, boards[0]);
    {
        let value_combo = value_combo.clone();
        let boards = boards.clone();
        board_combo.connect_selected_notify(move |combo| {
            let board = boards[combo.selected() as usize];
            fill_value_combo(&value_combo, board);
        });
    }
    pickers.append(&board_combo);
    pickers.append(&value_combo);
    add_box.append(&pickers);
    let confirm = gtk4::Button::with_label(&crate::tr!("Add"));
    confirm.add_css_class(CSS_SUGGESTED_ACTION);
    confirm.set_halign(gtk4::Align::End);
    add_box.append(&confirm);
    add_toolbar.set_content(Some(&add_box));

    let stack = gtk4::Stack::new();
    stack.set_vhomogeneous(false);
    stack.add_named(&list_toolbar, Some("list"));
    stack.add_named(&add_toolbar, Some("add"));
    dialog.set_child(Some(&stack));

    fill_classification_list(&list, state, game, win, slot);

    {
        let stack = stack.clone();
        let board_combo = board_combo.clone();
        add_btn.connect_clicked(move |_| {
            stack.set_transition_type(gtk4::StackTransitionType::SlideRight);
            stack.set_visible_child(&add_toolbar);
            let board_combo = board_combo.clone();
            glib::idle_add_local_once(move || {
                board_combo.grab_focus();
            });
        });
    }
    {
        let stack = stack.clone();
        back_btn.connect_clicked(move |_| {
            stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
            stack.set_visible_child(&list_toolbar);
        });
    }
    {
        let state = state.clone();
        let game = game.clone();
        let win = win.clone();
        let list = list.clone();
        let slot = slot.clone();
        let board_combo = board_combo.clone();
        let value_combo = value_combo.clone();
        confirm.connect_clicked(move |_| {
            let board = boards[board_combo.selected() as usize];
            let value_index = value_combo.selected() as usize;
            let Some(value) = board.values.get(value_index) else {
                return;
            };
            edit_field(&slot, |metadata| {
                upsert_classification(metadata, board.id, value);
            });
            fill_classification_list(&list, &state, &game, &win, &slot);
            refresh_rows(&state, &game, &win, &slot);
        });
    }

    let parent = win.clone();
    dialog.present(Some(&parent));
}

/// Insert or replace a board: the junction's primary key is
/// (game, kind), so a second entry for the same board is an update, not
/// a duplicate. Kind folds to uppercase, the way the sources spell
/// them; empty halves store nothing.
fn upsert_classification(metadata: &mut ira_models::ScraperMetadata, kind: &str, value: &str) {
    let kind = kind.trim().to_uppercase();
    let value = value.trim().to_string();
    if kind.is_empty() || value.is_empty() {
        return;
    }
    match metadata
        .classifications
        .iter_mut()
        .find(|c| c.kind.eq_ignore_ascii_case(&kind))
    {
        Some(existing) => existing.value = value,
        None => metadata
            .classifications
            .push(ScraperClassification { kind, value }),
    }
}

/// The rating picker's second row: the chosen board's known values.
fn fill_value_combo(value_combo: &adw::ComboRow, board: &ira_models::ratings::RatingBoard) {
    value_combo.set_model(Some(&super::helpers::string_list_from(
        &board
            .values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>(),
    )));
    value_combo.set_selected(0);
}

/// (Re)fill the editor's list page: every stored rating with a remove
/// button. Edits restage the draft and refresh the Identity page's rows.
fn fill_classification_list(
    list: &gtk4::ListBox,
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
) {
    clear_children(list);
    let metadata = slot.draft.borrow().clone();
    if metadata.classifications.is_empty() {
        list.append(&status_row(&crate::tr!("No age ratings stored")));
        return;
    }
    for classification in &metadata.classifications {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        // "STEAM_GERMANY 12" is really "USK 12" — display the board's
        // name and the canonical value spelling, raw text otherwise.
        let name = ira_models::ratings::display(&classification.kind, &classification.value)
            .unwrap_or_else(|| format!("{} {}", classification.kind, classification.value));
        row.set_title(&name);
        if let Some(texture) =
            ira_images::rating_texture(&classification.kind, &classification.value, 24)
        {
            let mark = gtk4::Image::from_paintable(Some(&texture));
            mark.set_pixel_size(24);
            row.add_prefix(&mark);
        }
        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class(CSS_FLAT);
        remove.set_valign(gtk4::Align::Center);
        {
            let state = state.clone();
            let game = game.clone();
            let win = win.clone();
            let list = list.clone();
            let slot = slot.clone();
            let kind = classification.kind.clone();
            remove.connect_clicked(move |_| {
                edit_field(&slot, |metadata| {
                    metadata
                        .classifications
                        .retain(|c| !c.kind.eq_ignore_ascii_case(&kind));
                });
                fill_classification_list(&list, &state, &game, &win, &slot);
                refresh_rows(&state, &game, &win, &slot);
            });
        }
        row.add_suffix(&remove);
        list.append(&row);
    }
}

/// The synopsis editor: the displayed language's text in a text view.
/// Applying it emptied removes the entry; the language's siblings stay
/// untouched.
pub(super) fn show_synopsis_edit_dialog(
    state: &SharedState,
    game: &Game,
    win: &adw::Window,
    slot: &ScraperSlot,
    lang: String,
    text: String,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&crate::tr!("Synopsis"));
    dialog.set_content_width(460);
    dialog.set_content_height(340);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let title = gtk4::Label::new(Some(&crate::tr!("Synopsis")));
    title.add_css_class("heading");
    header.set_title_widget(Some(&title));
    // The apply lives in the header — nothing floating over dead space.
    let apply = gtk4::Button::with_label(&crate::tr!("Apply"));
    apply.add_css_class(CSS_SUGGESTED_ACTION);
    apply.set_valign(gtk4::Align::Center);
    header.pack_end(&apply);
    toolbar.add_top_bar(&header);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    // The framed, scrolling text view fills the dialog — a real editor
    // surface instead of a text field stranded in empty space.
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let view = gtk4::TextView::new();
    view.set_wrap_mode(gtk4::WrapMode::WordChar);
    view.buffer().set_text(&text);
    scrolled.set_child(Some(&view));
    let frame = gtk4::Frame::new(None);
    frame.set_child(Some(&scrolled));
    content.append(&frame);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    {
        let slot = slot.clone();
        let dialog = dialog.clone();
        let view = view.clone();
        let (state, game, win) = (state.clone(), game.clone(), win.clone());
        apply.connect_clicked(move |_| {
            let buffer = view.buffer();
            let (start, end) = buffer.bounds();
            let text = buffer.text(&start, &end, false).trim().to_string();
            dialog.close();
            edit_field(&slot, |m| {
                if text.is_empty() {
                    m.synopses.retain(|(l, _)| *l != lang);
                } else {
                    match m.synopses.iter_mut().find(|(l, _)| *l == lang) {
                        Some(entry) => entry.1 = text.clone(),
                        None => m.synopses.push((lang.clone(), text.clone())),
                    }
                }
            });
            refresh_rows(&state, &game, &win, &slot);
        });
    }

    dialog.present(Some(win));
}

#[cfg(test)]
mod tests {
    use super::upsert_classification;
    use ira_models::ScraperMetadata;

    #[test]
    fn test_upsert_classification_inserts_then_updates_by_kind() {
        let mut metadata = ScraperMetadata::default();
        upsert_classification(&mut metadata, "PEGI", "18");
        upsert_classification(&mut metadata, "ESRB", "M");
        assert_eq!(metadata.classifications.len(), 2);
        // A second entry for the same board replaces the value — the
        // junction's primary key is (game, kind).
        upsert_classification(&mut metadata, "pegi", "16");
        assert_eq!(metadata.classifications.len(), 2);
        assert_eq!(metadata.classifications[0].kind, "PEGI");
        assert_eq!(metadata.classifications[0].value, "16");
    }

    #[test]
    fn test_upsert_classification_trims_and_needs_both_halves() {
        let mut metadata = ScraperMetadata::default();
        upsert_classification(&mut metadata, "  CERO  ", " C ");
        assert_eq!(
            metadata.classifications.first().map(|c| (c.kind.as_str(), c.value.as_str())),
            Some(("CERO", "C"))
        );
        // Empty boards and empty values store nothing.
        upsert_classification(&mut metadata, "", "18");
        upsert_classification(&mut metadata, "   ", "18");
        upsert_classification(&mut metadata, "PEGI", "");
        upsert_classification(&mut metadata, "PEGI", "   ");
        assert_eq!(metadata.classifications.len(), 1);
    }
}
