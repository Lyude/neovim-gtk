use std::cell::RefCell;
use std::cmp::min;
use std::iter;
use std::rc::Rc;
use std::ops::Deref;

use unicode_width::*;

use glib;
use gtk;
use gtk::prelude::*;
use pango;

use crate::highlight::HighlightMap;
use crate::nvim::{self, NvimSession, ErrorReport, NeovimClient};
use crate::render;

const MAX_VISIBLE_ROWS: i32 = 10;

struct State {
    nvim: Option<Rc<nvim::NeovimClient>>,
    renderer: gtk::CellRendererText,
    tree: gtk::TreeView,
    item_scroll: gtk::ScrolledWindow,
    info_scroll: gtk::ScrolledWindow,
    css_provider: gtk::CssProvider,
    info_label: gtk::Label,
    word_column: gtk::TreeViewColumn,
    kind_column: gtk::TreeViewColumn,
    menu_column: gtk::TreeViewColumn,
    preview: bool,
}

impl State {
    pub fn new() -> Self {
        let tree = gtk::TreeView::builder()
            .headers_visible(false)
            .enable_search(false)
            .fixed_height_mode(true)
            .build();
        tree.selection().set_mode(gtk::SelectionMode::Single);

        let css_provider = gtk::CssProvider::new();
        let style_context = tree.style_context();
        style_context.add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let renderer = gtk::CellRendererText::new();
        renderer.set_ellipsize(pango::EllipsizeMode::End);

        // word
        let word_column = gtk::TreeViewColumn::builder()
            .sizing(gtk::TreeViewColumnSizing::Fixed)
            .build();
        word_column.pack_start(&renderer, true);
        word_column.add_attribute(&renderer, "text", 0);
        tree.append_column(&word_column);

        // kind
        let kind_column = gtk::TreeViewColumn::builder()
            .sizing(gtk::TreeViewColumnSizing::Fixed)
            .build();
        kind_column.pack_start(&renderer, true);
        kind_column.add_attribute(&renderer, "text", 1);
        tree.append_column(&kind_column);

        // menu
        let menu_column = gtk::TreeViewColumn::builder()
            .sizing(gtk::TreeViewColumnSizing::Fixed)
            .build();
        menu_column.pack_start(&renderer, true);
        menu_column.add_attribute(&renderer, "text", 2);
        tree.append_column(&menu_column);

        let item_scroll = gtk::ScrolledWindow::builder()
            .propagate_natural_width(true)
            .propagate_natural_height(true)
            .child(&tree)
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
            tree,
            renderer,
            item_scroll,
            info_scroll,
            css_provider,
            info_label,
            word_column,
            kind_column,
            menu_column,
            preview: true,
        }
    }

    fn before_show(&mut self, ctx: PopupMenuContext) {
        if self.nvim.is_none() {
            self.nvim = Some(ctx.nvim.clone());
        }

        self.item_scroll.set_max_content_width(ctx.max_width);
        self.info_scroll.set_max_content_width(ctx.max_width);
        self.item_scroll.set_max_content_height(
            calc_treeview_height(&self.tree, &self.renderer, ctx.menu_items.len())
        );
        self.update_tree(&ctx);
        self.select(ctx.selected);
    }

    fn limit_column_widths(&self, ctx: &PopupMenuContext) {
        const DEFAULT_PADDING: i32 = 5;

        let mut max_word = ("", 0);
        let mut max_kind = ("", 0);
        let mut max_menu = ("", 0);
        for item in ctx.menu_items {
            let kind_width = item.kind.width_cjk();
            let word_width = item.word.width_cjk();
            let menu_width = item.menu.width_cjk();

            if kind_width > max_kind.1 {
                max_kind = (item.kind, kind_width);
            }
            if word_width > max_word.1 {
                max_word = (item.word, word_width);
            }
            if menu_width > max_menu.1 {
                max_menu = (item.menu, menu_width);
            }
        }
        let max_word = max_word.0;
        let max_kind = max_kind.0;
        let max_menu = max_menu.0;

        let layout = ctx.font_ctx.create_layout();
        let max_width = self.item_scroll.max_content_width();
        let (xpad, _) = self.renderer.padding();

        layout.set_text(max_word);
        let (word_max_width, _) = layout.pixel_size();
        let word_column_width = word_max_width + xpad * 2 + DEFAULT_PADDING;

        if !max_kind.is_empty() {
            layout.set_text(max_kind);
            let (kind_width, _) = layout.pixel_size();

            self.kind_column
                .set_fixed_width(kind_width + xpad * 2 + DEFAULT_PADDING);
            self.kind_column.set_visible(true);

            self.word_column
                .set_fixed_width(min(max_width - kind_width, word_column_width));
        } else {
            self.kind_column.set_visible(false);
            self.word_column
                .set_fixed_width(min(max_width, word_column_width));
        }

        if !max_menu.is_empty() {
            layout.set_text(max_menu);
            let (menu_max_width, _) = layout.pixel_size();
            self.menu_column
                .set_fixed_width(menu_max_width + xpad * 2 + DEFAULT_PADDING);
            self.menu_column.set_visible(true);
        } else {
            self.menu_column.set_visible(false);
        }
    }

    fn update_tree(&self, ctx: &PopupMenuContext) {
        if ctx.menu_items.is_empty() {
            return;
        }

        self.limit_column_widths(ctx);

        self.renderer.set_font(Some(ctx.font_ctx.font_description().to_string().as_str()));

        let hl = &ctx.hl;
        self.renderer.set_foreground_rgba(Some(&hl.pmenu_fg().into()));

        update_css(&self.css_provider, hl);

        let list_store = gtk::ListStore::new(&[glib::Type::STRING; 4]);
        let all_column_ids: Vec<u32> = (0..4).map(|i| i as u32).collect();

        for line in ctx.menu_items {
            let line_array: [&dyn glib::ToValue; 4] = [&line.word, &line.kind, &line.menu, &line.info];
            list_store.insert_with_values(
                None,
                all_column_ids
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, line_array[i]))
                .collect::<Box<_>>()
                .as_ref()
            );
        }

        self.tree.set_model(Some(&list_store));
    }

    fn select(&self, selected: i64) {
        if selected >= 0 {
            let selected_path = gtk::TreePath::from_string(&format!("{}", selected)).unwrap();
            self.tree.selection().select_path(&selected_path);
            self.tree.scroll_to_cell(
                Some(&selected_path),
                Option::<&gtk::TreeViewColumn>::None,
                false,
                0.0,
                0.0,
            );

            self.show_info_column(&selected_path);
        } else {
            self.tree.selection().unselect_all();
            self.info_scroll.hide();
        }
    }

    fn show_info_column(&self, selected_path: &gtk::TreePath) {
        let model = self.tree.model().unwrap();
        let iter = model.iter(selected_path);

        if let Some(iter) = iter {
            let info: String = model.get(&iter, 3);
            let info = info.trim();

            if self.preview && !info.is_empty() {
                self.info_label.set_text(info);
                self.info_scroll.vadjustment().set_value(0.0);
                self.info_scroll.hadjustment().set_value(0.0);
                self.info_scroll.show();
                return;
            }
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

        let button_controller = gtk::GestureClick::builder()
            .button(1)
            .build();
        button_controller.connect_pressed(clone!(state => move |_, _, x, y| {
            let state = state.borrow();
            if let Some(nvim) = state.nvim.as_ref().unwrap().nvim() {
                tree_button_press(&state.tree, x, y, &nvim, "<C-y>");
            }
        }));
        state_ref.tree.add_controller(&button_controller);

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
        self.popover.popup()
    }

    pub fn hide(&mut self) {
        self.open = false;
        // popdown() in case of fast hide/show
        // situation does not work and just close popup window
        // so hide() is important here
        self.popover.hide();
    }

    pub fn select(&self, selected: i64) {
        self.state.borrow().select(selected);
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
    pub menu_items: &'a [nvim::CompleteItem<'a>],
    pub selected: i64,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub max_width: i32,
}

pub fn tree_button_press(
    tree: &gtk::TreeView,
    x: f64,
    y: f64,
    nvim: &NvimSession,
    last_command: &str,
) {
    let (paths, ..) = tree.selection().selected_rows();
    let selected_idx = if !paths.is_empty() {
        let ids = paths[0].indices();
        if !ids.is_empty() {
            ids[0]
        } else {
            -1
        }
    } else {
        -1
    };

    if let Some((Some(tree_path), ..)) = tree.path_at_pos(x as i32, y as i32) {
        let target_idx = tree_path.indices()[0];

        let scroll_count = find_scroll_count(selected_idx, target_idx);

        let apply_command: String = if target_idx > selected_idx {
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
}

fn find_scroll_count(selected_idx: i32, target_idx: i32) -> i32 {
    if selected_idx < 0 {
        target_idx + 1
    } else if target_idx > selected_idx {
        target_idx - selected_idx
    } else {
        selected_idx - target_idx
    }
}

pub fn update_css(css_provider: &gtk::CssProvider, hl: &HighlightMap) {
    let bg = hl.pmenu_bg_sel();
    let fg = hl.pmenu_fg_sel();

    css_provider.load_from_data(
        &format!(
            ".view :selected {{ color: {}; background-color: {};}}\n
                .view {{ background-color: {}; }}",
            fg.to_hex(),
            bg.to_hex(),
            hl.pmenu_bg().to_hex(),
        )
        .as_bytes(),
    );
}

pub fn calc_treeview_height(
    tree: &gtk::TreeView,
    renderer: &gtk::CellRendererText,
    item_count: usize,
) -> i32 {
    let (_, natural_size) = renderer.preferred_height(tree);
    let (_, ypad) = renderer.padding();

    let row_height = natural_size + ypad;

    row_height * min(item_count, MAX_VISIBLE_ROWS as usize) as i32
}
