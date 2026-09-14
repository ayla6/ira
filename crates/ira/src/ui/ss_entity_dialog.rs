//! Entity picking for the metadata editor. Companies come from the local
//! `scraper_companies` cache — every game match feeds it, and the source
//! offers no company list to fetch — while genres come from the whole
//! genre table fetched once and cached on disk. A company missing locally
//! can only arrive by matching more games, so the dialog says so instead
//! of pretending to search online.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::helpers::{clear_children, poll_channel, status_row};
use super::steam_search_dialog::{build_search_dialog, match_result_row, SearchDialogWidgets};
use super::state::SharedState;
use ira_api::ScraperCreds;
use ira_models::ScraperEntity;

/// Which metadata field the picker feeds; decides the source and the copy.
#[derive(Clone, Copy)]
pub(super) enum EntityKind {
    Developer,
    Publisher,
    Genre,
}

impl EntityKind {
    fn title(self) -> String {
        match self {
            Self::Developer => crate::tr!("Pick a developer"),
            Self::Publisher => crate::tr!("Pick a publisher"),
            Self::Genre => crate::tr!("Pick a genre"),
        }
    }

    fn empty_text(self) -> String {
        match self {
            Self::Developer | Self::Publisher => {
                crate::tr!("Companies appear here as games get matched")
            }
            Self::Genre => crate::tr!("No results found"),
        }
    }
}

/// Search one entity source and hand the pick back through `on_pick`.
pub(super) fn show_entity_picker(
    state: &SharedState,
    kind: EntityKind,
    parent: &impl IsA<gtk4::Widget>,
    on_pick: Rc<dyn Fn(ScraperEntity)>,
) {
    let SearchDialogWidgets {
        dialog,
        entry,
        search_btn,
        list,
    } = build_search_dialog(&kind.title(), 460, 440, 460, "", Some(&crate::tr!("Search…")));

    let entries: Rc<RefCell<Vec<ScraperEntity>>> = Default::default();

    // Companies are a local query, so they are ready when the dialog
    // opens; genres may need the one-time table fetch off-thread.
    match kind {
        EntityKind::Developer | EntityKind::Publisher => match companies(state, "") {
            Ok(rows) => *entries.borrow_mut() = rows,
            Err(e) => eprintln!("Entity picker: {e}"),
        },
        EntityKind::Genre => {
            let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<ScraperEntity>, String>>();
            let creds = {
                let s = state.borrow();
                ScraperCreds::from_account(
                    s.cfg.screenscraper_id.clone(),
                    s.cfg.screenscraper_password.clone(),
                )
            };
            let steam = state.borrow().steam.clone();
            std::thread::spawn(move || {
                let outcome = steam.screenscraper_genres(&creds).map(|rows| {
                    rows.into_iter()
                        .map(|g| ScraperEntity {
                            id: g.id,
                            name: g.name,
                        })
                        .collect()
                });
                let _ = tx.send(outcome);
            });
            let entries = entries.clone();
            let list_c = list.clone();
            poll_channel(rx, move |outcome| match outcome {
                Ok(rows) => *entries.borrow_mut() = rows,
                Err(e) => {
                    clear_children(&list_c);
                    list_c.append(&status_row(&e));
                }
            });
        }
    }

    let repaint: Rc<dyn Fn(&str)> = Rc::new({
        let entries = entries.clone();
        let list = list.clone();
        let dialog = dialog.clone();
        let on_pick = on_pick.clone();
        move |filter: &str| {
            clear_children(&list);
            let needle = filter.to_lowercase();
            let rows: Vec<ScraperEntity> = entries
                .borrow()
                .iter()
                .filter(|e| needle.is_empty() || e.name.to_lowercase().contains(&needle))
                .take(60)
                .cloned()
                .collect();
            if rows.is_empty() {
                list.append(&status_row(&kind.empty_text()));
                return;
            }
            for entity in rows {
                let on_pick = on_pick.clone();
                let dlg = dialog.clone();
                let picked = entity.clone();
                let row = match_result_row(&entity.name, &format!("id {}", entity.id), move || {
                    on_pick(picked.clone());
                    dlg.close();
                });
                list.append(&row);
            }
        }
    });

    let entry_c = entry.clone();
    let repaint_entry = repaint.clone();
    entry.connect_changed(move |_| repaint_entry(entry_c.text().trim()));
    let entry_c = entry.clone();
    let repaint_btn = repaint.clone();
    search_btn.connect_clicked(move |_| repaint_btn(entry_c.text().trim()));

    dialog.present(Some(parent));
    repaint("");
}

fn companies(state: &SharedState, filter: &str) -> Result<Vec<ScraperEntity>, String> {
    ira_db::scraper_companies_search(&state.borrow().db, filter)
}
