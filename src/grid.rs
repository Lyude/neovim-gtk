use std::ops::{Deref, Index, IndexMut};
use std::rc::Rc;

use gdk;
use gtk::{self, prelude::*};
use pango::prelude::*;
use pango::{FontDescription, LayoutExt};

use fnv::FnvHashMap;

use neovim_lib::Value;

use crate::highlight::{Highlight, HighlightMap};
use crate::nvim::{RepaintGridEvent, RepaintMode};
use crate::mode;
use crate::render;
use crate::ui_model::{ModelRect, ModelRectVec, UiModel};

const DEFAULT_GRID: u64 = 1;
const DEFAULT_FONT_NAME: &str = "DejaVu Sans Mono 12";

type ButtonEventCb = Fn(u64, &gdk::EventButton) + 'static;
type KeyEventCb = Fn(u64, &gdk::EventKey) -> Inhibit + 'static;
type ScrollEventCb = Fn(u64, &gdk::EventScroll) + 'static;

struct Callbacks {
    button_press_cb: Option<Box<ButtonEventCb>>,
    button_release_cb: Option<Box<ButtonEventCb>>,
    key_press_cb: Option<Box<KeyEventCb>>,
    key_release_cb: Option<Box<KeyEventCb>>,
    scroll_cb: Option<Box<ScrollEventCb>>,
}

impl Callbacks {
    pub fn new() -> Self {
        Callbacks {
            button_press_cb: None,
            button_release_cb: None,
            key_press_cb: None,
            key_release_cb: None,
            scroll_cb: None,
        }
    }
}

pub struct GridMap {
    grids: FnvHashMap<u64, Grid>,
    fixed: gtk::Fixed,

    callbacks: Rc<Callbacks>,
}

impl Index<u64> for GridMap {
    type Output = Grid;

    fn index(&self, idx: u64) -> &Grid {
        &self.grids[&idx]
    }
}

impl IndexMut<u64> for GridMap {
    fn index_mut(&mut self, idx: u64) -> &mut Grid {
        self.grids.get_mut(&idx).unwrap()
    }
}

impl GridMap {
    pub fn new() -> Self {
        let fixed = gtk::Fixed::new();
        fixed.set_hexpand(true);
        fixed.set_vexpand(true);

        GridMap {
            grids: FnvHashMap::default(),
            fixed,

            callbacks: Rc::new(Callbacks::new()),
        }
    }

    pub fn queue_redraw_all(&mut self, hl: &HighlightMap) {
        for grid_id in self.grids.keys() {
            self.queue_redraw(
                hl,
                &RepaintGridEvent::new(*grid_id, RepaintMode::All),
            );
        }
    }

    pub fn queue_redraw(&mut self, hl: &HighlightMap, ev: &RepaintGridEvent) {
        if let Some(grid) = self.grids.get(&ev.grid_id.unwrap()) {
            match ev.mode {
                RepaintMode::All => {
                    grid.update_dirty_glyphs(hl);
                    grid.drawing_area.queue_draw();
                }
                RepaintMode::Area(ref rect) => grid.queue_draw_area(hl, &[rect]),
                RepaintMode::AreaList(ref list) => grid.queue_draw_area(hl, &list.list),
                RepaintMode::Nothing => (),
            }
        } else {
            warn!("Event from no known grid {:?}", ev.grid_id);
        }
    }

    pub fn current_unwrap(&self) -> &Grid {
        self.grids.get(&DEFAULT_GRID).unwrap()
    }

    pub fn current(&self) -> Option<&Grid> {
        self.grids.get(&DEFAULT_GRID)
    }

    pub fn current_model_mut(&mut self) -> Option<&mut UiModel> {
        self.grids.get_mut(&DEFAULT_GRID).map(|g| &mut g.model)
    }

    pub fn current_model(&self) -> Option<&UiModel> {
        self.grids.get(&DEFAULT_GRID).map(|g| &g.model)
    }

    pub fn get_or_create(&mut self, idx: u64) -> &mut Grid {
        if self.grids.contains_key(&idx) {
            return self.grids.get_mut(&idx).unwrap();
        }

        let grid = Grid::new(idx);
        self.fixed.put(&*grid, 0, 0);

        let cbs = self.callbacks.clone();
        grid.connect_button_press_event(move |_, ev| {
            cbs.button_press_cb.map(|cb| cb(idx, ev));
            Inhibit(false)
        });

        let cbs = self.callbacks.clone();
        grid.connect_button_release_event(move |_, ev| {
            cbs.button_release_cb.map(|cb| cb(idx, ev));
            Inhibit(false)
        });

        let cbs = self.callbacks.clone();
        grid.connect_key_press_event(move |_, ev| {
            cbs.key_press_cb
                .map(|cb| cb(idx, ev))
                .unwrap_or(Inhibit(false))
        });

        let cbs = self.callbacks.clone();
        grid.connect_key_release_event(move |_, ev| {
            cbs.key_release_cb
                .map(|cb| cb(idx, ev))
                .unwrap_or(Inhibit(false))
        });

        let cbs = self.callbacks.clone();
        grid.connect_scroll_event(move |_, ev| {
            cbs.scroll_cb.map(|cb| cb(idx, ev));
            Inhibit(false)
        });

        self.grids.insert(idx, grid);
        self.grids.get_mut(&idx).unwrap()
    }

    pub fn destroy(&mut self, idx: u64) {
        self.grids.remove(&idx);
    }

    pub fn clear_glyphs(&mut self) {
        for grid in self.grids.values_mut() {
            grid.model.clear_glyphs();
        }
    }

    pub fn set_font_description(&mut self, font_description: FontDescription) {
        for grid in self.grids.values_mut() {
            let pango_context = grid.drawing_area.create_pango_context().unwrap();
            pango_context.set_font_description(&font_description);
            grid.font_ctx.update(pango_context);
        }
    }

    pub fn update_font_features(&mut self, font_features: render::FontFeatures) {
        for grid in self.grids.values_mut() {
            grid.font_ctx.update_font_features(font_features);
        }
    }

    pub fn update_line_space(&mut self, line_space: i32) {
        for grid in self.grids.values_mut() {
            grid.font_ctx.update_line_space(line_space);
        }
    }

    pub fn update_mode(&mut self, mode: &str, idx: usize) {
        for grid in self.grids.values_mut() {
            grid.mode.update(mode, idx);
        }
    }

    pub fn set_info(&mut self, cursor_style_enabled: bool, info: Vec<mode::ModeInfo>) {
        for grid in self.grids.values_mut() {
            grid.mode.set_info(cursor_style_enabled, info);
        }
    }
}

impl GridMap {
    pub fn connect_button_press_event<T>(&mut self, cb: T)
    where
        T: Fn(u64, &gdk::EventButton) + 'static,
    {
        Rc::get_mut(&mut self.callbacks).unwrap().button_press_cb = Some(Box::new(cb));
    }

    pub fn connect_button_release_event<T>(&mut self, cb: T)
    where
        T: Fn(u64, &gdk::EventButton) + 'static,
    {
        Rc::get_mut(&mut self.callbacks).unwrap().button_release_cb = Some(Box::new(cb));
    }

    pub fn connect_key_press_event<T>(&mut self, cb: T)
    where
        T: Fn(u64, &gdk::EventKey) -> Inhibit + 'static,
    {
        Rc::get_mut(&mut self.callbacks).unwrap().key_press_cb = Some(Box::new(cb));
    }

    pub fn connect_key_release_event<T>(&mut self, cb: T)
    where
        T: Fn(u64, &gdk::EventKey) -> Inhibit + 'static,
    {
        Rc::get_mut(&mut self.callbacks).unwrap().key_release_cb = Some(Box::new(cb));
    }

    pub fn connect_scroll_event<T>(&mut self, cb: T)
    where
        T: Fn(u64, &gdk::EventScroll) + 'static,
    {
        Rc::get_mut(&mut self.callbacks).unwrap().scroll_cb = Some(Box::new(cb));
    }
}

impl Deref for GridMap {
    type Target = gtk::Fixed;

    fn deref(&self) -> &gtk::Fixed {
        &self.fixed
    }
}

pub struct Grid {
    grid: u64,
    model: UiModel,
    drawing_area: gtk::DrawingArea,
    pub font_ctx: render::Context,
    pub mode: mode::Mode,
}

impl Grid {
    pub fn queue_draw_area<M: AsRef<ModelRect>>(
        &mut self,
        hl: &HighlightMap,
        rect_list: &[M],
    ) {
        // extends by items before, then after changes

        let rects: Vec<_> = rect_list
            .iter()
            .map(|rect| rect.as_ref().clone())
            .map(|mut rect| {
                rect.extend_by_items(&self.model);
                rect
            }).collect();

        self.update_dirty_glyphs(hl);

        let cell_metrics = self.font_ctx.cell_metrics();

        for mut rect in rects {
            rect.extend_by_items(&self.model);

            let (x, y, width, height) = rect.to_area_extend_ink(&self.model, cell_metrics);
            self.drawing_area.queue_draw_area(x, y, width, height);
        }
    }

    pub fn update_dirty_glyphs(&mut self, hl: &HighlightMap) {
        render::shape_dirty(&self.font_ctx, &mut self.model, hl);
    }
}

impl Grid {
    pub fn new(grid: u64) -> Self {
        let drawing_area = gtk::DrawingArea::new();

        drawing_area.set_can_focus(true);

        drawing_area.add_events(
            gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_MOTION_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK
            | gdk::EventMask::ENTER_NOTIFY_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
        );

        let pango_context = drawing_area.create_pango_context().unwrap();
        pango_context.set_font_description(&FontDescription::from_string(DEFAULT_FONT_NAME));

        Grid {
            grid,
            model: UiModel::empty(),
            drawing_area,
            font_ctx: render::Context::new(pango_context),
            mode: mode::Mode::new(),
        }
    }

    pub fn get_cursor(&self) -> (usize, usize) {
        self.model.get_cursor()
    }

    pub fn cur_point(&self) -> ModelRect {
        self.model.cur_point()
    }

    pub fn id(&self) -> u64 {
        self.grid
    }

    pub fn resize(&mut self, columns: u64, rows: u64) {
        if self.model.columns != columns as usize || self.model.rows != rows as usize {
            self.model = UiModel::new(rows, columns);
        }
    }

    pub fn cursor_goto(&mut self, row: usize, col: usize) -> ModelRectVec {
        self.model.set_cursor(row, col)
    }

    pub fn clear(&mut self, default_hl: &Rc<Highlight>) {
        self.model.clear(default_hl);
    }

    pub fn line(
        &mut self,
        row: usize,
        col_start: usize,
        cells: Vec<Vec<Value>>,
        highlights: &HighlightMap,
    ) -> ModelRect {
        let mut hl_id = None;
        let mut col_end = col_start;

        for cell in cells {
            let ch = cell.get(0).unwrap().as_str().unwrap_or("");
            hl_id = cell.get(1).and_then(|h| h.as_u64()).or(hl_id);
            let repeat = cell.get(2).and_then(|r| r.as_u64()).unwrap_or(1) as usize;

            self.model.put(
                row,
                col_end,
                ch,
                ch.is_empty(),
                repeat,
                highlights.get(hl_id),
            );
            col_end += repeat;
        }

        ModelRect::new(row, row, col_start, col_end - 1)
    }

    pub fn scroll(
        &mut self,
        top: u64,
        bot: u64,
        left: u64,
        right: u64,
        rows: i64,
        _: i64,
        default_hl: &Rc<Highlight>,
    ) -> ModelRect {
        self.model.scroll(
            top as i64,
            bot as i64 - 1,
            left as usize,
            right as usize - 1,
            rows,
            default_hl,
        )
    }
}

impl Deref for Grid {
    type Target = gtk::DrawingArea;

    fn deref(&self) -> &gtk::DrawingArea {
        &self.drawing_area
    }
}

fn gtk_draw(state_arc: &Arc<UiMutex<State>>, ctx: &cairo::Context) -> Inhibit {
    let state = state_arc.borrow();
    if state.nvim.is_initialized() {
        draw_content(&*state, ctx);
    } else if state.nvim.is_initializing() {
        draw_initializing(&*state, ctx);
    }

    Inhibit(false)
}

fn draw_content(state: &State, ctx: &cairo::Context) {
    ctx.push_group();

    let render_state = state.render_state.borrow();
    render::render(
        ctx,
        state.cursor.as_ref().unwrap(),
        &render_state.font_ctx,
        state.grids.current_model().unwrap(),
        &render_state.hl,
        state.transparency_settings.filled_alpha(),
    );
    render::fill_background(
        ctx,
        &render_state.hl,
        state.transparency_settings.background_alpha(),
    );

    ctx.pop_group_to_source();
    ctx.paint();
}

fn draw_initializing(state: &State, ctx: &cairo::Context) {
    let render_state = state.render_state.borrow();
    let hl = &render_state.hl;
    let layout = pangocairo::functions::create_layout(ctx).unwrap();
    let alloc = state.drawing_area.get_allocation();

    ctx.set_source_rgb(hl.bg_color.0, hl.bg_color.1, hl.bg_color.2);
    ctx.paint();

    layout.set_text("Loading->");
    let (width, height) = layout.get_pixel_size();

    let x = alloc.width as f64 / 2.0 - width as f64 / 2.0;
    let y = alloc.height as f64 / 2.0 - height as f64 / 2.0;

    ctx.move_to(x, y);
    ctx.set_source_rgb(hl.fg_color.0, hl.fg_color.1, hl.fg_color.2);
    pangocairo::functions::update_layout(ctx, &layout);
    pangocairo::functions::show_layout(ctx, &layout);

    ctx.move_to(x + width as f64, y);
    state
        .cursor
        .as_ref()
        .unwrap()
        .draw(ctx, &render_state.font_ctx, y, false, &hl);
}

