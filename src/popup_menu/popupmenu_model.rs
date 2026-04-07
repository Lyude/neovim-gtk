use once_cell::sync::Lazy;

use gio::{prelude::*, subclass::prelude::*};

use std::{cell::RefCell, convert::*, ops::Deref, rc::Rc};

use crate::nvim::PopupMenuItem;

glib::wrapper! {
    pub struct PopupMenuModel(ObjectSubclass<PopupMenuModelObject>)
        @implements gio::ListModel;
}

impl PopupMenuModel {
    pub fn new(items: &Rc<Vec<PopupMenuItem>>) -> Self {
        glib::Object::builder::<Self>()
            .property("items", glib::BoxedAnyObject::new(items.clone()))
            .build()
    }

    pub fn update_items(&self, items: &Rc<Vec<PopupMenuItem>>) {
        let imp = self.imp();
        let removed = imp.0.borrow().len().try_into().unwrap();
        let added = items.len().try_into().unwrap();
        *imp.0.borrow_mut() = items.clone();

        // Treat updates as a full reset. Completion lists are small, so a granular diff would add
        // more bookkeeping than the UI churn it would avoid.
        self.items_changed(0, removed, added);
    }
}

#[derive(Default)]
pub struct PopupMenuModelObject(RefCell<Rc<Vec<PopupMenuItem>>>);

#[glib::object_subclass]
impl ObjectSubclass for PopupMenuModelObject {
    const NAME: &'static str = "NvimPopupMenuModel";
    type Type = PopupMenuModel;
    type Interfaces = (gio::ListModel,);
}

impl ObjectImpl for PopupMenuModelObject {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![
                glib::ParamSpecObject::builder::<glib::BoxedAnyObject>("items")
                    .write_only()
                    .build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            "items" => {
                *self.0.borrow_mut() = value
                    .get::<glib::BoxedAnyObject>()
                    .unwrap()
                    .borrow::<Rc<Vec<PopupMenuItem>>>()
                    .clone()
            }
            _ => unreachable!(),
        }
    }
}

impl ListModelImpl for PopupMenuModelObject {
    fn item(&self, position: u32) -> Option<glib::Object> {
        let items = self.0.borrow();
        PopupMenuItemRef::new(&items, position as usize)
            .map(|c| glib::BoxedAnyObject::new(c).upcast())
    }

    fn n_items(&self) -> u32 {
        self.0.borrow().len().try_into().unwrap()
    }

    fn item_type(&self) -> glib::Type {
        glib::BoxedAnyObject::static_type()
    }
}

#[derive(Clone, Default)]
pub struct PopupMenuItemRef {
    array: Rc<Vec<PopupMenuItem>>,
    pos: usize,
}

impl PopupMenuItemRef {
    pub fn new(array: &Rc<Vec<PopupMenuItem>>, pos: usize) -> Option<Self> {
        array.get(pos).map(|_| Self {
            array: array.clone(),
            pos,
        })
    }
}

impl Deref for PopupMenuItemRef {
    type Target = PopupMenuItem;

    fn deref(&self) -> &Self::Target {
        // SAFETY: pos is checked at creation time
        unsafe { self.array.get_unchecked(self.pos) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup_item(word: &str) -> PopupMenuItem {
        PopupMenuItem {
            word: word.to_owned(),
            kind: String::new(),
            menu: String::new(),
            info: String::new(),
        }
    }

    #[test]
    fn update_items_reuses_model_and_emits_items_changed() {
        // This exercises gio::ListModel/glib signal delivery only, so it does not need gtk::init()
        // or a display-backed test environment.
        let model = PopupMenuModel::new(&Rc::new(vec![popup_item("one")]));
        let changes = Rc::new(RefCell::new(Vec::new()));

        model.connect_items_changed({
            let changes = changes.clone();
            move |_, position, removed, added| {
                changes.borrow_mut().push((position, removed, added));
            }
        });

        let updated_items = Rc::new(vec![popup_item("one"), popup_item("two")]);
        model.update_items(&updated_items);

        assert_eq!(model.n_items(), 2);
        assert_eq!(&*changes.borrow(), &[(0, 1, 2)]);
    }
}
