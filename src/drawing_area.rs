use lazy_static::lazy_static;

use gtk::{
    self,
    prelude::*,
    subclass::prelude::*,
};
use glib;

glib::wrapper! {
    pub struct DrawingArea(ObjectSubclass<DrawingAreaImpl>)
        @extends gtk::Widget, gtk::DrawingArea;
}

/** Our temporary DrawingArea placeholder, which is aware of both the completion and context menu
  * popovers */
impl DrawingArea {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create NvimDrawingArea")
    }

    pub fn set_popover_menu(&self, popover_menu: &gtk::PopoverMenu) {
        self.set_property("popover-menu", popover_menu.to_value());
    }

    pub fn set_completion_popover(&self, completion_popover: &gtk::Popover) {
        self.set_property("completion-popover", completion_popover.to_value());
    }
}

#[derive(Debug, Default)]
pub struct DrawingAreaImpl {
    popover_menu: glib::WeakRef<gtk::PopoverMenu>,
    completion_popover: glib::WeakRef<gtk::Popover>,
}

#[glib::object_subclass]
impl ObjectSubclass for DrawingAreaImpl {
    const NAME: &'static str = "NvimDrawingArea";
    type Type = DrawingArea;
    type ParentType = gtk::DrawingArea;
}

impl ObjectImpl for DrawingAreaImpl {
    fn dispose(&self, _obj: &Self::Type) {
        if let Some(popover_menu) = self.popover_menu.upgrade() {
            popover_menu.unparent();
        }
        if let Some(completion_popover) = self.completion_popover.upgrade() {
            completion_popover.unparent();
        }
    }

    fn properties() -> &'static [glib::ParamSpec] {
        lazy_static! {
            static ref PROPERTIES: Vec<glib::ParamSpec> = vec![
                glib::ParamSpecObject::new(
                    "popover-menu",
                    "Popover menu",
                    "PopoverMenu to use as the context menu",
                    gtk::PopoverMenu::static_type(),
                    glib::ParamFlags::READWRITE
                ),
                glib::ParamSpecObject::new(
                    "completion-popover",
                    "Completion popover",
                    "Popover to use for completion results from neovim",
                    gtk::Popover::static_type(),
                    glib::ParamFlags::READWRITE,
                ),
            ];
        }

        PROPERTIES.as_ref()
    }

    fn set_property(
        &self,
        obj: &Self::Type,
        _id: usize,
        value: &glib::Value,
        pspec: &glib::ParamSpec
    ) {
        match pspec.name() {
            "popover-menu" => {
                if let Some(popover_menu) = self.popover_menu.upgrade() {
                    popover_menu.unparent();
                }
                let popover_menu: gtk::PopoverMenu = value.get().unwrap();

                popover_menu.set_parent(obj);
                self.popover_menu.set(Some(&popover_menu));
            },
            "completion-popover" => {
                if let Some(popover) = self.completion_popover.upgrade() {
                    popover.unparent();
                }
                let popover: gtk::Popover = value.get().unwrap();

                popover.set_parent(obj);
                self.completion_popover.set(Some(&popover));
            }
            _ => unimplemented!(),
        }
    }

    fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "popover-menu" => self.popover_menu.upgrade().to_value(),
            "completion-popover" => self.completion_popover.upgrade().to_value(),
            _ => unimplemented!(),
        }
    }
}

impl WidgetImpl for DrawingAreaImpl {
    fn size_allocate(&self, widget: &Self::Type, width: i32, height: i32, baseline: i32) {
        self.parent_size_allocate(widget, width, height, baseline);
        self.popover_menu.upgrade().unwrap().present();
        self.completion_popover.upgrade().unwrap().present();
    }
}
impl gtk::subclass::prelude::DrawingAreaImpl for DrawingAreaImpl {}
