use crate::Game;
use std::cell::RefCell;
use gtk4::subclass::prelude::ObjectSubclassIsExt;

mod imp {
    use crate::Game;
    use std::cell::RefCell;
    use glib::subclass::prelude::*;

    #[derive(Default)]
    pub struct GameItem {
        pub game: RefCell<Option<Game>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GameItem {
        const NAME: &'static str = "GameGridItem";
        type Type = super::GameItem;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for GameItem {}
}

glib::wrapper! {
    pub struct GameItem(ObjectSubclass<imp::GameItem>);
}

impl GameItem {
    pub fn new(game: &Game) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().game.replace(Some(game.clone()));
        obj
    }

    pub fn game(&self) -> Option<Game> {
        self.imp().game.borrow().clone()
    }
}
