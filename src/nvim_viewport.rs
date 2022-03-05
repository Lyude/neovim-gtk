use lazy_static::lazy_static;

use gtk::{
    self,
    graphene,
    prelude::*,
    subclass::prelude::*,
};
use glib;

use std::{
    sync::{Arc, Weak},
    cell::{RefCell, Ref, RefMut},
};

use crate::{
    render, ui_model,
    cursor::*,
    render::CellMetrics,
    highlight::HighlightMap,
    ui::UiMutex,
    shell::{State, RenderState},
};

glib::wrapper! {
    pub struct NvimViewport(ObjectSubclass<NvimViewportObject>)
        @extends gtk::Widget,
        @implements gtk::Accessible;
}

impl NvimViewport {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create NvimViewport")
    }

    pub fn set_shell_state(&self, state: &Arc<UiMutex<State>>) {
        self.set_property("shell_state", glib::BoxedAnyObject::new(state.clone()).to_value());
    }

    pub fn set_context_menu(&self, popover_menu: &gtk::PopoverMenu) {
        self.set_property("context-menu", popover_menu.to_value());
    }

    pub fn set_completion_popover(&self, completion_popover: &gtk::Popover) {
        self.set_property("completion-popover", completion_popover.to_value());
    }
}

#[derive(Default)]
pub struct NvimViewportObject {
    state: RefCell<Weak<UiMutex<State>>>,
    context_menu: glib::WeakRef<gtk::PopoverMenu>,
    completion_popover: glib::WeakRef<gtk::Popover>,
}

#[glib::object_subclass]
impl ObjectSubclass for NvimViewportObject {
    const NAME: &'static str = "NvimViewport";
    type Type = NvimViewport;
    type ParentType = gtk::Widget;

    fn class_init(klass: &mut Self::Class) {
        klass.set_layout_manager_type::<gtk::BinLayout>();
        // TODO: CSS class?
        klass.set_accessible_role(gtk::AccessibleRole::Widget);
    }
}

impl ObjectImpl for NvimViewportObject {
    fn dispose(&self, _obj: &Self::Type) {
        if let Some(popover_menu) = self.context_menu.upgrade() {
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
                    "shell-state",
                    "Shell state",
                    "A back-reference to the main state structure for nvim-gtk",
                    glib::BoxedAnyObject::static_type(),
                    glib::ParamFlags::WRITABLE
                ),
                glib::ParamSpecObject::new(
                    "context-menu",
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
            "shell-state" => {
                debug_assert!(self.state.borrow().upgrade().is_none());

                *self.state.borrow_mut() = Arc::downgrade(
                    &value.get::<glib::BoxedAnyObject>().unwrap().borrow()
                );
            },
            "context-menu" => {
                if let Some(context_menu) = self.context_menu.upgrade() {
                    context_menu.unparent();
                }
                let context_menu: gtk::PopoverMenu = value.get().unwrap();

                context_menu.set_parent(obj);
                self.context_menu.set(Some(&context_menu));
            },
            "completion-popover" => {
                if let Some(popover) = self.completion_popover.upgrade() {
                    popover.unparent();
                }
                let popover: gtk::Popover = value.get().unwrap();

                popover.set_parent(obj);
                self.completion_popover.set(Some(&popover));
            },
            _ => unreachable!(),
        }
    }

    fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "context-menu" => self.context_menu.upgrade().to_value(),
            "completion-popover" => self.completion_popover.upgrade().to_value(),
            _ => unreachable!(),
        }
    }
}

impl WidgetImpl for NvimViewportObject {
    fn size_allocate(&self, widget: &Self::Type, width: i32, height: i32, baseline: i32) {
        self.parent_size_allocate(widget, width, height, baseline);

        self.context_menu.upgrade().unwrap().present();
        self.completion_popover.upgrade().unwrap().present();

        if let Some(state) = self.state.borrow_mut().upgrade() {
            state.borrow_mut().try_nvim_resize();
        }
    }

    fn snapshot(&self, widget: &Self::Type, snapshot: &gtk::Snapshot) {
        self.parent_snapshot(widget, snapshot);

        let state = match self.state.borrow().upgrade() {
            Some(state) => state,
            None => return,
        };
        let state = state.borrow();
        let render_state = state.render_state.borrow();

        // Render nvim background
        snapshot.append_color(
            &render_state.hl.bg().into(), // FIXME: read transparency_settings
            &graphene::Rect::new(
                0.0,
                0.0,
                widget.allocated_width() as f32,
                widget.allocated_height() as f32
            )
        );

        if state.nvim_clone().is_initialized() {
            let mut grids = state.grids.borrow_mut();
            let ui_model = match grids.current_model_mut() {
                Some(ui_model) => ui_model,
                None => return,
            };

            self.snapshot_nvim(
                snapshot,
                state.cursor(),
                &render_state.font_ctx,
                ui_model,
                &render_state.hl
            );
        } else {
            self.snapshot_initializing(widget, snapshot, &render_state)
        }
    }
}

impl NvimViewportObject {
    fn snapshot_initializing(
        &self,
        widget: &<Self as ObjectSubclass>::Type,
        snapshot: &gtk::Snapshot,
        render_state: &RenderState,
    ) {
        let layout = widget.create_pango_layout(Some("Loading…"));

        let attr_list = pango::AttrList::new();
        attr_list.insert(render_state.hl.fg().to_pango_fg());
        layout.set_attributes(Some(&attr_list));

        let (width, height) = layout.pixel_size();
        snapshot.render_layout(
            &widget.style_context(),
            widget.allocated_width() as f64 / 2.0 - width as f64 / 2.0,
            widget.allocated_height() as f64 / 2.0 - height as f64 / 2.0,
            &layout,
        );
    }

    // TODO: Figure out final name of functions, temp names:
    // TODO: Also potentially just move the code in here into nvim_viewport?
    // old: draw_*()
    // new: render_*() (since we use render nodes)
    fn snapshot_nvim<C: Cursor>(
        &self,
        snapshot: &gtk::Snapshot,
        cursor: Option<&C>,
        font_ctx: &render::Context,
        ui_model: &mut ui_model::UiModel,
        hl: &HighlightMap,
    ) {
        let cell_metrics = font_ctx.cell_metrics();
        let &CellMetrics { char_width, line_height, .. } = cell_metrics;

        for (row, line) in ui_model.model_mut().iter_mut().enumerate() {
            let line_y = (row as f64 * line_height) as f32;

            let mut line_x = 0.0;
            for (col, cell) in line.line.iter().enumerate() {
                let pos = (line_x, line_y);

                self.snapshot_cell_bg(snapshot, line, hl, cell, col, pos, cell_metrics, None);
                line_x += char_width as f32;
            }

            line_x = 0.0;
            for (col, cell) in line.line.iter().enumerate() {
                let pos = (line_x, line_y);
                let items = &mut *line.item_line[col];

                self.snapshot_cell(snapshot, items, hl, cell, pos, cell_metrics, 1.0);
                line_x += char_width as f32;
            }

            // TODO: underline/undercurl
        }

        if let Some(cursor) = cursor {
            self.snapshot_cursor(snapshot, cursor, font_ctx, ui_model, hl);
        }
    }

    fn snapshot_cursor<C: Cursor>(
        &self,
        snapshot: &gtk::Snapshot,
        cursor: &C,
        font_ctx: &render::Context,
        ui_model: &ui_model::UiModel,
        hl: &HighlightMap,
    ) {
        if !cursor.is_visible() {
            return;
        }

        let cell_metrics = font_ctx.cell_metrics();
        let (cursor_row, cursor_col) = ui_model.get_cursor();

        let line_x = cursor_col as f64 * cell_metrics.char_width;
        let line_y = cursor_row as f64 * cell_metrics.line_height;

        let cursor_line = match ui_model.model().get(cursor_row) {
            Some(cursor_line) => cursor_line,
            None => return,
        };

        let next_cell = cursor_line.line.get(cursor_col + 1);
        let double_width = next_cell.map_or(false, |c| c.double_width);

        let (clip_y, clip_width, clip_height) =
            cursor_rect(cursor.mode_info(), cell_metrics, line_y, double_width);

        let clip_rect = graphene::Rect::new(
            line_x as f32, clip_y as f32, clip_width as f32, clip_height as f32
        );

        snapshot.push_clip(&clip_rect);
        snapshot.push_opacity(cursor.alpha());

        snapshot.append_color(&hl.cursor_bg().into(), &clip_rect);
        cursor.render(snapshot, font_ctx, line_y, double_width, hl);

        let cell_start_col = cursor_line.cell_to_item(cursor_col);
        let cell_start_line_x =
            line_x - (cursor_col as i32 - cell_start_col) as f64 * cell_metrics.char_width;

        for item in &*cursor_line.item_line[cursor_col] {
            if item.glyphs().is_some() {
                let fg = hl.actual_cell_fg(&cursor_line.line[cursor_col]).inverse(cursor.alpha());

                snapshot.append_node(item.new_render_node(
                    fg.as_ref().into(),
                    (cell_start_line_x as f32, (line_y + cell_metrics.ascent) as f32)
                ));
            }
        }

        // TODO: Underline

        snapshot.pop();
        snapshot.pop();
    }

    fn snapshot_underline_strikethrough(
        snapshot: &gtk::Snapshot,
        line: &ui_model::Line,
        hl: &HighlightMap,
        cell: &ui_model::Cell,
        (line_x, line_y): (f32, f32),
        cell_metrics: &CellMetrics,
        inverse_level: f64,
    ) {
        if cell.hl.strikethrough {
            let fg = hl.actual_cell_fg(cell).inverse(inverse_level);
        }
        todo!();
    }

    fn snapshot_cell_bg(
        &self,
        snapshot: &gtk::Snapshot,
        line: &ui_model::Line,
        hl: &HighlightMap,
        cell: &ui_model::Cell,
        col: usize,
        (line_x, line_y): (f32, f32),
        cell_metrics: &CellMetrics,
        bg_alpha: Option<f64>,
    ) {
        let &CellMetrics {
            char_width,
            line_height,
            ..
        } = cell_metrics;
        //let &RowView {
            //ctx,
            //line,
            //line_y,
            //cell_metrics:
                //&CellMetrics {
                    //char_width,
                    //line_height,
                    //..
                //},
            //..
        //} = cell_view;

        let bg = hl.cell_bg(cell);

        // TODO: Figure out if we need to cache the render nodes here
        if let Some(bg) = bg {
            if !line.is_binded_to_item(col) {
                if bg != hl.bg() {
                    snapshot.append_color(
                        &bg.to_rgbo(bg_alpha.unwrap_or(1.0)),
                        &graphene::Rect::new(
                            line_x,
                            line_y,
                            char_width.ceil() as f32,
                            line_height as f32
                        )
                    );
                }
            } else {
                snapshot.append_color(
                    &bg.into(),
                    &graphene::Rect::new(
                        line_x,
                        line_y,
                        (char_width * line.item_len_from_idx(col) as f64) as f32,
                        line_height as f32
                    )
                );
            }
        }
    }

    /// Generate render nodes for the current cell
    fn snapshot_cell(
        &self,
        snapshot: &gtk::Snapshot,
        items: &mut [ui_model::Item],
        hl: &HighlightMap,
        cell: &ui_model::Cell,
        (line_x, line_y): (f32, f32),
        cell_metrics: &CellMetrics,
        inverse_level: f64,
    ) {
        for item in items {
            if item.glyphs().is_some() {
                let fg = hl.actual_cell_fg(cell).inverse(inverse_level);

                snapshot.append_node(item.render_node(
                    fg.as_ref(),
                    (line_x, line_y + cell_metrics.ascent as f32)
                ));
            }
        }
    }

}
