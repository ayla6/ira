use std::cell::{Cell, RefCell};
use glib::subclass::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SidebarItemKind {
    #[default]
    AllGames,
    CollectionHeader,
    Game,
    UncategorizedHeader,
}

mod imp {
    use super::*;

    pub struct SidebarItem {
        pub kind: RefCell<SidebarItemKind>,
        pub db_id: Cell<i64>,
        pub name: RefCell<String>,
        pub icon_path: RefCell<String>,
        pub group_id: Cell<i64>,
        pub count: Cell<usize>,
        pub collapsed: Cell<bool>,
        pub hidden: Cell<bool>,
        pub playing: Cell<bool>,
    }

    impl Default for SidebarItem {
        fn default() -> Self {
            Self {
                kind: RefCell::new(SidebarItemKind::default()),
                db_id: Cell::new(0),
                name: RefCell::new(String::new()),
                icon_path: RefCell::new(String::new()),
                group_id: Cell::new(0),
                count: Cell::new(0),
                collapsed: Cell::new(false),
                hidden: Cell::new(false),
                playing: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarItem {
        const NAME: &'static str = "IraSidebarItem";
        type Type = super::SidebarItem;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for SidebarItem {}
}

glib::wrapper! {
    pub struct SidebarItem(ObjectSubclass<imp::SidebarItem>);
}

impl SidebarItem {
    pub fn new_all_games() -> Self {
        let obj: Self = glib::Object::new();
        *obj.imp().kind.borrow_mut() = SidebarItemKind::AllGames;
        obj
    }

    pub fn new_collection_header(group_id: i64, name: &str, count: usize, collapsed: bool) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        *imp.kind.borrow_mut() = SidebarItemKind::CollectionHeader;
        imp.group_id.set(group_id);
        *imp.name.borrow_mut() = name.to_string();
        imp.count.set(count);
        imp.collapsed.set(collapsed);
        obj
    }

    pub fn new_game(db_id: i64, name: &str, icon_path: &str, hidden: bool, playing: bool) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        *imp.kind.borrow_mut() = SidebarItemKind::Game;
        imp.db_id.set(db_id);
        *imp.name.borrow_mut() = name.to_string();
        *imp.icon_path.borrow_mut() = icon_path.to_string();
        imp.hidden.set(hidden);
        imp.playing.set(playing);
        obj
    }

    pub fn new_uncategorized_header(count: usize, collapsed: bool) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        *imp.kind.borrow_mut() = SidebarItemKind::UncategorizedHeader;
        imp.group_id.set(0);
        imp.count.set(count);
        imp.collapsed.set(collapsed);
        obj
    }

    pub fn kind(&self) -> SidebarItemKind {
        *self.imp().kind.borrow()
    }

    pub fn db_id(&self) -> i64 {
        self.imp().db_id.get()
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn icon_path(&self) -> String {
        self.imp().icon_path.borrow().clone()
    }

    pub fn group_id(&self) -> i64 {
        self.imp().group_id.get()
    }

    pub fn count(&self) -> usize {
        self.imp().count.get()
    }

    pub fn collapsed(&self) -> bool {
        self.imp().collapsed.get()
    }

    pub fn hidden(&self) -> bool {
        self.imp().hidden.get()
    }

    pub fn playing(&self) -> bool {
        self.imp().playing.get()
    }
}
