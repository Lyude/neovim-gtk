mod completion_model;
mod list_row;

use std::cell::RefCell;
use std::convert::*;
use std::iter;
use std::rc::Rc;
use std::ops::Deref;

use unicode_width::*;

use glib;
use gtk;
use gtk::prelude::*;

use crate::{
    render::{self, CellMetrics},
    highlight::HighlightMap,
    nvim::{self, ErrorReport, NeovimClient, CompleteItem},
};
use completion_model::{CompletionModel, CompleteItemRef};
use list_row::{CompletionListRow, CompletionListRowState, PADDING};

pub const MAX_VISIBLE_ROWS: i32 = 10;

#[derive(Default)]
pub struct State {
    nvim: Option<Rc<nvim::NeovimClient>>,
    items: Rc<Vec<CompleteItem>>,
    list_view: gtk::ListView,
    list_model: gtk::SingleSelection,
    list_row_state: Rc<RefCell<CompletionListRowState>>,
    item_scroll: gtk::ScrolledWindow,
    info_scroll: gtk::ScrolledWindow,
    info_label: gtk::Label,
    css_provider: gtk::CssProvider,
    row_height: i32,
    prev_selected: Option<u32>,
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
        list_view.add_css_class("nvim-completion-list");

        let css_provider = gtk::CssProvider::new();
        gtk::StyleContext::add_provider_for_display(
            &list_view.display(),
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION
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
            list_row_state: Rc::new(RefCell::new(CompletionListRowState::default())),
            prev_selected: None,
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
        self.item_scroll.set_max_content_height(self.row_height as i32 * MAX_VISIBLE_ROWS);
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
         * "greedy" percentage of space (probably 1/3 of the availavle width). This is to say: if
         * one column wants additional space to avoid ellipsizing itself, it's allowed to request as
         * much space as it wants up to the greedy percentage. Any width it receives beyond this
         * percentage of space is dependent on whether a lower priority column has used it already
         * or not.
         */

        let layout = ctx.font_ctx.create_layout();
        layout.set_text(&max_word);
        let (word_max_width, _) = layout.pixel_size();
        let word_column_width = word_max_width + DEFAULT_PADDING;

        let mut row_state = self.list_row_state.borrow_mut();
        if !max_kind.is_empty() {
            layout.set_text(&max_kind);
            let (mut kind_width, _) = layout.pixel_size();

            kind_width += DEFAULT_PADDING;
            row_state.kind_col_width = Some(kind_width);
            row_state.word_col_width = (ctx.max_width - kind_width).min(word_column_width);
        } else {
            row_state.word_col_width = ctx.max_width.min(word_column_width);
            row_state.kind_col_width = None;
        }

        if !max_menu.is_empty() {
            let space_left = ctx.max_width
                - row_state.word_col_width
                - row_state.kind_col_width.unwrap_or(0);

            layout.set_text(&max_menu);
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

        let CellMetrics { pango_ascent, pango_descent, .. } = ctx.font_ctx.cell_metrics();
        self.row_height =
            (((pango_ascent + pango_descent) as f64 / pango::SCALE as f64)
             + (PADDING * 2) as f64).ceil() as i32;
        self.limit_column_widths(&ctx);
        update_css(&self.css_provider, &ctx.hl, &ctx.font_ctx);

        self.items = Rc::new(ctx.menu_items);
        self.list_model.set_model(Some(&CompletionModel::new(&self.items)));
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
    popover: gtk::Popover,
    open: bool,

    state: Rc<RefCell<State>>,
}

impl PopupMenu {
    pub fn new() -> PopupMenu {
        let state = State::new();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let popover = gtk::Popover::builder()
            .autohide(false)
            .can_focus(false)
            .child(&content)
            .css_classes(vec!["background".into(), "nvim-completion".into()])
            .position(gtk::PositionType::Top)
            .build();

        content.append(&state.item_scroll);
        content.append(&state.info_scroll);

        let state = Rc::new(RefCell::new(state));
        let state_ref = state.borrow();

        // Setup the signals for rendering the item list
        let item_factory = gtk::SignalListItemFactory::new();

        let list_state_ref = state_ref.list_row_state.clone();
        item_factory.connect_setup(move |_, list_item| {
            list_item.set_child(Some(&CompletionListRow::new(&list_state_ref)));
        });
        item_factory.connect_teardown(|_, list_item| {
            list_item.set_child(Option::<&gtk::Widget>::None);
        });
        item_factory.connect_bind(|_, list_item| {
            let row: CompletionListRow = list_item.child().unwrap().downcast().unwrap();
            row.set_row(list_item.item().map(|obj| {
                obj
                    .downcast::<glib::BoxedAnyObject>()
                    .unwrap()
                    .borrow::<CompleteItemRef>()
                    .clone()
            }).as_ref());
        });
        item_factory.connect_unbind(|_, list_item| {
            let row: CompletionListRow = list_item.child().unwrap().downcast().unwrap();
            row.set_row(Option::<&CompleteItemRef>::None);
        });

        state_ref.list_view.set_factory(Some(&item_factory));
        state_ref.list_view.connect_activate(glib::clone!(@weak state => move |_, idx| {
            list_select(&mut state.borrow_mut(), idx, "<C-y>");
        }));
        let list_model = state_ref.list_model.clone();
        state_ref.list_view.connect_unmap(move |_| list_model.set_model(None::<&CompletionModel>));

        drop(state_ref);
        PopupMenu {
            popover,
            state,
            open: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn show(&mut self, ctx: PopupMenuContext) {
        self.open = true;

        self.popover.set_pointing_to(Some(&gdk::Rectangle::new(
            ctx.x,
            ctx.y,
            ctx.width,
            ctx.height
        )));
        self.state.borrow_mut().before_show(ctx);
        self.popover.popup();
    }

    pub fn hide(&mut self) {
        let popover = &self.popover;
        self.open = false;
        glib::idle_add_local_once(clone!(popover => move || {
            // popdown() in case of fast hide/show
            // situation does not work and just close popup window
            // so hide() is important here
            popover.hide();
        }));
    }

    pub fn select(&self, selected: Option<u32>) {
        self.state.borrow_mut().select(selected);
    }

    pub fn set_preview(&self, preview: bool) {
        self.state.borrow_mut().set_preview(preview);
    }
}

impl Deref for PopupMenu {
    type Target = gtk::Popover;

    fn deref(&self) -> &Self::Target {
        &self.popover
    }
}

pub struct PopupMenuContext<'a> {
    pub nvim: &'a Rc<NeovimClient>,
    pub hl: &'a HighlightMap,
    pub font_ctx: &'a render::Context,
    pub menu_items: Vec<nvim::CompleteItem>,
    pub selected: Option<u32>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub max_width: i32,
}

pub fn list_select(
    state: &mut State,
    idx: u32,
    last_command: &str
) {
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

pub fn update_css(css_provider: &gtk::CssProvider, hl: &HighlightMap, font_ctx: &render::Context) {
    let font_desc = font_ctx.font_description();

    css_provider.load_from_data(
        &format!(
            "listview.nvim-completion-list {{\
                background-color: {bg};\
                font-family: \"{font}\";\
                font-size: {size}pt;\
                margin: 0px;\
                padding: 0px;\
            }}\
            listview.nvim-completion-list row {{\
                padding: {margin}px;\
                color: {fg};\
            }}\
            listview.nvim-completion-list row:selected {{\
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
        )
        .as_bytes(),
    );
}
