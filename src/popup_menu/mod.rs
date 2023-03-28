mod list_row;
mod popover;
mod popupmenu_model;

use std::cell::RefCell;
use std::convert::*;
use std::iter;
use std::ops::Deref;
use std::rc::Rc;

use log::*;

use unicode_width::*;

use gtk::prelude::*;

use crate::{
    highlight::HighlightMap,
    nvim::{self, ErrorReport, NeovimClient, PendingPopupMenu, PopupMenuItem},
    render::{self, CellMetrics},
    shell::RenderState,
    spawn_timeout,
    ui_model::ModelRect,
};
use list_row::{PopupMenuListRow, PopupMenuListRowState, PADDING};
pub use popover::PopupMenuPopover;
use popupmenu_model::{PopupMenuItemRef, PopupMenuModel};

pub const MAX_VISIBLE_ROWS: i32 = 10;

#[derive(Default)]
pub struct State {
    nvim: Option<Rc<nvim::NeovimClient>>,
    items: Rc<Vec<PopupMenuItem>>,
    list_view: gtk::ListView,
    list_model: gtk::SingleSelection,
    list_row_state: Rc<RefCell<PopupMenuListRowState>>,
    item_scroll: gtk::ScrolledWindow,
    info_scroll: gtk::ScrolledWindow,
    info_label: gtk::Label,
    css_provider: gtk::CssProvider,
    open: bool,
    row_height: i32,
    prev_selected: Option<u32>,
    prev_bounds: Option<(f64, f64, f64, f64)>,
    preview: bool,
}

impl State {
    pub fn new() -> Self {
        let list_model = gtk::SingleSelection::builder()
            .can_unselect(true)
            .autoselect(false)
            .build();
        let list_view = gtk::ListView::builder()
            .show_separators(false)
            .single_click_activate(false)
            .model(&list_model)
            .build();
        list_view.add_css_class("nvim-popupmenu-list");

        let css_provider = gtk::CssProvider::new();
        gtk::StyleContext::add_provider_for_display(
            &list_view.display(),
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let item_scroll = gtk::ScrolledWindow::builder()
            .propagate_natural_width(true)
            .propagate_natural_height(true)
            .child(&list_view)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();

        let info_label = gtk::Label::builder()
            .wrap(true)
            .selectable(true)
            .vexpand(true)
            .xalign(0.0)
            .yalign(0.0)
            .margin_top(3)
            .margin_bottom(3)
            .margin_start(3)
            .margin_end(3)
            .build();

        let info_scroll = gtk::ScrolledWindow::builder()
            .propagate_natural_width(true)
            .propagate_natural_height(true)
            .max_content_height(175)
            .child(&info_label)
            .build();

        State {
            nvim: None,
            items: Rc::default(),
            list_view,
            list_model,
            item_scroll,
            info_scroll,
            css_provider,
            info_label,
            row_height: 0,
            list_row_state: Rc::new(RefCell::new(PopupMenuListRowState::default())),
            open: false,
            prev_selected: None,
            prev_bounds: None,
            preview: true,
        }
    }

    fn before_show(&mut self, ctx: PopupMenuContext) {
        if self.nvim.is_none() {
            self.nvim = Some(ctx.nvim.clone());
        }

        let PopupMenuContext {
            selected,
            max_width,
            ..
        } = ctx;

        self.update_list(ctx);

        self.info_scroll.set_max_content_width(max_width);
        self.item_scroll.set_max_content_width(max_width);
        self.item_scroll
            .set_max_content_height(self.row_height * MAX_VISIBLE_ROWS);
        self.item_scroll.hadjustment().set_value(0.0);

        self.select(selected);
    }

    fn limit_column_widths(&mut self, ctx: &PopupMenuContext) {
        const DEFAULT_PADDING: i32 = 10;

        let mut max_word = ("", 0);
        let mut max_kind = ("", 0);
        let mut max_menu = ("", 0);
        for item in ctx.menu_items.iter() {
            let kind_width = item.kind.width_cjk();
            let word_width = item.word.width_cjk();
            let menu_width = item.menu.width_cjk();

            if kind_width > max_kind.1 {
                max_kind = (&item.kind, kind_width);
            }
            if word_width > max_word.1 {
                max_word = (&item.word, word_width);
            }
            if menu_width > max_menu.1 {
                max_menu = (&item.menu, menu_width);
            }
        }
        let max_word = max_word.0;
        let max_kind = max_kind.0;
        let max_menu = max_menu.0;

        /* TODO: Calculate the minimum acceptable column size by allowing each column a guaranteed
         * "greedy" percentage of space (probably 1/3 of the available width). This is to say: if
         * one column wants additional space to avoid ellipsizing itself, it's allowed to request as
         * much space as it wants up to the greedy percentage. Any width it receives beyond this
         * percentage of space is dependent on whether a lower priority column has used it already
         * or not.
         */

        let layout = ctx.font_ctx.create_layout();
        layout.set_text(max_word);
        let (word_max_width, _) = layout.pixel_size();
        let word_column_width = word_max_width + DEFAULT_PADDING;

        let mut row_state = self.list_row_state.borrow_mut();
        if !max_kind.is_empty() {
            layout.set_text(max_kind);
            let (mut kind_width, _) = layout.pixel_size();

            kind_width += DEFAULT_PADDING;
            row_state.kind_col_width = Some(kind_width);
            row_state.word_col_width = (ctx.max_width - kind_width).min(word_column_width);
        } else {
            row_state.word_col_width = ctx.max_width.min(word_column_width);
            row_state.kind_col_width = None;
        }

        if !max_menu.is_empty() {
            let space_left =
                ctx.max_width - row_state.word_col_width - row_state.kind_col_width.unwrap_or(0);

            layout.set_text(max_menu);
            row_state.menu_col_width =
                Some((layout.pixel_size().0 + DEFAULT_PADDING).min(space_left));
        } else {
            row_state.menu_col_width = None;
        }
    }

    fn update_list(&mut self, ctx: PopupMenuContext) {
        if ctx.menu_items.is_empty() {
            return;
        }

        let nvim_client = self.nvim.as_ref().unwrap();
        let nvim = nvim_client.nvim().unwrap();
        let api_info = self.nvim.as_ref().unwrap().api_info().unwrap();

        if api_info.ui_pum_set_height {
            let len = ctx.menu_items.len().min(MAX_VISIBLE_ROWS as usize);
            spawn_timeout!(nvim.ui_pum_set_height(len as i64));
        }

        let CellMetrics {
            pango_ascent,
            pango_descent,
            ..
        } = ctx.font_ctx.cell_metrics();

        // FIXME: We're still doing something with with what we do for calculating
        // CellMetrics.char_height, since using it here doesn't seem to get the right value for
        // determining the y coordinate of the selected item. For now, just use the old method
        let char_height = (pango_ascent + pango_descent) as f64 / pango::SCALE as f64;
        self.row_height = (char_height + (PADDING * 2) as f64).ceil() as i32;

        self.limit_column_widths(&ctx);

        self.items = Rc::new(ctx.menu_items);
        self.list_model
            .set_model(Some(&PopupMenuModel::new(&self.items)));
    }

    fn update_css(&self, hl: &HighlightMap, font_ctx: &render::Context) {
        let font_desc = font_ctx.font_description();

        self.css_provider.load_from_data(&format!(
            "listview.nvim-popupmenu-list {{\
                    background-color: {bg};\
                    font-family: \"{font}\";\
                    font-size: {size}pt;\
                }}\
                listview.nvim-popupmenu-list > row {{\
                    padding: {margin}px;\
                    color: {fg};\
                }}\
                listview.nvim-popupmenu-list > row:selected {{\
                    background-color: {bg_sel};\
                    color: {fg_sel};\
                }}",
            margin = PADDING,
            fg_sel = hl.pmenu_fg_sel().to_hex(),
            bg_sel = hl.pmenu_bg_sel().to_hex(),
            fg = hl.pmenu_fg().to_hex(),
            bg = hl.pmenu_bg().to_hex(),
            font = font_desc.family().unwrap().as_str(),
            size = (font_desc.size() as f64 / pango::SCALE as f64),
        ));
    }

    fn select(&mut self, selected: Option<u32>) {
        if let Some(selected) = selected {
            self.list_model.set_selected(selected);

            // Scroll if necessary to ensure the selected item is in view. We can determine the
            // position to scroll to by taking advantage of the fact that all rows are of equal
            // height.
            if self.items.len() > MAX_VISIBLE_ROWS as usize {
                let row_top = self.row_height * selected as i32;
                let row_bottom = row_top + self.row_height;
                let height = self.list_view.height();
                let vadjust = self.item_scroll.vadjustment();
                let scroll = vadjust.value();

                if scroll > row_top as f64 {
                    vadjust.set_value(row_top as f64);
                } else if scroll + (height as f64) < row_bottom as f64 {
                    vadjust.set_value((row_bottom - height) as f64);
                }
            }

            self.show_info_column(selected);
        } else {
            self.list_model.set_selected(gtk::INVALID_LIST_POSITION);
            self.info_scroll.hide();
        }
        self.prev_selected = selected;
    }

    fn show_info_column(&self, selected: u32) {
        let info = self.items[selected as usize].info.trim();

        if self.preview && !info.is_empty() {
            self.info_label.set_text(info);
            self.info_scroll.vadjustment().set_value(0.0);
            self.info_scroll.hadjustment().set_value(0.0);
            self.info_scroll.show();
            return;
        }

        self.info_scroll.hide();
        self.info_label.set_text("");
    }

    fn set_preview(&mut self, preview: bool) {
        self.preview = preview;
    }
}

pub struct PopupMenu {
    popover: PopupMenuPopover,

    state: Rc<RefCell<State>>,
}

impl PopupMenu {
    pub fn new(render_state: &RenderState) -> PopupMenu {
        let state = State::new();
        state.update_css(&render_state.hl, &render_state.font_ctx);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let popover = PopupMenuPopover::new();
        popover.set_autohide(false);
        popover.set_can_focus(false);
        popover.set_child(Some(&content));
        popover.add_css_class("background");
        popover.add_css_class("nvim-popover");
        popover.set_position(gtk::PositionType::Top);

        content.append(&state.item_scroll);
        content.append(&state.info_scroll);

        let state = Rc::new(RefCell::new(state));
        let state_ref = state.borrow();

        // Setup the signals for rendering the item list
        let item_factory = gtk::SignalListItemFactory::new();

        let list_state_ref = state_ref.list_row_state.clone();
        item_factory.connect_setup(move |_, list_item| {
            list_item.set_child(Some(&PopupMenuListRow::new(&list_state_ref)));
        });
        item_factory.connect_teardown(|_, list_item| {
            list_item.set_child(Option::<&gtk::Widget>::None);
        });
        item_factory.connect_bind(|_, list_item| {
            let row: PopupMenuListRow = list_item.child().unwrap().downcast().unwrap();
            row.set_row(
                list_item
                    .item()
                    .map(|obj| {
                        obj.downcast::<glib::BoxedAnyObject>()
                            .unwrap()
                            .borrow::<PopupMenuItemRef>()
                            .clone()
                    })
                    .as_ref(),
            );
        });
        item_factory.connect_unbind(|_, list_item| {
            let row: PopupMenuListRow = list_item.child().unwrap().downcast().unwrap();
            row.set_row(Option::<&PopupMenuItemRef>::None);
        });

        state_ref.list_view.set_factory(Some(&item_factory));
        state_ref
            .list_view
            .connect_activate(glib::clone!(@weak state => move |_, idx| {
                list_select(&mut state.borrow_mut(), idx, "<C-y>");
            }));
        let list_model = state_ref.list_model.clone();
        state_ref
            .list_view
            .connect_unmap(move |_| list_model.set_model(None::<&PopupMenuModel>));

        drop(state_ref);
        PopupMenu { popover, state }
    }

    pub fn is_open(&self) -> bool {
        self.state.borrow().open
    }

    // Setup bounds reporting for the popup, if this nvim instance supports it
    fn first_time_init(&self, ctx: &PopupMenuContext, render_state: &Rc<RefCell<RenderState>>) {
        /* If nvim hasn't been set to the popup state, this is our first time showing the popup
         * menu
         */
        if self.state.borrow().nvim.is_some() {
            return;
        }

        let nvim_client = ctx.nvim.clone();
        let api_info = ctx
            .nvim
            .api_info()
            .expect("Popup menu being shown before nvim is ready?");

        let nvim = nvim_client.nvim().unwrap();
        if api_info.ui_pum_set_bounds {
            self.popover.connect_bounds_changed(glib::clone!(
                @strong self.state as state, @strong render_state => move |_, x, y, w, h| {
                    let mut state = state.borrow_mut();

                    /* Figure out the actual cell size of the bounds */
                    let render_state = render_state.borrow();
                    let cell_metrics = render_state.font_ctx.cell_metrics();
                    let (mut x, mut y, mut w, mut h) = cell_metrics.get_fractional_grid_area(
                        (x as f64, y as f64, w as f64, h as f64)
                    );

                    if x < 0.0 {
                        w += x;
                        x = 0.0;
                    }
                    if y < 0.0 {
                        h += y;
                        y = 0.0;
                    }

                    if state.prev_bounds != Some((x, y, w, h)) {
                        state.prev_bounds = Some((x, y, w, h));
                        debug!("popup_menu bounds: {w}x{h} @ {x}x{y}");

                        nvim.spawn(glib::clone!(@strong nvim => async move {
                            // XXX the arg order is weird here, but correct
                            nvim.ui_pum_set_bounds(w, h, y, x)
                                .await
                                .report_err();
                        }));
                    }
                }
            ));
            self.popover
                .connect_unmap(glib::clone!(@strong self.state as state => move |_| {
                    state.borrow_mut().prev_bounds = None;
                }));
        }
    }

    pub fn show(&self, ctx: PopupMenuContext, render_state: &Rc<RefCell<RenderState>>) {
        self.first_time_init(&ctx, render_state);

        let mut state = self.state.borrow_mut();
        state.open = true;

        self.popover.set_pointing_to(Some(&gdk::Rectangle::new(
            ctx.x, ctx.y, ctx.width, ctx.height,
        )));
        state.before_show(ctx);
        self.popover.popup();
    }

    pub fn hide(&self) {
        self.state.borrow_mut().open = false;
        self.popover.popdown();
    }

    pub fn select(&self, selected: Option<u32>) {
        self.state.borrow_mut().select(selected);
    }

    pub fn set_preview(&self, preview: bool) {
        self.state.borrow_mut().set_preview(preview);
    }

    pub fn update_css(&self, hl: &HighlightMap, font_ctx: &render::Context) {
        self.state.borrow().update_css(hl, font_ctx);
    }

    // Hide/show the popupmenu, according to the current pending status
    pub fn flush(
        &self,
        pending: PendingPopupMenu,
        nvim: &Rc<NeovimClient>,
        render_state: &Rc<RefCell<RenderState>>,
        max_popup_width: i32,
    ) {
        match pending {
            PendingPopupMenu::Show {
                items: menu_items,
                selected,
                pos: (row, col),
            } => {
                let render_state_ref = render_state.borrow();
                let point = ModelRect::point(col as usize, row as usize);
                let (x, y, width, height) = point.to_area(render_state_ref.font_ctx.cell_metrics());

                let context = PopupMenuContext {
                    nvim,
                    hl: &render_state_ref.hl,
                    font_ctx: &render_state_ref.font_ctx,
                    menu_items,
                    selected,
                    x,
                    y,
                    width,
                    height,
                    max_width: max_popup_width,
                };

                self.show(context, render_state);
            }
            PendingPopupMenu::Select(selected) => self.select(selected),
            PendingPopupMenu::Hide => self.hide(),
            PendingPopupMenu::None => (),
        }
    }
}

impl Deref for PopupMenu {
    type Target = PopupMenuPopover;

    fn deref(&self) -> &Self::Target {
        &self.popover
    }
}

pub struct PopupMenuContext<'a> {
    pub nvim: &'a Rc<NeovimClient>,
    pub hl: &'a HighlightMap,
    pub font_ctx: &'a render::Context,
    pub menu_items: Vec<nvim::PopupMenuItem>,
    pub selected: Option<u32>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub max_width: i32,
}

pub fn list_select(state: &mut State, idx: u32, last_command: &str) {
    if let Some(nvim) = state.nvim.as_ref().unwrap().nvim() {
        let prev = state.prev_selected.map(|p| p as i32).unwrap_or(-1); // TODO: verify this is right
        let idx = idx.try_into().unwrap();
        if prev == idx {
            return;
        }

        let scroll_count = find_scroll_count(prev, idx);
        let apply_command: String = if idx > prev {
            (0..scroll_count)
                .map(|_| "<C-n>")
                .chain(iter::once(last_command))
                .collect()
        } else {
            (0..scroll_count)
                .map(|_| "<C-p>")
                .chain(iter::once(last_command))
                .collect()
        };

        nvim.block_timeout(nvim.input(&apply_command)).report_err();
    }
    state.prev_selected = Some(idx);
}

pub fn find_scroll_count(selected_idx: i32, target_idx: i32) -> i32 {
    if selected_idx < 0 {
        target_idx + 1
    } else if target_idx > selected_idx {
        target_idx - selected_idx
    } else {
        selected_idx - target_idx
    }
}
