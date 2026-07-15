use std::cell::{Cell, RefCell};

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
    }

    impl Default for GameSelectionModel {
        fn default() -> Self {
            Self {
                model: RefCell::new(None),
                selected: RefCell::new(gtk4::Bitset::new_empty()),
                clicked_position: Cell::new(gtk4::INVALID_LIST_POSITION),
            }
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

        fn select_item(&self, position: u32, _unselect_rest: bool) -> bool {
            self.clicked_position.set(position);

            let model = self.model.borrow();
            let Some(store) = model.as_ref() else {
                return false;
            };
            let Some(item) = store
                .item(position)
                .and_then(|o| o.downcast::<SidebarItem>().ok())
            else {
                return false;
            };

            let new_selection = gtk4::Bitset::new_empty();

            if item.kind() == SidebarItemKind::Game {
                let db_id = item.db_id();
                for i in 0..store.n_items() {
                    if let Some(it) = store
                        .item(i)
                        .and_then(|o| o.downcast::<SidebarItem>().ok())
                    {
                        if it.kind() == SidebarItemKind::Game && it.db_id() == db_id {
                            new_selection.add(i);
                        }
                    }
                }
            } else {
                new_selection.add(position);
            }

            let old = self.selected.borrow().copy();
            let selected = self.selected.borrow_mut();
            if selected.equals(&new_selection) {
                return true;
            }
            selected.remove_all();
            selected.union(&new_selection);
            drop(selected);
            drop(model);

            let changes = old.copy();
            changes.union(&new_selection);
            let min = changes.minimum();
            let max = changes.maximum();
            self.obj().selection_changed(min, max - min + 1);

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
