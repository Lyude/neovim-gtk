use once_cell::sync::Lazy;

use gtk::{prelude::*, subclass::prelude::*};

glib::wrapper! {
    pub struct TreeView(ObjectSubclass<TreeViewObject>)
        @extends gtk::Widget, gtk::TreeView;
}

/// A popup-aware TreeView widget for the file browser pane
impl TreeView {
    pub fn new() -> Self {
        glib::Object::new::<Self>()
    }

    pub fn set_context_menu(&self, context_menu: &gtk::PopoverMenu) {
        self.set_property("context-menu", context_menu);
    }
}

#[derive(Default)]
pub struct TreeViewObject {
    context_menu: glib::WeakRef<gtk::PopoverMenu>,
}

#[glib::object_subclass]
impl ObjectSubclass for TreeViewObject {
    const NAME: &'static str = "NvimFileBrowserTreeView";
    type Type = TreeView;
    type ParentType = gtk::TreeView;
}

impl ObjectImpl for TreeViewObject {
    fn dispose(&self) {
        if let Some(context_menu) = self.context_menu.upgrade() {
            context_menu.unparent();
        }
    }

    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![
                glib::ParamSpecObject::builder::<gtk::PopoverMenu>("context-menu")
                    .nick("Context menu")
                    .blurb("PopoverMenu to use as the context menu")
                    .build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        let obj = self.obj();
        match pspec.name() {
            "context-menu" => {
                if let Some(context_menu) = self.context_menu.upgrade() {
                    context_menu.unparent();
                }

                let context_menu: gtk::PopoverMenu = value.get().unwrap();
                context_menu.set_parent(&*obj);
                self.context_menu.set(Some(&context_menu));
            }
            _ => unimplemented!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "context-menu" => self.context_menu.upgrade().to_value(),
            _ => unimplemented!(),
        }
    }
}

impl WidgetImpl for TreeViewObject {
    fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
        self.parent_size_allocate(width, height, baseline);
        self.context_menu.upgrade().unwrap().present();
    }
}

impl gtk::subclass::prelude::TreeViewImpl for TreeViewObject {}
