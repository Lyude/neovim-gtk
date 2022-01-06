use lazy_static::lazy_static;

use gtk::{
    self,
    prelude::*,
    subclass::prelude::*,
};
use glib;

glib::wrapper! {
    pub struct TreeView(ObjectSubclass<TreeViewImpl>)
        @extends gtk::Widget, gtk::TreeView;
}

/// A popup-aware TreeView widget for the file browser pane
impl TreeView {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create FileBrowserTreeView")
    }

    pub fn set_context_menu(&self, context_menu: &gtk::PopoverMenu) {
        self.set_property("context-menu", context_menu.to_value());
    }
}

#[derive(Debug, Default)]
pub struct TreeViewImpl {
    context_menu: glib::WeakRef<gtk::PopoverMenu>,
}

#[glib::object_subclass]
impl ObjectSubclass for TreeViewImpl {
    const NAME: &'static str = "NvimFileBrowserTreeView";
    type Type = TreeView;
    type ParentType = gtk::TreeView;
}

impl ObjectImpl for TreeViewImpl {
    fn dispose(&self, _obj: &Self::Type) {
        if let Some(context_menu) = self.context_menu.upgrade() {
            context_menu.unparent();
        }
    }

    fn properties() -> &'static [glib::ParamSpec] {
        lazy_static! {
            static ref PROPERTIES: Vec<glib::ParamSpec> = vec![
                glib::ParamSpecObject::new(
                    "context-menu",
                    "Context menu",
                    "PopoverMenu to use as the context menu",
                    gtk::PopoverMenu::static_type(),
                    glib::ParamFlags::READWRITE
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
            "context-menu" => {
                if let Some(context_menu) = self.context_menu.upgrade() {
                    context_menu.unparent();
                }

                let context_menu: gtk::PopoverMenu = value.get().unwrap();
                context_menu.set_parent(obj);
                self.context_menu.set(Some(&context_menu));
            },
            _ => unimplemented!(),
        }
    }

    fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "context-menu" => self.context_menu.upgrade().to_value(),
            _ => unimplemented!(),
        }
    }
}

impl WidgetImpl for TreeViewImpl {
    fn size_allocate(&self, widget: &Self::Type, width: i32, height: i32, baseline: i32) {
        self.parent_size_allocate(widget, width, height, baseline);
        self.context_menu.upgrade().unwrap().present();
    }
}

impl gtk::subclass::prelude::TreeViewImpl for TreeViewImpl {}
