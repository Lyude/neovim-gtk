use std::cell::RefCell;
use std::cmp::{max, min};
use std::iter;
use std::rc::Rc;
use std::sync::Arc;

use cairo;
use gtk;
use gtk::prelude::*;
use glib;
use pango;

use unicode_segmentation::UnicodeSegmentation;

use crate::cursor;
use crate::highlight::{Highlight, HighlightMap};
use crate::nvim_viewport::NvimViewport;
use crate::mode;
use crate::nvim::{self, NeovimClient};
use crate::popup_menu;
use crate::render::{self, CellMetrics};
use crate::shell;
use crate::ui::UiMutex;
use crate::ui_model::ModelLayout;

pub struct Level {
    model_layout: ModelLayout,
    prompt_offset: usize,
    preferred_width: i32,
    preferred_height: i32,
}

impl Level {
    pub fn insert(&mut self, c: String, shift: bool, render_state: &shell::RenderState) {
        self.model_layout
            .insert_char(c, shift, render_state.hl.default_hl());
        self.update_preferred_size(render_state);
    }

    pub fn replace_from_ctx(&mut self, ctx: &CmdLineContext, render_state: &shell::RenderState) {
        let content = ctx.get_lines(&render_state.hl);
        self.replace_line(content.lines, false);
        self.prompt_offset = content.prompt_offset;
        self.model_layout
            .set_cursor(self.prompt_offset + ctx.pos as usize);
        self.update_preferred_size(render_state);
    }

    pub fn from_ctx(ctx: &CmdLineContext, render_state: &shell::RenderState) -> Self {
        let content = ctx.get_lines(&render_state.hl);
        let mut level = Level::from_lines(content.lines, ctx.max_width, render_state);

        level.prompt_offset = content.prompt_offset;
        level
            .model_layout
            .set_cursor(level.prompt_offset + ctx.pos as usize);
        level.update_preferred_size(render_state);

        level
    }

    fn replace_line(&mut self, lines: Vec<Vec<(Rc<Highlight>, Vec<String>)>>, append: bool) {
        if append {
            self.model_layout.layout_append(lines);
        } else {
            self.model_layout.layout(lines);
        }
    }

    fn update_preferred_size(&mut self, render_state: &shell::RenderState) {
        let &CellMetrics {
            line_height,
            char_width,
            ..
        } = render_state.font_ctx.cell_metrics();

        let (columns, rows) = self.model_layout.size();
        let columns = max(columns, 5);

        self.preferred_width = (char_width * columns as f64) as i32;
        self.preferred_height = (line_height * rows as f64) as i32;
    }

    pub fn from_multiline_content(
        content: &Vec<Vec<(u64, String)>>,
        max_width: i32,
        render_state: &shell::RenderState,
    ) -> Self {
        let lines = content.to_attributed_content(&render_state.hl);
        Level::from_lines(lines, max_width, render_state)
    }

    pub fn from_lines(
        lines: Vec<Vec<(Rc<Highlight>, Vec<String>)>>,
        max_width: i32,
        render_state: &shell::RenderState,
    ) -> Self {
        let &CellMetrics { char_width, .. } = render_state.font_ctx.cell_metrics();

        let max_width_chars = (max_width as f64 / char_width) as u64;

        let mut model_layout = ModelLayout::new(max_width_chars);
        model_layout.layout(lines);

        let mut level = Level {
            model_layout,
            preferred_width: -1,
            preferred_height: -1,
            prompt_offset: 0,
        };

        level.update_preferred_size(render_state);
        level
    }

    fn update_cache(&mut self, render_state: &shell::RenderState) {
        render::shape_dirty(
            &render_state.font_ctx,
            &mut self.model_layout.model,
            &render_state.hl,
        );
    }

    fn set_cursor(&mut self, render_state: &shell::RenderState, pos: usize) {
        self.model_layout.set_cursor(self.prompt_offset + pos);
        self.update_preferred_size(render_state);
    }
}

fn prompt_lines(
    firstc: &str,
    prompt: &str,
    indent: u64,
    hl: &HighlightMap,
) -> (usize, Vec<(Rc<Highlight>, Vec<String>)>) {
    let prompt: Vec<(Rc<Highlight>, Vec<String>)> = if !firstc.is_empty() {
        if firstc.len() >= indent as usize {
            vec![(hl.default_hl(), vec![firstc.to_owned()])]
        } else {
            vec![(
                hl.default_hl(),
                iter::once(firstc.to_owned())
                    .chain((firstc.len()..indent as usize).map(|_| " ".to_owned()))
                    .collect(),
            )]
        }
    } else if !prompt.is_empty() {
        prompt
            .lines()
            .map(|l| {
                (
                    hl.default_hl(),
                    l.graphemes(true).map(|g| g.to_owned()).collect(),
                )
            })
            .collect()
    } else {
        vec![]
    };

    let prompt_offset = prompt.last().map(|l| l.1.len()).unwrap_or(0);

    (prompt_offset, prompt)
}

struct State {
    nvim: Option<Rc<nvim::NeovimClient>>,
    levels: Vec<Level>,
    block: Option<Level>,
    render_state: Rc<RefCell<shell::RenderState>>,
    drawing_area: gtk::DrawingArea,
    cursor: Option<cursor::BlinkCursor<State>>,
}

impl State {
    fn new(drawing_area: gtk::DrawingArea, render_state: Rc<RefCell<shell::RenderState>>) -> Self {
        State {
            nvim: None,
            levels: Vec::new(),
            block: None,
            render_state,
            drawing_area,
            cursor: None,
        }
    }

    fn request_area_size(&self) {
        let drawing_area = self.drawing_area.clone();
        let block = self.block.as_ref();
        let level = self.levels.last();

        let (block_width, block_height) = block
            .map(|b| (b.preferred_width, b.preferred_height))
            .unwrap_or((0, 0));
        let (level_width, level_height) = level
            .map(|l| (l.preferred_width, l.preferred_height))
            .unwrap_or((0, 0));

        drawing_area.set_size_request(
            max(level_width, block_width),
            max(block_height + level_height, 40),
        );
    }

    fn preferred_height(&self) -> i32 {
        let level = self.levels.last();
        level.map(|l| l.preferred_height).unwrap_or(0)
            + self.block.as_ref().map(|b| b.preferred_height).unwrap_or(0)
    }

    fn set_cursor(&mut self, render_state: &shell::RenderState, pos: usize, level: usize) {
        debug_assert!(level > 0);

        // queue old cursor position
        self.queue_redraw_cursor();

        if let Some(l) = self.levels.get_mut(level - 1) {
            l.set_cursor(render_state, pos)
        }
    }

    // FIXME: Get rid of this once we have a better widget then DrawingArea
    #[allow(unused)]
    fn queue_redraw_cursor(&mut self) {
        if let Some(ref level) = self.levels.last() {
            let level_preferred_height = level.preferred_height;
            let block_preferred_height =
                self.block.as_ref().map(|b| b.preferred_height).unwrap_or(0);

            let gap = self.drawing_area.allocated_height()
                - level_preferred_height
                - block_preferred_height;

            let model = &level.model_layout.model;

            let mut cur_point = model.cur_point();
            cur_point.extend_by_items(Some(model));

            let render_state = self.render_state.borrow();
            let cell_metrics = render_state.font_ctx.cell_metrics();

            let (x, y, width, height) = cur_point.to_area_extend_ink(Some(model), cell_metrics);

            self.drawing_area.queue_draw();
            /* TODO: Get our own custom DrawingArea replacement working here instead of doing a full
             * redraw
             */
            /*
            if gap > 0 {
                self.drawing_area
                    .queue_draw_area(x, y + gap / 2, width, height);
            } else {
                self.drawing_area
                    .queue_draw_area(x, y + block_preferred_height, width, height);
            }
            */
        }
    }
}

impl cursor::CursorRedrawCb for State {
    fn queue_redraw_cursor(&mut self) {
        self.queue_redraw_cursor();
    }
}

pub struct CmdLine {
    popover: gtk::Popover,
    wild_tree: gtk::TreeView,
    wild_scroll: gtk::ScrolledWindow,
    wild_css_provider: gtk::CssProvider,
    wild_renderer: gtk::CellRendererText,
    wild_column: gtk::TreeViewColumn,
    displayed: bool,
    state: Arc<UiMutex<State>>,
}

impl CmdLine {
    pub fn new(drawing: &NvimViewport, render_state: Rc<RefCell<shell::RenderState>>) -> Self {
        let popover = gtk::Popover::new();
        popover.set_autohide(false);
        popover.set_position(gtk::PositionType::Right);
        popover.set_parent(drawing);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let drawing_area = gtk::DrawingArea::new();
        content.append(&drawing_area);

        let state = Arc::new(UiMutex::new(State::new(drawing_area.clone(), render_state)));
        let weak_cb = Arc::downgrade(&state);
        let cursor = cursor::BlinkCursor::new(weak_cb);
        state.borrow_mut().cursor = Some(cursor);

        drawing_area.set_draw_func(clone!(state => move |_, ctx, _, _| gtk_draw(ctx, &state)));

        let (wild_scroll, wild_tree, wild_css_provider, wild_renderer, wild_column) =
            CmdLine::create_wildmenu(&state);
        content.append(&wild_scroll);
        popover.set_child(Some(&content));

        CmdLine {
            popover,
            state,
            displayed: false,
            wild_scroll,
            wild_tree,
            wild_css_provider,
            wild_renderer,
            wild_column,
        }
    }

    fn create_wildmenu(
        state: &Arc<UiMutex<State>>,
    ) -> (
        gtk::ScrolledWindow,
        gtk::TreeView,
        gtk::CssProvider,
        gtk::CellRendererText,
        gtk::TreeViewColumn,
    ) {
        let css_provider = gtk::CssProvider::new();

        let tree = gtk::TreeView::new();
        let style_context = tree.style_context();
        style_context.add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

        tree.selection().set_mode(gtk::SelectionMode::Single);
        tree.set_headers_visible(false);
        tree.set_focusable(false);

        let renderer = gtk::CellRendererText::new();
        renderer.set_ellipsize(pango::EllipsizeMode::End);

        let column = gtk::TreeViewColumn::new();
        column.pack_start(&renderer, true);
        column.add_attribute(&renderer, "text", 0);
        tree.append_column(&column);

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_propagate_natural_height(true);
        scroll.set_propagate_natural_width(true);

        scroll.set_child(Some(&tree));

        let controller = gtk::GestureClick::builder()
            .button(1)
            .build();
        controller.connect_pressed(clone!(state => move |controller, _, x, y| {
            let state = state.borrow();
            let nvim = state.nvim.as_ref().unwrap().nvim();
            let tree = controller.widget().downcast().unwrap();
            if let Some(mut nvim) = nvim {
                popup_menu::tree_button_press(&tree, x, y, &mut nvim, "");
            }
        }));
        tree.add_controller(&controller);

        (scroll, tree, css_provider, renderer, column)
    }

    pub fn show_level(&mut self, ctx: &CmdLineContext) {
        let mut state = self.state.borrow_mut();
        if state.nvim.is_none() {
            state.nvim = Some(ctx.nvim.clone());
        }
        let render_state = state.render_state.clone();
        let render_state = render_state.borrow();

        if ctx.level_idx as usize == state.levels.len() {
            let level = state.levels.last_mut().unwrap();
            level.replace_from_ctx(ctx, &*render_state);
            level.update_cache(&*render_state);
        } else {
            let mut level = Level::from_ctx(ctx, &*render_state);
            level.update_cache(&*render_state);
            state.levels.push(level);
        }

        state.request_area_size();

        if !self.displayed {
            self.displayed = true;
            self.popover.set_pointing_to(Some(&gdk::Rectangle::new(
                ctx.x, ctx.y, ctx.width, ctx.height
            )));
            self.popover.popup();
            state.cursor.as_mut().unwrap().start();
        } else {
            state.drawing_area.queue_draw()
        }
    }

    pub fn special_char(
        &self,
        render_state: &shell::RenderState,
        c: String,
        shift: bool,
        level: u64,
    ) {
        let mut state = self.state.borrow_mut();

        if let Some(level) = state.levels.get_mut((level - 1) as usize) {
            level.insert(c, shift, render_state);
            level.update_cache(&*render_state);
        } else {
            error!("Level {} does not exists", level);
        }

        state.request_area_size();
        state.drawing_area.queue_draw()
    }

    pub fn hide_level(&mut self, level_idx: u64) {
        let mut state = self.state.borrow_mut();

        if level_idx as usize == state.levels.len() {
            state.levels.pop();
        }

        if state.levels.is_empty() {
            self.popover.hide();
            self.displayed = false;
            state.cursor.as_mut().unwrap().leave_focus();
        }
    }

    pub fn show_block(&mut self, content: &Vec<Vec<(u64, String)>>, max_width: i32) {
        let mut state = self.state.borrow_mut();
        let mut block =
            Level::from_multiline_content(content, max_width, &*state.render_state.borrow());
        block.update_cache(&*state.render_state.borrow());
        state.block = Some(block);
        state.request_area_size();
    }

    pub fn block_append(&mut self, content: &Vec<(u64, String)>) {
        let mut state = self.state.borrow_mut();
        let render_state = state.render_state.clone();
        {
            let attr_content = content.to_attributed_content(&render_state.borrow().hl);

            let block = state.block.as_mut().unwrap();
            block.replace_line(attr_content, true);
            block.update_preferred_size(&*render_state.borrow());
            block.update_cache(&*render_state.borrow());
        }
        state.request_area_size();
    }

    pub fn block_hide(&self) {
        self.state.borrow_mut().block = None;
    }

    pub fn pos(&self, render_state: &shell::RenderState, pos: u64, level: u64) {
        self.state
            .borrow_mut()
            .set_cursor(render_state, pos as usize, level as usize);
    }

    pub fn set_mode_info(&self, mode_info: Option<mode::ModeInfo>) {
        self.state
            .borrow_mut()
            .cursor
            .as_mut()
            .unwrap()
            .set_mode_info(mode_info);
    }

    pub fn show_wildmenu(
        &self,
        items: Vec<String>,
        render_state: &shell::RenderState,
        max_width: i32,
    ) {
        // update font/color
        self.wild_renderer.set_font(Some(
            render_state
                .font_ctx
                .font_description()
                .to_string()
                .as_str(),
        ));

        self.wild_renderer.set_foreground_rgba(Some(&render_state.hl.pmenu_fg().into()));

        popup_menu::update_css(&self.wild_css_provider, &render_state.hl);

        // set width
        // this calculation produce width more then needed, but this is looks ok :)
        let max_item_width = (items.iter().map(|item| item.len()).max().unwrap() as f64
            * render_state.font_ctx.cell_metrics().char_width) as i32
            + self.state.borrow().levels.last().unwrap().preferred_width;
        self.wild_column
            .set_fixed_width(min(max_item_width, max_width));
        self.wild_scroll.set_max_content_width(max_width);

        // set height
        let treeview_height =
            popup_menu::calc_treeview_height(&self.wild_tree, &self.wild_renderer, items.len());

        // load data
        let list_store = gtk::ListStore::new(&[glib::Type::STRING; 1]);
        for item in items {
            list_store.insert_with_values(None, &[(0, &item)]);
        }
        self.wild_tree.set_model(Some(&list_store));

        self.wild_scroll.set_max_content_height(treeview_height);

        self.wild_scroll.show();
    }

    pub fn hide_wildmenu(&self) {
        self.wild_scroll.hide();
    }

    pub fn wildmenu_select(&self, selected: i64) {
        if selected >= 0 {
            let wild_tree = self.wild_tree.clone();
            glib::idle_add_local_once(move || {
                let selected_path = gtk::TreePath::from_string(&format!("{}", selected)).unwrap();
                wild_tree.selection().select_path(&selected_path);
                wild_tree.scroll_to_cell(Some(&selected_path), Option::<&gtk::TreeViewColumn>::None, false, 0.0, 0.0);
            });
        } else {
            self.wild_tree.selection().unselect_all();
        }
    }
}

fn gtk_draw(ctx: &cairo::Context, state: &Arc<UiMutex<State>>) {
    let state = state.borrow();
    let preferred_height = state.preferred_height();
    let level = state.levels.last();
    let block = state.block.as_ref();

    let render_state = state.render_state.borrow();

    ctx.push_group();

    render::fill_background(ctx, &render_state.hl, None);

    let gap = state.drawing_area.allocated_height() - preferred_height;
    if gap > 0 {
        ctx.translate(0.0, (gap / 2) as f64);
    }

    if let Some(block) = block {
        render::draw(
            ctx,
            &cursor::EmptyCursor::new(),
            &render_state.font_ctx,
            &block.model_layout.model,
            &render_state.hl,
            None,
        );

        ctx.translate(0.0, block.preferred_height as f64);
    }

    if let Some(level) = level {
        render::draw(
            ctx,
            state.cursor.as_ref().unwrap(),
            &render_state.font_ctx,
            &level.model_layout.model,
            &render_state.hl,
            None,
        );
    }

    ctx.pop_group_to_source().unwrap();
    ctx.paint().unwrap();
}

pub struct CmdLineContext<'a> {
    pub nvim: &'a Rc<NeovimClient>,
    pub content: Vec<(u64, String)>,
    pub pos: u64,
    pub firstc: String,
    pub prompt: String,
    pub indent: u64,
    pub level_idx: u64,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub max_width: i32,
}

impl<'a> CmdLineContext<'a> {
    fn get_lines(&self, hl: &HighlightMap) -> LineContent {
        let mut content_line = self.content.to_attributed_content(hl);
        let (prompt_offset, prompt_lines) =
            prompt_lines(&self.firstc, &self.prompt, self.indent, hl);

        let mut content: Vec<_> = prompt_lines.into_iter().map(|line| vec![line]).collect();

        if content.is_empty() {
            content.push(content_line.remove(0));
        } else {
            if let Some(line) = content.last_mut() {
                line.extend(content_line.remove(0))
            }
        }

        LineContent {
            lines: content,
            prompt_offset,
        }
    }
}

struct LineContent {
    lines: Vec<Vec<(Rc<Highlight>, Vec<String>)>>,
    prompt_offset: usize,
}

trait ToAttributedModelContent {
    fn to_attributed_content(&self, hl: &HighlightMap) -> Vec<Vec<(Rc<Highlight>, Vec<String>)>>;
}

impl ToAttributedModelContent for Vec<Vec<(u64, String)>> {
    fn to_attributed_content(&self, hl: &HighlightMap) -> Vec<Vec<(Rc<Highlight>, Vec<String>)>> {
        self.iter()
            .map(|line_chars| {
                line_chars
                    .iter()
                    .map(|c| {
                        (
                            hl.get(c.0.into()),
                            c.1.graphemes(true).map(|g| g.to_owned()).collect(),
                        )
                    })
                    .collect()
            })
            .collect()
    }
}

impl ToAttributedModelContent for Vec<(u64, String)> {
    fn to_attributed_content(&self, hl: &HighlightMap) -> Vec<Vec<(Rc<Highlight>, Vec<String>)>> {
        vec![self
            .iter()
            .map(|c| {
                (
                    hl.get(c.0.into()),
                    c.1.graphemes(true).map(|g| g.to_owned()).collect(),
                )
            })
            .collect()]
    }
}
