use lazy_static::lazy_static;

use gtk::{
    graphene::{Point, Rect},
    prelude::*,
    subclass::prelude::*,
};

use std::{
    cell::RefCell,
    sync::{Arc, Weak},
};

use crate::{render::*, shell::TransparencySettings, ui::UiMutex};

use crate::cmd_line::State;

glib::wrapper! {
    pub struct CmdlineViewport(ObjectSubclass<CmdlineViewportObject>)
        @extends gtk::Widget,
        @implements gtk::Accessible;
}

impl CmdlineViewport {
    pub fn new() -> Self {
        glib::Object::new::<Self>(&[])
    }

    pub fn set_state(&self, state: &Arc<UiMutex<State>>) {
        self.set_property("cmdline-state", glib::BoxedAnyObject::new(state.clone()));
    }

    pub fn clear_snapshot_cache(&self) {
        self.set_property("snapshot-cached", false);
    }
}

#[derive(Default)]
struct CmdlineViewportInner {
    state: Weak<UiMutex<State>>,
    block_cache: Option<gsk::RenderNode>,
    level_cache: Option<gsk::RenderNode>,
}

#[derive(Default)]
pub struct CmdlineViewportObject {
    inner: RefCell<CmdlineViewportInner>,
}

#[glib::object_subclass]
impl ObjectSubclass for CmdlineViewportObject {
    const NAME: &'static str = "NvimCmdlineViewport";
    type Type = CmdlineViewport;
    type ParentType = gtk::Widget;

    fn class_init(klass: &mut Self::Class) {
        klass.set_css_name("widget");
        klass.set_accessible_role(gtk::AccessibleRole::Widget);
    }
}

impl ObjectImpl for CmdlineViewportObject {
    fn properties() -> &'static [glib::ParamSpec] {
        lazy_static! {
            static ref PROPERTIES: Vec<glib::ParamSpec> = vec![
                glib::ParamSpecObject::new(
                    "cmdline-state",
                    "Cmdline state",
                    "A back-reference to the main state structure for the external cmdline",
                    glib::BoxedAnyObject::static_type(),
                    glib::ParamFlags::WRITABLE
                ),
                glib::ParamSpecBoolean::new(
                    "snapshot-cached",
                    "Snapshot cached",
                    "Whether or not we have a snapshot of the level or block grids cached. Ignores \
                    non-false writes.",
                    false,
                    glib::ParamFlags::READWRITE
                ),
            ];
        }

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            "cmdline-state" => {
                let mut inner = self.inner.borrow_mut();
                debug_assert!(inner.state.upgrade().is_none());

                inner.state =
                    Arc::downgrade(&value.get::<glib::BoxedAnyObject>().unwrap().borrow());
            }
            "snapshot-cached" => {
                if value.get::<bool>().unwrap() == false {
                    let mut inner = self.inner.borrow_mut();
                    inner.block_cache = None;
                    inner.level_cache = None;
                }
            }
            _ => unreachable!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "snapshot-cached" => {
                let inner = self.inner.borrow();
                (inner.level_cache.is_some() || inner.block_cache.is_some()).to_value()
            }
            _ => unreachable!(),
        }
    }
}

impl WidgetImpl for CmdlineViewportObject {
    fn snapshot(&self, snapshot: &gtk::Snapshot) {
        let obj = self.obj();
        let mut inner = self.inner.borrow_mut();
        let state = match inner.state.upgrade() {
            Some(state) => state,
            None => return,
        };
        let mut state = state.borrow_mut();
        let render_state = state.render_state.clone();
        let render_state = render_state.borrow();
        let font_ctx = &render_state.font_ctx;
        let hl = &render_state.hl;

        snapshot.append_color(
            &hl.bg().into(),
            &Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32),
        );

        snapshot.save();

        let preferred_height = state.preferred_height();
        let gap = obj.height() - preferred_height;
        if gap > 0 {
            snapshot.translate(&Point::new(0.0, (gap / 2) as f32));
        }

        if let Some(block) = state.block.as_mut() {
            if inner.block_cache.is_none() {
                inner.block_cache = snapshot_nvim(font_ctx, &mut block.model_layout.model, hl);
            }
            if let Some(ref cache) = inner.block_cache {
                snapshot.append_node(cache);
            }

            snapshot.translate(&Point::new(0.0, block.preferred_height as f32));
        }

        if let Some(level) = state.levels.last_mut() {
            if inner.level_cache.is_none() {
                inner.level_cache = snapshot_nvim(font_ctx, &mut level.model_layout.model, hl);
            }
            if let Some(ref cache) = inner.level_cache {
                snapshot.append_node(cache);
            }
        }

        if let Some(level) = state.levels.last() {
            if let Some(ref cursor) = state.cursor {
                snapshot_cursor(
                    snapshot,
                    cursor,
                    font_ctx,
                    &level.model_layout.model,
                    hl,
                    TransparencySettings::new(), // FIXME
                );
            }
        }

        snapshot.restore();
    }
}
