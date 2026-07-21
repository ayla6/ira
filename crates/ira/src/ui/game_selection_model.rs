use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use glib::subclass::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use super::sidebar_item::{SidebarItem, SidebarItemKind};

mod imp {
    use super::*;

    pub struct GameSelectionModel {
        pub model: RefCell<Option<gio::ListStore>>,
        pub selected: RefCell<gtk4::Bitset>,
        pub clicked_position: Cell<u32>,
        pub select_single: Cell<bool>,
    }

    impl Default for GameSelectionModel {
        fn default() -> Self {
            Self {
                model: RefCell::new(None),
                selected: RefCell::new(gtk4::Bitset::new_empty()),
                clicked_position: Cell::new(gtk4::INVALID_LIST_POSITION),
                select_single: Cell::new(true),
            }
        }
    }

    impl GameSelectionModel {
        fn emit_selection_changed(&self, old: &gtk4::Bitset, target: &gtk4::Bitset) {
            let changes = old.copy();
            changes.union(target);
            let min = changes.minimum();
            let max = changes.maximum();
            self.obj().selection_changed(min, max - min + 1);
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GameSelectionModel {
        const NAME: &'static str = "IraGameSelectionModel";
        type Type = super::GameSelectionModel;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel, gtk4::SelectionModel);
    }

    impl ObjectImpl for GameSelectionModel {}

    impl ListModelImpl for GameSelectionModel {
        fn item_type(&self) -> glib::Type {
            SidebarItem::static_type()
        }

        fn n_items(&self) -> u32 {
            self.model
                .borrow()
                .as_ref()
                .map(|m| m.n_items())
                .unwrap_or(0)
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.model
                .borrow()
                .as_ref()
                .and_then(|m| m.item(position))
                .map(|o| o.upcast::<glib::Object>())
        }
    }

    impl SelectionModelImpl for GameSelectionModel {
        fn is_selected(&self, position: u32) -> bool {
            self.selected.borrow().contains(position)
        }

        fn selection_in_range(&self, _position: u32, _n_items: u32) -> gtk4::Bitset {
            self.selected.borrow().copy()
        }

        fn select_item(&self, position: u32, unselect_rest: bool) -> bool {
            self.clicked_position.set(position);
            self.select_single.set(unselect_rest);

            let new_selection = gtk4::Bitset::new_empty();
            new_selection.add(position);

            let old = self.selected.borrow().copy();
            {
                let selected = self.selected.borrow_mut();
                if unselect_rest {
                    if selected.equals(&new_selection) {
                        return true;
                    }
                    selected.remove_all();
                    selected.union(&new_selection);
                } else {
                    selected.union(&new_selection);
                }
            }

            self.emit_selection_changed(&old, &new_selection);

            true
        }

        fn unselect_item(&self, position: u32) -> bool {
            self.clicked_position.set(position);
            self.select_single.set(false);

            let to_remove = gtk4::Bitset::new_empty();
            to_remove.add(position);

            let old = self.selected.borrow().copy();
            {
                let selected = self.selected.borrow_mut();
                selected.subtract(&to_remove);
            }

            self.emit_selection_changed(&old, &to_remove);

            true
        }

        fn select_range(&self, position: u32, n_items: u32, unselect_rest: bool) -> bool {
            self.clicked_position.set(position);
            self.select_single.set(false);

            let model = self.model.borrow();
            let Some(store) = model.as_ref() else {
                return false;
            };

            let end = (position.saturating_add(n_items)).min(store.n_items());
            let new_selection = gtk4::Bitset::new_empty();
            for i in position..end {
                new_selection.add(i);
            }

            let old = self.selected.borrow().copy();
            {
                let selected = self.selected.borrow_mut();
                if unselect_rest {
                    if selected.equals(&new_selection) {
                        return true;
                    }
                    selected.remove_all();
                    selected.union(&new_selection);
                } else {
                    selected.union(&new_selection);
                }
            }
            drop(model);

            self.emit_selection_changed(&old, &new_selection);

            true
        }

        fn unselect_range(&self, position: u32, n_items: u32) -> bool {
            self.clicked_position.set(position);
            self.select_single.set(false);

            let to_remove = gtk4::Bitset::new_empty();
            let end = position.saturating_add(n_items);
            for i in position..end {
                to_remove.add(i);
            }

            let old = self.selected.borrow().copy();
            {
                let selected = self.selected.borrow_mut();
                selected.subtract(&to_remove);
            }

            self.emit_selection_changed(&old, &to_remove);

            true
        }

        fn unselect_all(&self) -> bool {
            let old = self.selected.borrow().copy();
            let selected = self.selected.borrow_mut();
            if selected.is_empty() {
                return true;
            }
            selected.remove_all();
            drop(selected);

            let min = old.minimum();
            let max = old.maximum();
            self.obj().selection_changed(min, max - min + 1);
            true
        }

        fn set_selection(&self, selected: &gtk4::Bitset, _mask: &gtk4::Bitset) -> bool {
            let old = self.selected.borrow().copy();
            let self_selected = self.selected.borrow_mut();
            if self_selected.equals(selected) {
                return true;
            }
            self_selected.remove_all();
            self_selected.union(selected);
            drop(self_selected);

            let changes = old.copy();
            changes.union(selected);
            let min = changes.minimum();
            let max = changes.maximum();
            self.obj().selection_changed(min, max - min + 1);
            true
        }
    }
}

glib::wrapper! {
    pub struct GameSelectionModel(ObjectSubclass<imp::GameSelectionModel>)
        @implements gio::ListModel, gtk4::SelectionModel;
}

impl GameSelectionModel {
    pub fn new(model: Option<&gio::ListStore>) -> Self {
        let obj: Self = glib::Object::new();
        if let Some(m) = model {
            obj.set_model(m);
        }
        obj
    }

    pub fn clicked_position(&self) -> u32 {
        self.imp().clicked_position.get()
    }

    pub fn select_single(&self) -> bool {
        self.imp().select_single.get()
    }

    pub fn selected_db_ids(&self) -> HashSet<String> {
        let model = self.imp().model.borrow();
        let selected = self.imp().selected.borrow();
        let mut ids = HashSet::new();
        if let Some(store) = model.as_ref() {
            for i in 0..store.n_items() {
                if selected.contains(i) {
                    if let Some(item) = store
                        .item(i)
                        .and_then(|o| o.downcast::<SidebarItem>().ok())
                    {
                        if item.kind() == SidebarItemKind::Game {
                            ids.insert(match item.variant_id() {
                                Some(vid) => format!("{}-v{}", item.db_id(), vid),
                                None => item.db_id().to_string(),
                            });
                        }
                    }
                }
            }
        }
        ids
    }

    pub fn set_model(&self, model: &gio::ListStore) {
        {
            let mut m = self.imp().model.borrow_mut();
            *m = Some(model.clone());
        }
        self.imp().selected.borrow_mut().remove_all();

        let self_weak = self.downgrade();
        model.connect_items_changed(move |_, position, removed, added| {
            let Some(self_obj) = self_weak.upgrade() else {
                return;
            };
            {
                let imp = self_obj.imp();
                imp.selected.borrow_mut().splice(position, removed, added);
            }
            self_obj.items_changed(position, removed, added);
        });

        let n = model.n_items();
        if n > 0 {
            self.items_changed(0, 0, n);
        }
    }
}
