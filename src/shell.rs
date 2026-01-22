use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::thread;

use log::{debug, error};

use futures::{executor::block_on, FutureExt};

use tokio::sync::{Mutex as AsyncMutex, Notify};

use gdk::{prelude::*, Display, ModifierType};
use gio::ApplicationCommandLine;
use gtk::prelude::*;
use gtk::{Button, MenuButton, Notebook};
use pango::FontDescription;

use nvim_rs::Value;

use crate::color::{Color, COLOR_BLACK, COLOR_WHITE};
use crate::grid::GridMap;
use crate::highlight::{BackgroundState, HighlightMap};
use crate::misc::{decode_uri, escape_filename, split_at_comma};
use crate::nvim::{
    self, CallErrorExt, ErrorReport, NeovimApiInfo, NeovimClient, NormalError, NvimHandler,
    NvimInitError, NvimSession, PendingPopupMenu, RedrawMode, Tabpage,
};
use crate::settings::{FontSource, Settings};
use crate::ui_model::ModelRect;
use crate::{spawn_timeout, spawn_timeout_user_err, NvimTransport};

use crate::cmd_line::{CmdLine, CmdLineContext};
use crate::cursor::{Cursor, CursorRedrawCb};
use crate::input;
use crate::input::keyval_to_input_string;
use crate::mode;
use crate::nvim_viewport::NvimViewport;
use crate::popup_menu::PopupMenu;
use crate::render;
use crate::render::CellMetrics;
use crate::subscriptions::{SubscriptionHandle, SubscriptionKey, Subscriptions};
use crate::tabline::Tabline;
use crate::ui::{Components, UiMutex};
use crate::Args;

const DEFAULT_FONT_NAME: &str = "DejaVu Sans Mono 12";
pub const MINIMUM_SUPPORTED_NVIM_VERSION: &str = "0.3.2";

macro_rules! idle_cb_call {
    ($state:ident.$cb:ident($( $x:expr ),*)) => (
        glib::idle_add_once(move || {
            if let Some(ref cb) = $state.borrow().$cb {
                (&mut *cb.borrow_mut())($($x),*);
            }
        });
    )
}

pub struct RenderState {
    pub font_ctx: render::Context,
    pub hl: HighlightMap,
    pub mode: mode::Mode,
}

impl RenderState {
    pub fn new(pango_context: pango::Context) -> Self {
        RenderState {
            font_ctx: render::Context::new(pango_context),
            hl: HighlightMap::new(),
            mode: mode::Mode::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct TransparencySettings {
    pub background_alpha: f64,
    pub filled_alpha: f64,
}

impl TransparencySettings {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for TransparencySettings {
    fn default() -> Self {
        TransparencySettings {
            background_alpha: 1.0,
            filled_alpha: 1.0,
        }
    }
}

/// Contains state related to resize requests we are going to/have sent to nvim
pub struct ResizeRequests {
    /// The most recently submitted resize request, if any. This might not have been received by
    /// neovim yet.
    pub current: Option<(i32, i32)>,
    /// The next resize request to submit to neovim, if any.
    pub requested: Option<(i32, i32)>,
    /// Whether there's a resize future active or not
    active: bool,
}

pub struct ResizeState {
    /// The current state of neovim's resize requests
    pub requests: AsyncMutex<ResizeRequests>,
    /// Signal when we've finished a resize request
    autocmd_status: Notify,
}

impl ResizeState {
    pub fn notify_finished(&self) {
        self.autocmd_status.notify_one();
    }
}

/// A collection of all header bar buttons used in nvim-gtk
pub struct HeaderBarButtons {
    open_btn: MenuButton,
    new_tab_btn: Button,
    paste_btn: Button,
    save_btn: Button,
    primary_menu_btn: MenuButton,
}

impl HeaderBarButtons {
    pub fn new(
        open_btn: MenuButton,
        new_tab_btn: Button,
        paste_btn: Button,
        save_btn: Button,
        primary_menu_btn: MenuButton,
    ) -> Self {
        Self {
            open_btn,
            new_tab_btn,
            paste_btn,
            primary_menu_btn,
            save_btn,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.new_tab_btn.set_sensitive(enabled);
        self.paste_btn.set_sensitive(enabled);
        self.save_btn.set_sensitive(enabled);
        self.primary_menu_btn.set_sensitive(enabled);

        // Use an idle callback for open_btn, as we might be calling this from one of its own
        // callbacks which would result in borrowing it mutably twice
        let open_btn = self.open_btn.clone();
        glib::idle_add_local_once(move || open_btn.set_sensitive(enabled));
    }
}

/// A struct containing all of the widgets in neovim-gtk that interact with Neovim in some way using
/// RPC calls. They are grouped together so that they may be easily enabled/disabled when nvim is
/// blocked/unblocked.
pub struct ActionWidgets {
    header_bar: Option<Box<HeaderBarButtons>>,
    tabs: Notebook,
    file_browser: gtk::Box,
}

impl ActionWidgets {
    /// Enable or disable all widgets
    pub fn set_enabled(&self, enabled: bool) {
        if let Some(ref header_bar) = self.header_bar {
            header_bar.set_enabled(enabled);
        }
        self.tabs.set_sensitive(enabled);
        self.file_browser.set_sensitive(enabled);
    }
}

type CommandCallback = Box<dyn FnMut(&mut State, nvim::NvimCommand) + Send + 'static>;
type DetachedCallback = Box<RefCell<dyn FnMut() + Send + 'static>>;
type NvimStartedCallback = Box<RefCell<dyn FnMut() + Send + 'static>>;

pub struct State {
    pub grids: GridMap,

    mouse_enabled: bool,
    nvim: Rc<NeovimClient>,
    cursor: Option<Cursor<State>>,
    popup_menu: PopupMenu,
    cmd_line: CmdLine,
    settings: Rc<RefCell<Settings>>,
    pub render_state: Rc<RefCell<RenderState>>,

    resize_status: Arc<ResizeState>,
    focus_state: Arc<AsyncMutex<FocusState>>,

    pub clipboard_clipboard: gdk::Clipboard,
    pub clipboard_primary: gdk::Clipboard,

    stack: gtk::Stack,
    pub nvim_viewport: NvimViewport,
    pub pending_redraw: RedrawMode,
    pub pending_popupmenu: PendingPopupMenu,
    tabs: Tabline,
    im_context: gtk::IMMulticontext,
    error_area: crate::error::ErrorArea,

    pub options: RefCell<Args>,
    transparency_settings: TransparencySettings,

    detach_cb: Option<DetachedCallback>,
    nvim_started_cb: Option<NvimStartedCallback>,
    command_cb: Option<CommandCallback>,

    subscriptions: RefCell<Subscriptions>,

    action_widgets: Arc<UiMutex<Option<ActionWidgets>>>,

    app_cmdline: Rc<RefCell<Option<ApplicationCommandLine>>>,
}

impl State {
    pub fn new(settings: Rc<RefCell<Settings>>, options: Args) -> State {
        let nvim_viewport = NvimViewport::new();

        let pango_context = nvim_viewport.create_pango_context();
        pango_context.set_font_description(Some(&FontDescription::from_string(DEFAULT_FONT_NAME)));

        let mut render_state = RenderState::new(pango_context);
        render_state.hl.set_use_cterm(options.cterm_colors);

        let popup_menu = PopupMenu::new(&render_state);
        let render_state = Rc::new(RefCell::new(render_state));

        let cmd_line = CmdLine::new(&nvim_viewport, render_state.clone());

        let display = Display::default().unwrap();

        State {
            grids: GridMap::new(),
            nvim: Rc::new(NeovimClient::new()),
            mouse_enabled: true,
            cursor: None,
            popup_menu,
            cmd_line,
            settings,
            render_state,

            resize_status: Arc::new(ResizeState {
                requests: AsyncMutex::new(ResizeRequests {
                    current: None,
                    requested: None,
                    active: false,
                }),
                autocmd_status: Notify::new(),
            }),
            focus_state: Arc::new(AsyncMutex::new(FocusState {
                last: true,
                next: true,
                is_pending: false,
            })),

            clipboard_clipboard: display.clipboard(),
            clipboard_primary: display.primary_clipboard(),

            // UI
            stack: gtk::Stack::new(),
            nvim_viewport,
            pending_redraw: RedrawMode::Nothing,
            pending_popupmenu: PendingPopupMenu::None,
            tabs: Tabline::new(),
            im_context: gtk::IMMulticontext::new(),
            error_area: crate::error::ErrorArea::new(),

            options: RefCell::new(options),
            transparency_settings: TransparencySettings::new(),

            detach_cb: None,
            nvim_started_cb: None,
            command_cb: None,

            subscriptions: RefCell::new(Subscriptions::new()),

            action_widgets: Arc::new(UiMutex::new(None)),

            app_cmdline: Rc::new(RefCell::new(None)),
        }
    }

    pub fn nvim(&self) -> Option<NvimSession> {
        self.nvim.nvim()
    }

    pub fn nvim_clone(&self) -> Rc<NeovimClient> {
        self.nvim.clone()
    }

    pub fn set_action_widgets(
        &self,
        header_bar: Option<Box<HeaderBarButtons>>,
        file_browser: gtk::Box,
    ) {
        self.action_widgets.replace(Some(ActionWidgets {
            header_bar,
            tabs: self.tabs.clone(),
            file_browser,
        }));
    }

    pub fn action_widgets(&self) -> Arc<UiMutex<Option<ActionWidgets>>> {
        self.action_widgets.clone()
    }

    pub fn start_nvim_initialization(&self) -> bool {
        if self.nvim.is_uninitialized() {
            self.nvim.set_in_progress();
            true
        } else {
            false
        }
    }

    pub fn set_detach_cb<F>(&mut self, cb: Option<F>)
    where
        F: FnMut() + Send + 'static,
    {
        if let Some(c) = cb {
            self.detach_cb = Some(Box::new(RefCell::new(c)));
        } else {
            self.detach_cb = None;
        }
    }

    pub fn set_nvim_started_cb<F>(&mut self, cb: Option<F>)
    where
        F: FnMut() + Send + 'static,
    {
        if let Some(c) = cb {
            self.nvim_started_cb = Some(Box::new(RefCell::new(c)));
        } else {
            self.nvim_started_cb = None;
        }
    }

    pub fn set_nvim_command_cb<F>(&mut self, cb: Option<F>)
    where
        F: FnMut(&mut State, nvim::NvimCommand) + Send + 'static,
    {
        if let Some(c) = cb {
            self.command_cb = Some(Box::new(c));
        } else {
            self.command_cb = None;
        }
    }

    pub fn set_font_desc(&mut self, desc: &str) {
        let font_description = FontDescription::from_string(desc);

        if font_description.size() <= 0 {
            error!("Font size must be > 0");
            return;
        }

        let pango_context = self.nvim_viewport.create_pango_context();
        pango_context.set_font_description(Some(&font_description));

        self.render_state
            .borrow_mut()
            .font_ctx
            .update(pango_context);
        self.grids.clear_glyphs();
        self.try_nvim_resize();
        self.queue_draw(RedrawMode::All);
    }

    pub fn set_font_features(&mut self, font_features: String) {
        let font_features = render::FontFeatures::from(font_features);

        self.render_state
            .borrow_mut()
            .font_ctx
            .update_font_features(font_features);
        self.grids.clear_glyphs();
        self.queue_draw(RedrawMode::All);
    }

    pub fn set_line_space(&mut self, line_space: String) {
        let line_space: i32 = match line_space.parse() {
            Ok(line_space) => line_space,
            Err(e) => {
                error!("Can't convert argument to integer: {e}");
                return;
            }
        };

        self.render_state
            .borrow_mut()
            .font_ctx
            .update_line_space(line_space);
        self.grids.clear_glyphs();
        self.try_nvim_resize();
        self.queue_draw(RedrawMode::All);
    }

    pub fn set_transparency(&mut self, background_alpha: f64, filled_alpha: f64) {
        self.transparency_settings = TransparencySettings {
            background_alpha,
            filled_alpha,
        };
        self.queue_draw(RedrawMode::ClearCache);
    }

    pub fn transparency(&self) -> TransparencySettings {
        self.transparency_settings
    }

    pub fn set_cursor_blink(&mut self, val: i32) {
        if let Some(cursor) = &mut self.cursor {
            cursor.set_cursor_blink(val);
        }
    }

    pub fn set_exit_status(&self, val: i32) {
        let lock = self.app_cmdline.borrow();
        let r: &ApplicationCommandLine = lock.as_ref().unwrap();
        r.set_exit_status(val);
    }

    pub fn open_file(&self, path: &str) {
        if let Some(nvim) = self.nvim() {
            let action_widgets = self.action_widgets();
            let path = format!("e {path}");

            action_widgets.borrow().as_ref().unwrap().set_enabled(false);

            nvim.clone().spawn(async move {
                let res = nvim.command(&path).await;

                glib::idle_add_once(move || {
                    if let Some(action_widgets) = action_widgets.borrow().as_ref() {
                        action_widgets.set_enabled(true);
                    }
                });

                if let Err(e) = res {
                    if let Ok(e) = NormalError::try_from(&*e) {
                        // Filter out errors we get if the user is presented with a prompt
                        if !e.has_code(325) {
                            e.print(&nvim).await;
                        }
                        return;
                    }
                    e.print();
                }
            });
        }
    }

    pub fn cd(&self, path: &str) {
        if let Some(nvim) = self.nvim() {
            let path = format!("cd {path}");
            spawn_timeout!(nvim.command(&path));
        }
    }

    pub fn clipboard_clipboard_set(&self, text: &str) {
        self.clipboard_clipboard.set_text(text);
    }

    pub fn clipboard_primary_set(&self, text: &str) {
        self.clipboard_primary.set_text(text);
    }

    fn close_popup_menu(&self) {
        if self.popup_menu.is_open() {
            if let Some(nvim) = self.nvim() {
                nvim.block_timeout(nvim.input("<Esc>")).report_err();
            }
        }
    }

    fn update_dirty_glyphs(&mut self) {
        let render_state = self.render_state.borrow();
        if let Some(model) = self.grids.current_model_mut() {
            render::shape_dirty(&render_state.font_ctx, model, &render_state.hl);
        }
    }

    fn im_commit(&self, ch: &str) {
        if let Some(nvim) = self.nvim() {
            input::im_input(&nvim, ch);
        }
    }

    fn calc_nvim_size_from(&self, (w, h): (i32, i32)) -> (i32, i32) {
        let &CellMetrics {
            line_height,
            char_width,
            ..
        } = self.render_state.borrow().font_ctx.cell_metrics();

        (
            ((w as f64 / char_width).trunc() as i32).max(1),
            ((h as f64 / line_height).trunc() as i32).max(3),
        )
    }

    fn calc_nvim_size(&self) -> (i32, i32) {
        self.calc_nvim_size_from((self.nvim_viewport.width(), self.nvim_viewport.height()))
    }

    fn show_error_area(&self) {
        let stack = self.stack.clone();
        glib::idle_add_local_once(move || stack.set_visible_child_name("Error"));
    }

    fn set_im_location(&self) {
        if let Some((row, col)) = self.grids.current().map(|g| g.get_cursor()) {
            let (x, y, width, height) = ModelRect::point(col, row)
                .to_area(self.render_state.borrow().font_ctx.cell_metrics());

            self.im_context
                .set_cursor_location(&gdk::Rectangle::new(x, y, width, height));
            self.im_context.reset();
        }
    }

    pub fn resize_status(&self) -> Arc<ResizeState> {
        self.resize_status.clone()
    }

    pub fn try_nvim_resize(&mut self) {
        let nvim = match self.nvim() {
            Some(nvim) => nvim,
            None => return,
        };

        {
            let mut status = nvim.block_on(self.resize_status.requests.lock());

            let our_req = self.calc_nvim_size();
            if status.requested == Some(our_req) {
                return;
            } else if status.current == Some(our_req) {
                if status.requested.is_some() {
                    debug!("Resize request matches last committed size, cancelling reqs");
                }
                status.requested = None;
                return;
            }

            debug!("Requesting resize to {our_req:?}");
            status.requested.replace(our_req);

            // Finish if the UI isn't ready yet
            if status.current.is_none() {
                return;
            }

            // Don't spawn a resize future if one's already active
            if status.active {
                debug!("Request already pending, not starting new one");
                return;
            }
            status.active = true;
        }

        let status_ref = self.resize_status.clone();
        nvim.clone().spawn(async move {
            loop {
                let (cols, rows) = {
                    let mut status = status_ref.requests.lock().await;
                    let req = status.requested.take();

                    if let Some((cols, rows)) = req {
                        status.current = req;
                        (cols, rows)
                    } else {
                        status.active = false;
                        debug!("No new resize requests, finishing");
                        return;
                    }
                };

                debug!("Committing new size {cols}x{rows}...");

                /* We don't use subscriptions for this since we want to ensure that there's
                 * no potential for RPC requests between autocmd registration and our resize
                 * request
                 */
                nvim.call_atomic(vec![
                    Value::Array(vec![
                        "nvim_command".into(),
                        Value::Array(vec![
                            "au VimResized * ++once cal rpcnotify(1, 'resized')".into()
                        ]),
                    ]),
                    Value::Array(vec![
                        "nvim_ui_try_resize".into(),
                        Value::Array(vec![cols.into(), rows.into()]),
                    ]),
                ])
                .await
                .report_err();

                // Wait for the resize request to finish, and then update the request state
                status_ref.autocmd_status.notified().await;
            }
        });
    }

    fn edit_paste(&self, clipboard: &'static str) {
        let nvim = self.nvim();
        if let Some(nvim) = nvim {
            let render_state = self.render_state.borrow();
            if render_state.mode.is(&mode::NvimMode::Insert)
                || render_state.mode.is(&mode::NvimMode::Normal)
            {
                spawn_timeout_user_err!(nvim.command(&format!("normal! \"{clipboard}Pl")));
            } else {
                spawn_timeout_user_err!(nvim.input(&format!("<C-r>{clipboard}")));
            };
        }
    }

    fn edit_copy(&self, clipboard: &'static str) {
        if let Some(nvim) = self.nvim() {
            spawn_timeout_user_err!(nvim.command(&format!("normal! \"{clipboard}y")));
        }
    }

    fn max_popup_width(&self) -> i32 {
        self.nvim_viewport.width() - 20
    }

    pub fn subscribe<F>(&self, key: SubscriptionKey, args: &[&str], cb: F) -> SubscriptionHandle
    where
        F: Fn(Vec<String>) + 'static,
    {
        self.subscriptions.borrow_mut().subscribe(key, args, cb)
    }

    pub fn set_autocmds(&self) {
        let Some(nvim) = self.nvim() else {
            return;
        };
        self.subscriptions.borrow().set_autocmds(&nvim);
    }

    pub fn notify(&self, params: Vec<Value>) -> Result<(), String> {
        self.subscriptions.borrow().notify(params)
    }

    pub fn run_now(&self, handle: &SubscriptionHandle) {
        let Some(nvim) = self.nvim() else {
            return;
        };
        self.subscriptions.borrow().run_now(handle, &nvim);
    }

    pub fn set_font(&mut self, font_desc: String) {
        self.set_font_rpc(&font_desc);
    }

    pub fn set_font_rpc(&mut self, font_desc: &str) {
        {
            let mut settings = self.settings.borrow_mut();
            settings.set_font_source(FontSource::Rpc);
        }

        self.set_font_desc(font_desc);
    }

    pub fn on_command(&mut self, command: nvim::NvimCommand) {
        let mut cb = self.command_cb.take();

        if let Some(ref mut cb) = cb {
            cb(self, command);
        }

        self.command_cb = cb;
    }

    pub fn focus_update(&self, state: bool) {
        let nvim = {
            let mut focus_state = block_on(self.focus_state.lock());
            if focus_state.next == state {
                return;
            }
            focus_state.next = state;

            if focus_state.is_pending {
                // A future is still running, no need for another
                return;
            } else if let Some(nvim) = self.nvim() {
                focus_state.is_pending = true;
                nvim
            } else {
                return;
            }
        };

        let focus_state = self.focus_state.clone();
        nvim.clone().spawn(async move {
            loop {
                let next = {
                    let mut focus_state = focus_state.lock().await;
                    if focus_state.next == focus_state.last {
                        focus_state.is_pending = false;
                        return;
                    }

                    focus_state.last = focus_state.next;
                    focus_state.next
                };
                let autocmd = if next { "FocusGained" } else { "FocusLost" };

                debug!("Triggering {autocmd} autocmd");
                nvim.command(&format!("if exists('#{autocmd}')|doau {autocmd}|endif"))
                    .await
                    .report_err();
            }
        });
    }

    pub fn set_tabline(&self, visible: bool) {
        self.tabs.set_visible(visible)
    }

    pub fn set_background(&self, background: BackgroundState) {
        self.render_state
            .borrow_mut()
            .hl
            .set_background_state(background)
    }

    pub fn cursor(&self) -> Option<&Cursor<State>> {
        self.cursor.as_ref()
    }
}

pub struct UiState {
    mouse_pressed: bool,
    cursor_visible: Option<bool>,

    scroll_delta: (f64, f64),

    /// Last reported editor position (col, row)
    last_nvim_pos: (u64, u64),
    /// Last reported motion position
    last_pos: (f64, f64),
}

impl UiState {
    pub fn new() -> UiState {
        UiState {
            mouse_pressed: false,
            cursor_visible: None,
            scroll_delta: (0.0, 0.0),
            last_nvim_pos: (0, 0),
            last_pos: (0.0, 0.0),
        }
    }

    /// Set whether or not the cursor for the drawing area is visible. We cache this in UiState
    /// since otherwise we'd end up creating a new cursor every single time we receive a motion
    /// event
    fn set_cursor_visible(&mut self, nvim_viewport: &NvimViewport, visible: bool) {
        if Some(visible) == self.cursor_visible {
            return;
        }

        self.cursor_visible = Some(visible);
        let cursor = match visible {
            true => "text",
            false => "none",
        };

        nvim_viewport.set_cursor(gdk::Cursor::from_name(cursor, None).as_ref());
    }
}

async fn gtk_drop_receive(drop: &gdk::Drop) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Big fat hack: GDK language bindings for 4.x before 4.6 don't provide us with
    // GDK_FILE_LIST_TYPE. Waiting for 4.6 would be lame and we're too cool for that, so let's just
    // go hunt down the GType for it ourselves!
    let file_list_type = drop
        .formats()
        .types()
        .into_iter()
        .find(|t| t.name() == "GdkFileList")
        .expect("Failed to find GdkFileList GType")
        .to_owned();

    let value = drop
        .read_value_future(file_list_type, glib::Priority::DEFAULT)
        .await?;

    // We won't have GdkFileList until 4.6, however we know that GdkFileList is just a boxed GSList
    // type. So, use witch magic to extract the boxed GSList pointer ourselves.
    let raw_value = value.into_raw();
    let value = unsafe {
        let value = glib::gobject_ffi::g_value_get_boxed(&raw_value) as *mut glib::ffi::GSList;

        glib::SList::<gio::File>::from_glib_full(value)
    };

    Ok(value.into_iter().map(|f| f.uri().to_string()).collect())
}

fn gtk_handle_drop(state: &State, context: &glib::MainContext, drop: &gdk::Drop) -> bool {
    let nvim = match state.nvim() {
        Some(nvim) => nvim,
        None => return false,
    };
    let action_widgets = state.action_widgets();

    action_widgets.borrow().as_ref().unwrap().set_enabled(false);

    // TODO: Figure out timeout situation here
    let drop = drop.clone();
    context.spawn_local(async move {
        let input = match gtk_drop_receive(&drop).await {
            Ok(input) => input,
            Err(e) => {
                nvim.err_writeln(&format!("Drag and drop failed: {e}"))
                    .await
                    .report_err();
                drop.finish(gdk::DragAction::empty());
                if let Some(action_widgets) = action_widgets.borrow().as_ref() {
                    action_widgets.set_enabled(true);
                }
                return;
            }
        };

        match nvim
            .command(
                input
                    .into_iter()
                    .filter_map(|uri| decode_uri(&uri))
                    .fold("ar".to_owned(), |command, filename| {
                        format!("{} {}", command, escape_filename(&filename))
                    })
                    .as_str(),
            )
            .await
        {
            Err(e) => {
                match NormalError::try_from(&*e) {
                    Ok(e) => {
                        if !e.has_code(325) {
                            e.print(&nvim).await;
                        }
                    }
                    Err(_) => e.print(),
                };
                drop.finish(gdk::DragAction::empty());
            }
            Ok(_) => drop.finish(gdk::DragAction::COPY),
        };

        if let Some(action_widgets) = action_widgets.borrow().as_ref() {
            action_widgets.set_enabled(true);
        }
    });
    true
}

pub struct Shell {
    pub state: Arc<UiMutex<State>>,
    ui_state: Rc<RefCell<UiState>>,

    widget: gtk::Box,
}

impl Shell {
    pub fn new(settings: Rc<RefCell<Settings>>, options: Args) -> Shell {
        let shell = Shell {
            state: Arc::new(UiMutex::new(State::new(settings, options))),
            ui_state: Rc::new(RefCell::new(UiState::new())),

            widget: gtk::Box::new(gtk::Orientation::Vertical, 0),
        };

        let shell_ref = Arc::downgrade(&shell.state);
        shell.state.borrow_mut().cursor = Some(Cursor::new(shell_ref));

        shell
            .state
            .borrow()
            .nvim_viewport
            .set_shell_state(&shell.state);

        shell
    }

    pub fn is_nvim_initialized(&self) -> bool {
        let state = self.state.borrow();
        state.nvim.is_initialized()
    }

    pub fn init(
        &mut self,
        app_cmdline: Rc<RefCell<Option<ApplicationCommandLine>>>,
        components: &Arc<UiMutex<Components>>,
    ) {
        self.state.borrow_mut().app_cmdline = app_cmdline;

        let state_ref = &self.state;
        let ui_state_ref = &self.ui_state;
        let state = state_ref.borrow_mut();

        state.nvim_viewport.set_hexpand(true);
        state.nvim_viewport.set_vexpand(true);
        state.nvim_viewport.set_focusable(true);
        state.nvim_viewport.set_focus_on_click(true);
        state.nvim_viewport.set_receives_default(true);
        state
            .nvim_viewport
            .set_completion_popover(&state.popup_menu);

        state.im_context.set_use_preedit(false);

        let nvim_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        nvim_box.append(&*state.tabs);
        nvim_box.append(&state.nvim_viewport);

        state.stack.add_named(&nvim_box, Some("Nvim"));
        state.stack.add_named(&*state.error_area, Some("Error"));

        self.widget.append(&state.stack);

        let motion_controller = gtk::EventControllerMotion::new();
        motion_controller.connect_motion(glib::clone!(
            #[weak]
            state_ref,
            #[strong]
            ui_state_ref,
            move |controller, x, y| {
                gtk_motion_notify(
                    &state_ref.borrow(),
                    &mut ui_state_ref.borrow_mut(),
                    (x, y),
                    controller.current_event_state(),
                );
            }
        ));
        motion_controller.connect_enter(glib::clone!(
            #[weak]
            state_ref,
            #[strong]
            ui_state_ref,
            move |controller, x, y| {
                gtk_motion_notify(
                    &state_ref.borrow(),
                    &mut ui_state_ref.borrow_mut(),
                    (x, y),
                    controller.current_event_state(),
                );
            }
        ));
        state.nvim_viewport.add_controller(motion_controller);

        let key_controller = gtk::EventControllerKey::new();
        key_controller.set_im_context(Some(&state.im_context));
        key_controller.connect_key_pressed(glib::clone!(
            #[strong]
            ui_state_ref,
            #[weak]
            state_ref,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let mut state = state_ref.borrow_mut();
                state.cursor.as_mut().unwrap().reset_state();
                ui_state_ref
                    .borrow_mut()
                    .set_cursor_visible(&state.nvim_viewport, false);

                match state.nvim() {
                    Some(nvim) => input::gtk_key_press(&nvim, key, modifiers),
                    None => glib::Propagation::Proceed,
                }
            }
        ));
        state.nvim_viewport.add_controller(key_controller);

        fn get_button(controller: &gtk::GestureClick) -> u32 {
            match controller.current_button() {
                0 => 1, // 0 == no button, e.g. it's a touch event, so map it to left click
                button => button,
            }
        }

        let menu = self.create_context_menu();
        state.nvim_viewport.set_context_menu(&menu);
        let click_controller = gtk::GestureClick::builder().n_points(1).button(0).build();
        click_controller.connect_pressed(glib::clone!(
            #[weak]
            state_ref,
            #[strong]
            ui_state_ref,
            #[strong]
            menu,
            move |controller, _, x, y| {
                let state = state_ref.borrow();
                gtk_button_press(
                    &state,
                    &ui_state_ref,
                    get_button(controller),
                    x,
                    y,
                    controller.current_event_state(),
                    &menu,
                )
            }
        ));
        click_controller.connect_released(glib::clone!(
            #[weak]
            state_ref,
            #[strong]
            ui_state_ref,
            move |controller, _, x, y| {
                let state = state_ref.borrow();
                gtk_button_release(
                    &state,
                    &mut ui_state_ref.borrow_mut(),
                    get_button(controller),
                    x,
                    y,
                    controller.current_event_state(),
                )
            }
        ));
        state.nvim_viewport.add_controller(click_controller);

        let long_tap_controller = gtk::GestureLongPress::builder()
            .n_points(1)
            .touch_only(true)
            .build();
        long_tap_controller.connect_pressed(glib::clone!(
            #[weak]
            state_ref,
            #[strong]
            ui_state_ref,
            move |controller, x, y| {
                let state = state_ref.borrow();
                gtk_button_press(
                    &state,
                    &ui_state_ref,
                    3,
                    x,
                    y,
                    controller.current_event_state(),
                    &menu,
                )
            }
        ));
        state.nvim_viewport.add_controller(long_tap_controller);

        let focus_controller = gtk::EventControllerFocus::new();
        focus_controller.connect_enter(glib::clone!(
            #[weak]
            state_ref,
            move |_| {
                let mut state = state_ref.borrow_mut();
                let redraw_mode = state.cursor.as_mut().unwrap().set_widget_focus(true);
                state.queue_draw(redraw_mode);
            }
        ));
        focus_controller.connect_leave(glib::clone!(
            #[weak]
            state_ref,
            move |_| {
                let mut state = state_ref.borrow_mut();
                let redraw_mode = state.cursor.as_mut().unwrap().set_widget_focus(false);
                state.queue_draw(redraw_mode);
            }
        ));
        state.nvim_viewport.add_controller(focus_controller);

        let scroll_controller = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::DISCRETE,
        );
        scroll_controller.connect_scroll(glib::clone!(
            #[strong]
            ui_state_ref,
            #[weak]
            state_ref,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |controller, dx, dy| {
                gtk_scroll_event(
                    &mut state_ref.borrow_mut(),
                    &mut ui_state_ref.borrow_mut(),
                    (dx, dy),
                    controller.current_event_state(),
                );

                glib::Propagation::Proceed
            }
        ));
        state.nvim_viewport.add_controller(scroll_controller);

        let context = glib::MainContext::default();
        let dnd_target = gtk::DropTargetAsync::new(
            Some(gdk::ContentFormats::new(&["text/uri-list"])),
            gdk::DragAction::COPY,
        );
        dnd_target.connect_drop(glib::clone!(
            #[weak]
            state_ref,
            #[upgrade_or_panic]
            move |_, drop, _, _| {
                let state = state_ref.borrow();
                gtk_handle_drop(&state, &context, drop)
            }
        ));
        state.nvim_viewport.add_controller(dnd_target);

        state.nvim_viewport.connect_realize(glib::clone!(
            #[weak]
            state_ref,
            move |viewport| {
                let window: gtk::Window = viewport.root().unwrap().downcast().unwrap();

                // sometime set_client_window does not work without idle_add
                // and looks like not enabled im_context
                glib::idle_add_local_once(glib::clone!(
                    #[strong]
                    state_ref,
                    #[strong]
                    window,
                    move || {
                        state_ref
                            .borrow()
                            .im_context
                            .set_client_widget(Some(&window));
                    }
                ));

                let mut state = state_ref.borrow_mut();
                let redraw = state
                    .cursor
                    .as_mut()
                    .unwrap()
                    .set_toplevel_focus(window.is_active());
                if state.nvim().is_some() {
                    state.queue_draw(redraw);
                }

                window.connect_is_active_notify(glib::clone!(
                    #[strong]
                    state_ref,
                    move |window| {
                        gtk_active_notify(&mut state_ref.borrow_mut(), window.is_active());
                    }
                ));
            }
        ));

        state.im_context.connect_commit(glib::clone!(
            #[weak]
            state_ref,
            #[strong]
            ui_state_ref,
            move |_, ch| {
                let mut state = state_ref.borrow_mut();

                state.cursor.as_mut().unwrap().reset_state();
                ui_state_ref
                    .borrow_mut()
                    .set_cursor_visible(&state.nvim_viewport, false);
                state.im_commit(ch);
            }
        ));

        state.nvim_viewport.connect_map(glib::clone!(
            #[weak]
            state_ref,
            #[strong(rename_to = resize_state)]
            state.resize_status,
            #[strong]
            components,
            move |_| {
                init_nvim(&state_ref, &resize_state, &components);
            }
        ));
    }

    fn create_context_menu(&self) -> gtk::PopoverMenu {
        let state_ref = &self.state;

        let action_group = gio::SimpleActionGroup::new();

        let copy = gio::SimpleAction::new("copy", None);
        copy.connect_activate(glib::clone!(
            #[weak]
            state_ref,
            move |_, _| {
                let state = state_ref.borrow();
                state.edit_copy("+")
            }
        ));
        action_group.add_action(&copy);

        let paste = gio::SimpleAction::new("paste", None);
        paste.connect_activate(glib::clone!(
            #[weak]
            state_ref,
            move |_, _| {
                let state = state_ref.borrow();
                state.edit_paste("+")
            }
        ));
        action_group.add_action(&paste);

        let menu = gio::Menu::new();
        let section = gio::Menu::new();
        section.append(Some("Copy"), Some("menu.copy"));
        section.append(Some("Paste"), Some("menu.paste"));
        menu.append_section(None, &section);

        let popover = gtk::PopoverMenu::builder()
            .position(gtk::PositionType::Bottom)
            .menu_model(&menu)
            .has_arrow(false)
            .build();
        popover.insert_action_group("menu", Some(&action_group));

        popover.connect_closed(|popover| {
            if let Some(nvim_viewport) = popover.parent() {
                nvim_viewport.grab_focus();
            }
        });

        popover
    }

    #[cfg(unix)]
    pub fn set_font_desc(&self, font_name: &str) {
        self.state.borrow_mut().set_font_desc(font_name);
    }

    pub fn grab_focus(&self) {
        self.state.borrow().nvim_viewport.grab_focus();
    }

    pub fn open_file(&self, path: &str) {
        self.state.borrow().open_file(path);
    }

    pub fn cd(&self, path: &str) {
        self.state.borrow().cd(path);
    }

    pub fn detach_ui(&self) {
        let state = self.state.borrow();
        let nvim_client = state.nvim.clone();
        let api_info = nvim_client.api_info();

        if let Some(nvim) = nvim_client.nvim() {
            nvim_client.clear();
            nvim.block_timeout(nvim.ui_detach()).report_err();
            nvim.block_on(nvim.shutdown(api_info.map_or(1, |i| i.channel)));
        }
    }

    pub fn edit_paste(&self) {
        self.state.borrow().edit_paste("+");
    }

    pub fn edit_save_all(&self) {
        if let Some(nvim) = self.state.borrow().nvim() {
            spawn_timeout_user_err!(nvim.command(":wa"));
        }
    }

    pub fn new_tab(&self) {
        if let Some(nvim) = self.state.borrow().nvim() {
            spawn_timeout!(nvim.command(":tabe"));
        }
    }

    pub fn set_detach_cb<F>(&self, cb: Option<F>)
    where
        F: FnMut() + Send + 'static,
    {
        let mut state = self.state.borrow_mut();
        state.set_detach_cb(cb);
    }

    pub fn set_nvim_started_cb<F>(&self, cb: Option<F>)
    where
        F: FnMut() + Send + 'static,
    {
        let mut state = self.state.borrow_mut();
        state.set_nvim_started_cb(cb);
    }

    pub fn set_nvim_command_cb<F>(&self, cb: Option<F>)
    where
        F: FnMut(&mut State, nvim::NvimCommand) + Send + 'static,
    {
        let mut state = self.state.borrow_mut();
        state.set_nvim_command_cb(cb);
    }

    pub fn set_completeopts(&self, options: &str) {
        self.state
            .borrow()
            .popup_menu
            .set_preview(options.contains("preview"));
    }

    pub fn set_exit_status(&self, status: i32) {
        self.state.borrow().set_exit_status(status);
    }
}

impl Deref for Shell {
    type Target = gtk::Box;

    fn deref(&self) -> &gtk::Box {
        &self.widget
    }
}

/// Keeps track of focus/unfocus requests for neovim.
struct FocusState {
    /// The last focus state we sent to neovim, which may or may not have been received yet.
    last: bool,
    /// The next focus state to send to neovim, if any. If there's no new state to send, this is
    /// equal to `last`.
    next: bool,
    /// Whether there's a focus/unfocus request being sent to neovim.
    is_pending: bool,
}

fn gtk_active_notify(state: &mut State, active: bool) {
    state.focus_update(active);
    if active {
        state.im_context.focus_in();
    } else {
        state.im_context.focus_out();
    }

    let redraw = state.cursor.as_mut().unwrap().set_toplevel_focus(active);
    state.queue_draw(redraw);
}

fn gtk_scroll_event(
    state: &mut State,
    ui_state: &mut UiState,
    (dx, dy): (f64, f64),
    modifier_state: ModifierType,
) {
    if !state.mouse_enabled && !state.nvim.is_initializing() {
        return;
    }

    state.close_popup_menu();

    // Remember and accumulate scroll deltas, so slow scrolling still
    // works.
    ui_state.scroll_delta.0 += dx;
    ui_state.scroll_delta.1 += dy;

    // Perform scroll action for deltas with abs(delta) >= 1.
    let x = ui_state.scroll_delta.0 as isize;
    let y = ui_state.scroll_delta.1 as isize;
    for _ in 0..x {
        mouse_input(state, "wheel", "right", modifier_state, ui_state.last_pos)
    }
    for _ in 0..-x {
        mouse_input(state, "wheel", "left", modifier_state, ui_state.last_pos)
    }
    for _ in 0..y {
        mouse_input(state, "wheel", "down", modifier_state, ui_state.last_pos)
    }
    for _ in 0..-y {
        mouse_input(state, "wheel", "up", modifier_state, ui_state.last_pos)
    }
    // Subtract performed scroll deltas.
    ui_state.scroll_delta.0 -= x as f64;
    ui_state.scroll_delta.1 -= y as f64;
}

fn gtk_button_press(
    shell: &State,
    ui_state: &Rc<RefCell<UiState>>,
    button: u32,
    x: f64,
    y: f64,
    modifier_state: ModifierType,
    menu: &gtk::PopoverMenu,
) {
    if shell.mouse_enabled {
        if button != 3 {
            ui_state.borrow_mut().mouse_pressed = true;
        }

        match button {
            1 => mouse_input(shell, "left", "press", modifier_state, (x, y)),
            2 => mouse_input(shell, "middle", "press", modifier_state, (x, y)),
            3 => {
                menu.set_pointing_to(Some(&gdk::Rectangle::new(
                    x.round() as i32,
                    y.round() as i32,
                    0,
                    0,
                )));

                // Popping up the menu will trigger a focus event, so handle this in the idle loop
                // to avoid a double borrow_mut()
                glib::idle_add_local_once(glib::clone!(
                    #[strong]
                    menu,
                    move || menu.popup()
                ));
            }
            _ => (),
        }
    }
}

fn mouse_input(
    shell: &State,
    button: &'static str,
    action: &'static str,
    state: ModifierType,
    position: (f64, f64),
) {
    if let Some(nvim) = shell.nvim() {
        let (col, row) = mouse_coordinates_to_nvim(shell, position);
        let input_string = keyval_to_input_string("", state);

        let nvim_clone = nvim.clone();
        nvim.spawn(async move {
            nvim_clone.input_mouse(button, action, &input_string, 0, row as i64, col as i64)
                .await
                .ok_and_report()
                .expect("Can't send mouse input event");
        });
    }
}

/**
 * Translate gtk mouse event coordinates to nvim (col, row).
 */
fn mouse_coordinates_to_nvim(shell: &State, position: (f64, f64)) -> (u64, u64) {
    let &CellMetrics {
        line_height,
        char_width,
        ..
    } = shell.render_state.borrow().font_ctx.cell_metrics();
    let (x, y) = position;
    let col = (x / char_width).trunc() as u64;
    let row = (y / line_height).trunc() as u64;
    (col, row)
}

fn gtk_button_release(
    shell: &State,
    ui_state: &mut UiState,
    button: u32,
    x: f64,
    y: f64,
    modifier_state: ModifierType,
) {
    if button != 3 {
        ui_state.mouse_pressed = false;
    }

    if shell.mouse_enabled && !shell.nvim.is_initializing() {
        match button {
            1 => mouse_input(shell, "left", "release", modifier_state, (x, y)),
            2 => mouse_input(shell, "middle", "release", modifier_state, (x, y)),
            // We don't handle 3 here since that's used for the right click context menu
            _ => (),
        }
    }
}

fn gtk_motion_notify(
    shell: &State,
    ui_state: &mut UiState,
    position: (f64, f64),
    modifier_state: ModifierType,
) {
    if shell.mouse_enabled && ui_state.mouse_pressed {
        let pos = mouse_coordinates_to_nvim(shell, position);

        // if we fire LeftDrag on the same coordinates multiple times, then
        // we get: https://github.com/daa84/neovim-gtk/issues/185
        if pos != ui_state.last_nvim_pos {
            mouse_input(shell, "left", "drag", modifier_state, position);
            ui_state.last_nvim_pos = pos;
        }
    }

    ui_state.last_pos = position;
    ui_state.set_cursor_visible(&shell.nvim_viewport, true);
}

fn show_nvim_start_error(
    err: &nvim::NvimInitError,
    state_arc: Arc<UiMutex<State>>,
    comps: Arc<UiMutex<Components>>,
) {
    match err {
        NvimInitError::ResponseError { .. } => {
            let source = err.source();
            let cmd = err.cmd().unwrap().to_owned();
            glib::idle_add_once(move || {
                let state = state_arc.borrow();
                state.nvim.set_error();
                comps.borrow().window().remove_css_class("nvim-background");
                state.error_area.show_nvim_start_error(&source, &cmd);
                state.show_error_area();
            });
        }
        NvimInitError::MissingCapability(_) => unreachable!(),
        NvimInitError::TcpConnectError { ref addr, .. } => {
            let addr = addr.to_string();
            let source = err.source();
            glib::idle_add_once(move || {
                let state = state_arc.borrow();
                state.nvim.set_error();
                comps.borrow().window().remove_css_class("nvim-background");
                state.error_area.show_nvim_tcp_connect_error(&source, &addr);
                state.show_error_area();
            });
        }
        #[cfg(unix)]
        NvimInitError::UnixConnectError { ref addr, .. } => {
            let addr = addr.to_string_lossy().to_string();
            let source = err.source();
            glib::idle_add_once(move || {
                let state = state_arc.borrow();
                state.nvim.set_error();
                comps.borrow().window().remove_css_class("nvim-background");
                state
                    .error_area
                    .show_nvim_unix_connect_error(&source, &addr);
                state.show_error_area();
            });
        }
    }
}

fn show_nvim_init_error(
    err: &nvim::NvimInitError,
    state_arc: Arc<UiMutex<State>>,
    comps: Arc<UiMutex<Components>>,
) {
    let error_msg = format!("{err}");

    glib::idle_add_once(move || {
        let state = state_arc.borrow();
        state.nvim.set_error();
        comps.borrow().window().remove_css_class("nvim-background");
        state.error_area.show_nvim_init_error(&error_msg);
        state.show_error_area();
    });
}

fn init_nvim_async(
    state_arc: Arc<UiMutex<State>>,
    comps: Arc<UiMutex<Components>>,
    resize_status: Arc<ResizeState>,
    nvim_handler: NvimHandler,
    options: Args,
    cols: i32,
    rows: i32,
) {
    let nvim_result = match options.server {
        None => nvim::start(
            nvim_handler,
            options.nvim_bin_path.clone(),
            *options.timeout,
            options.nvim_args,
        ),
        Some(NvimTransport::SocketAddr(addr)) => {
            nvim::start_tcp_client(nvim_handler, addr, *options.timeout)
        }
        #[cfg(unix)]
        Some(NvimTransport::UnixSocket(addr)) => {
            nvim::start_unix_socket_client(nvim_handler, addr, *options.timeout)
        }
    };

    let (session, io_future) = match nvim_result {
        Ok(session) => session,
        Err(err) => {
            show_nvim_start_error(&err, state_arc, comps);
            return;
        }
    };

    set_nvim_to_state(state_arc.clone(), &session);

    // add callback on session end
    let cb_state_arc = state_arc.clone();
    session.spawn(io_future.map(|r| {
        if let Err(e) = r {
            if !e.is_reader_error() {
                error!("{e}");
            }
        }

        glib::idle_add_once(move || {
            cb_state_arc.borrow().nvim.clear();
            if let Some(ref cb) = cb_state_arc.borrow().detach_cb {
                (*cb.borrow_mut())();
            }
        });
    }));

    // attach ui
    let input_data = options.input_data;
    session.clone().spawn(async move {
        let mut initialized = false;

        match nvim::post_start_init(session.clone(), resize_status, input_data, rows, cols).await {
            Ok(api_info) => {
                set_nvim_initialized(state_arc.clone(), api_info);
                initialized = true;
            }
            Err(ref e) => show_nvim_init_error(e, state_arc.clone(), comps.clone()),
        }

        if initialized {
            if let Err(ref e) = session
                .timeout(session.command("runtime! ginit.vim"))
                .await
                .map_err(NvimInitError::new_post_init)
            {
                show_nvim_init_error(e, state_arc, comps);
            }
        }
    });
}

fn set_nvim_to_state(state_arc: Arc<UiMutex<State>>, nvim: &NvimSession) {
    let (sender, receiver) = mpsc::channel();
    let nvim = nvim.clone();

    glib::idle_add_once(move || {
        state_arc.borrow().nvim.set(nvim);
        let _ = sender.send(());
    });

    // Wait idle set nvim properly
    let _ = receiver.recv();
}

fn set_nvim_initialized(state_arc: Arc<UiMutex<State>>, api_info: NeovimApiInfo) {
    glib::idle_add_once(glib::clone!(
        #[strong]
        state_arc,
        move || {
            let mut state = state_arc.borrow_mut();
            state.nvim.set_initialized(api_info);
            // in some case resize can happens while initialization in progress
            // so force resize here
            state.try_nvim_resize();
            state.cursor.as_mut().unwrap().start();
        }
    ));

    idle_cb_call!(state_arc.nvim_started_cb());
}

fn init_nvim(
    state_ref: &Arc<UiMutex<State>>,
    resize_state: &Arc<ResizeState>,
    components: &Arc<UiMutex<Components>>,
) {
    let state = state_ref.borrow_mut();
    if state.start_nvim_initialization() {
        let (cols, rows) = state.calc_nvim_size_from(components.borrow().saved_size());

        debug!("Init nvim {cols}/{rows}");

        let nvim_handler = NvimHandler::new(state_ref.clone(), state.resize_status());
        let options = state.options.borrow_mut().input_data();
        thread::spawn(glib::clone!(
            #[strong]
            state_ref,
            #[strong]
            components,
            #[strong]
            resize_state,
            move || {
                init_nvim_async(
                    state_ref,
                    components,
                    resize_state,
                    nvim_handler,
                    options,
                    cols,
                    rows,
                )
            }
        ));
    }
}

// Neovim redraw events
impl State {
    pub fn queue_draw(&mut self, mode: RedrawMode) {
        if mode == RedrawMode::Nothing {
            return;
        }

        if mode >= RedrawMode::ClearCache {
            if mode == RedrawMode::All {
                self.update_dirty_glyphs();
            }
            self.nvim_viewport.clear_snapshot_cache();
        }

        self.grids.flush_cursor();
        self.nvim_viewport.queue_draw();
    }

    pub fn grid_line(
        &mut self,
        grid: u64,
        row: u64,
        col_start: u64,
        cells: Vec<Vec<Value>>,
    ) -> RedrawMode {
        let hl = &self.render_state.borrow().hl;
        self.grids[grid].line(row as usize, col_start as usize, cells, hl);
        RedrawMode::All
    }

    pub fn grid_clear(&mut self, grid: u64) -> RedrawMode {
        let hl = &self.render_state.borrow().hl;
        self.grids[grid].clear(&hl.default_hl());
        RedrawMode::All
    }

    pub fn grid_destroy(&mut self, grid: u64) -> RedrawMode {
        self.grids.destroy(grid);
        RedrawMode::All
    }

    pub fn grid_cursor_goto(&mut self, grid: u64, row: u64, column: u64) -> RedrawMode {
        self.grids[grid].cursor_goto(row as usize, column as usize);
        self.set_im_location();
        RedrawMode::Cursor
    }

    pub fn grid_resize(&mut self, grid: u64, columns: u64, rows: u64) -> RedrawMode {
        debug!("on_resize {columns}/{rows}");

        let Some(nvim) = self.nvim() else {
            return RedrawMode::Nothing;
        };
        nvim.block_on(async {
            let mut resize_state = self.resize_status.requests.lock().await;

            if resize_state.current.is_none() {
                resize_state.current = Some((columns as i32, rows as i32));
            }
        });

        self.grids.get_or_create(grid).resize(columns, rows);
        RedrawMode::Nothing
    }

    pub fn grid_scroll(
        &mut self,
        grid: u64,
        top: u64,
        bot: u64,
        left: u64,
        right: u64,
        rows: i64,
        cols: i64,
    ) -> RedrawMode {
        let hl = &self.render_state.borrow().hl.default_hl();
        self.grids[grid].scroll(top, bot, left, right, rows, cols, hl);
        RedrawMode::All
    }

    pub fn hl_attr_define(
        &mut self,
        id: u64,
        rgb_attr: HashMap<String, Value>,
        _: &Value,
        info: Vec<HashMap<String, Value>>,
    ) -> RedrawMode {
        let mut render_state = self.render_state.borrow_mut();
        let updated = render_state.hl.set(id, &rgb_attr, &info);

        if updated.pmenu {
            self.popup_menu
                .update_css(&render_state.hl, &render_state.font_ctx);
        }

        if updated.cursor {
            RedrawMode::Cursor
        } else {
            RedrawMode::Nothing
        }
    }

    pub fn default_colors_set(
        &mut self,
        fg: Option<u64>,
        bg: Option<u64>,
        sp: Option<u64>,
        cterm_fg: i64,
        cterm_bg: i64,
    ) -> RedrawMode {
        self.render_state.borrow_mut().hl.set_defaults(
            fg.map(Color::from_indexed_color),
            bg.map(Color::from_indexed_color),
            sp.map(Color::from_indexed_color),
            if cterm_fg > 0 {
                Color::from_cterm((cterm_fg - 1) as u8)
            } else {
                COLOR_WHITE
            },
            if cterm_bg > 0 {
                Color::from_cterm((cterm_bg - 1) as u8)
            } else {
                COLOR_BLACK
            },
        );
        RedrawMode::ClearCache
    }

    fn cur_point_area(&self) -> RedrawMode {
        if self.grids.current().is_some() {
            RedrawMode::Cursor
        } else {
            RedrawMode::Nothing
        }
    }

    pub fn on_mode_change(&mut self, mode: String, idx: u64) -> RedrawMode {
        let mut render_state = self.render_state.borrow_mut();
        render_state.mode.update(&mode, idx as usize);
        self.cursor
            .as_mut()
            .unwrap()
            .set_mode_info(render_state.mode.mode_info().cloned());
        self.cmd_line
            .set_mode_info(render_state.mode.mode_info().cloned());

        self.cur_point_area()
    }

    pub fn on_mouse(&mut self, on: bool) -> RedrawMode {
        self.mouse_enabled = on;
        RedrawMode::Nothing
    }

    pub fn on_busy(&mut self, busy: bool) -> RedrawMode {
        if busy {
            self.cursor.as_mut().unwrap().busy_on();
        } else {
            self.cursor.as_mut().unwrap().busy_off();
        }

        self.cur_point_area()
    }

    pub fn set_pending_popupmenu(&mut self, new_pending: PendingPopupMenu) -> RedrawMode {
        self.pending_popupmenu.update(new_pending);
        RedrawMode::Nothing
    }

    pub fn popupmenu_flush(&self, pending: PendingPopupMenu) {
        self.popup_menu.flush(
            pending,
            &self.nvim,
            &self.render_state,
            self.max_popup_width(),
        );
    }

    pub fn tabline_update(
        &mut self,
        selected: Tabpage,
        tabs: Vec<(Tabpage, Option<String>)>,
    ) -> RedrawMode {
        self.tabs.update_tabs(&self.nvim, selected, tabs);

        RedrawMode::Nothing
    }

    pub fn option_set(&mut self, name: String, val: Value) -> RedrawMode {
        match name.as_str() {
            "guifont" => self.set_font_from_value(val),
            _ => RedrawMode::Nothing,
        }
    }

    fn set_font_from_value(&mut self, val: Value) -> RedrawMode {
        if let Value::String(val) = val {
            if let Some(val) = val.into_str() {
                if !val.is_empty() {
                    let exists_fonts = self.render_state.borrow().font_ctx.font_families();
                    let fonts = split_at_comma(&val);
                    for font in &fonts {
                        let desc = FontDescription::from_string(font);
                        if desc.size() > 0
                            && exists_fonts.contains(&desc.family().unwrap_or_else(|| "".into()))
                        {
                            self.set_font_rpc(font);
                            return RedrawMode::All;
                        }
                    }

                    // font does not exists? set first one
                    if !fonts.is_empty() {
                        self.set_font_rpc(&fonts[0]);
                        return RedrawMode::All;
                    }
                }
            }
        }

        RedrawMode::Nothing
    }

    pub fn mode_info_set(
        &mut self,
        cursor_style_enabled: bool,
        mode_infos: Vec<HashMap<String, Value>>,
    ) -> RedrawMode {
        let mode_info_arr = mode_infos.iter().map(mode::ModeInfo::new).collect();

        match mode_info_arr {
            Ok(mode_info_arr) => {
                let mut render_state = self.render_state.borrow_mut();
                render_state
                    .mode
                    .set_info(cursor_style_enabled, mode_info_arr);
            }
            Err(err) => {
                error!("Error load mode info: {err}");
            }
        }

        RedrawMode::Nothing
    }

    pub fn cmdline_show(
        &mut self,
        content: Vec<(u64, String)>,
        pos: u64,
        firstc: String,
        prompt: String,
        indent: u64,
        level: u64,
    ) -> RedrawMode {
        {
            let cursor = self.grids.current().unwrap().cur_point();
            let render_state = self.render_state.borrow();
            let (x, y, width, height) = cursor.to_area(render_state.font_ctx.cell_metrics());
            let ctx = CmdLineContext {
                nvim: &self.nvim,
                content,
                pos,
                firstc,
                prompt,
                indent,
                level_idx: level,
                x,
                y,
                width,
                height,
                max_width: self.max_popup_width(),
            };

            self.cmd_line.show_level(&ctx);
        }

        self.on_busy(true)
    }

    pub fn cmdline_hide(&mut self, level: u64) -> RedrawMode {
        self.cmd_line.hide_level(level);
        self.on_busy(false)
    }

    pub fn cmdline_block_show(&mut self, content: Vec<Vec<(u64, String)>>) -> RedrawMode {
        let max_width = self.max_popup_width();
        self.cmd_line.show_block(&content, max_width);
        self.on_busy(true)
    }

    pub fn cmdline_block_append(&mut self, content: Vec<(u64, String)>) -> RedrawMode {
        self.cmd_line.block_append(&content);
        RedrawMode::Nothing
    }

    pub fn cmdline_block_hide(&mut self) -> RedrawMode {
        self.cmd_line.block_hide();
        self.on_busy(false)
    }

    pub fn cmdline_pos(&mut self, pos: u64, level: u64) -> RedrawMode {
        let render_state = self.render_state.borrow();
        self.cmd_line.pos(&render_state, pos, level);
        RedrawMode::Nothing
    }

    pub fn cmdline_special_char(&mut self, c: String, shift: bool, level: u64) -> RedrawMode {
        let render_state = self.render_state.borrow();
        self.cmd_line.special_char(&render_state, c, shift, level);
        RedrawMode::Nothing
    }

    pub fn wildmenu_show(&self, items: Vec<String>) -> RedrawMode {
        self.cmd_line
            .show_wildmenu(items, &self.render_state.borrow(), self.max_popup_width());
        RedrawMode::Nothing
    }

    pub fn wildmenu_hide(&self) -> RedrawMode {
        self.cmd_line.hide_wildmenu();
        RedrawMode::Nothing
    }

    pub fn wildmenu_select(&self, selected: i64) -> RedrawMode {
        self.cmd_line.wildmenu_select(selected);
        RedrawMode::Nothing
    }
}

impl CursorRedrawCb for State {
    fn queue_redraw_cursor(&mut self) {
        self.nvim_viewport.queue_draw();
    }
}
