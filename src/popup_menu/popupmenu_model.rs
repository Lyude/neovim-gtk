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
        glib::Object::new::<Self>(&[("items", &glib::BoxedAnyObject::new(items.clone()))])
    }
}

#[derive(Default)]
pub struct PopupMenuModelObject(RefCell<Rc<Vec<PopupMenuItem>>>);

#[glib::object_subclass]
impl ObjectSubclass for PopupMenuModelObject {
    const NAME: &'static str = "NvimPopupMenuModel";
    type Type = PopupMenuModel;
    type ParentType = glib::Object;
    type Interfaces = (gio::ListModel,);
}

impl ObjectImpl for PopupMenuModelObject {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![glib::ParamSpecObject::new(
                "items",
                "PopupMenu items",
                "A reference to the list of completion items",
                glib::BoxedAnyObject::static_type(),
                glib::ParamFlags::WRITABLE,
            )]
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
