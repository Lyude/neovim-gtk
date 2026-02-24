use std::convert::*;

use once_cell::sync::*;

use glib::SignalHandlerId;
use gtk::{self, graphene::*, prelude::*, subclass::prelude::*};

glib::wrapper! {
    pub struct PopupMenuPopover(ObjectSubclass<PopupMenuPopoverObject>)
        @extends gtk::Popover, gtk::Native, gtk::Widget,
        @implements gtk::Buildable, gtk::ConstraintTarget, gtk::Accessible, gtk::ShortcutManager;
}

impl PopupMenuPopover {
    pub fn new() -> Self {
        glib::Object::new::<Self>()
    }

    pub fn connect_bounds_changed<F>(&self, cb: F) -> SignalHandlerId
    where
        F: Fn(&Self, f32, f32, i32, i32) + 'static,
    {
        self.connect_local("bounds-change", true, move |values| {
            cb(
                values[0].get().as_ref().unwrap(),
                values[1].get().unwrap(),
                values[2].get().unwrap(),
                values[3].get().unwrap(),
                values[4].get().unwrap(),
            );
            None
        })
    }
}

#[derive(Default)]
pub struct PopupMenuPopoverObject(());

#[glib::object_subclass]
impl ObjectSubclass for PopupMenuPopoverObject {
    const NAME: &'static str = "NvimPopupMenuPopover";
    type Type = PopupMenuPopover;
    type ParentType = gtk::Popover;
}

impl ObjectImpl for PopupMenuPopoverObject {
    fn signals() -> &'static [glib::subclass::Signal] {
        static SIGNALS: Lazy<Vec<glib::subclass::Signal>> = Lazy::new(|| {
            vec![
                glib::subclass::Signal::builder("bounds-change")
                    .param_types([
                        glib::Type::F32, // x
                        glib::Type::F32, // y
                        glib::Type::I32, // w
                        glib::Type::I32, // h
                    ])
                    .build(),
            ]
        });

        SIGNALS.as_ref()
    }
}
impl PopoverImpl for PopupMenuPopoverObject {}

impl WidgetImpl for PopupMenuPopoverObject {
    fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
        self.parent_size_allocate(width, height, baseline);

        let obj = self.obj();
        let gdk_popup = obj.surface().unwrap().downcast::<gdk::Popup>().unwrap();

        let viewport = obj.parent().unwrap();
        let root = obj.root().unwrap();

        let viewport_bounds = viewport.compute_bounds(&root).unwrap();

        obj.emit_by_name::<()>(
            "bounds-change",
            &[
                &(gdk_popup.position_x() as f32 - viewport_bounds.x()),
                &(gdk_popup.position_y() as f32 - viewport_bounds.y()),
                &width,
                &height,
            ],
        );
    }
}
