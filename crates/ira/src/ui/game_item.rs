use crate::Game;
use gtk4::subclass::prelude::ObjectSubclassIsExt;
use std::rc::Rc;

mod imp {
    use crate::Game;
    use glib::subclass::prelude::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    pub struct GameItem {
        pub game: RefCell<Option<Rc<Game>>>,
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
        obj.imp().game.replace(Some(Rc::new(game.clone())));
        obj
    }

    pub fn game(&self) -> Option<Rc<Game>> {
        self.imp().game.borrow().clone()
    }
}
