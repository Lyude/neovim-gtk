use std::cell::{Ref, RefCell, RefMut};
use std::convert::TryFrom;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::{env, thread};

use gdk;
use gio::prelude::*;
use gio::{Menu, MenuItem, SimpleAction};
use glib::variant::FromVariant;
use gtk::{
    self,
    prelude::*,
    AboutDialog, ApplicationWindow, Button, HeaderBar, Orientation, Paned,
};

use toml;

use crate::file_browser::FileBrowserWidget;
use crate::misc;
use crate::nvim::*;
use crate::plug_manager;
use crate::project::Projects;
use crate::settings::{Settings, SettingsLoader};
use crate::shell::{self, Shell, ShellOptions, HeaderBarButtons, StartMode};
use crate::shell_dlg;
use crate::subscriptions::{SubscriptionHandle, SubscriptionKey};

macro_rules! clone {
    (@param _) => ( _ );
    (@param $x:ident) => ( $x );
    ($($n:ident),+ => move || $body:expr) => (
        {
            $( let $n = $n.clone(); )+
                move || $body
        }
    );
    ($($n:ident),+ => move |$($p:tt),+| $body:expr) => (
        {
            $( let $n = $n.clone(); )+
                move |$(clone!(@param $p),)+| $body
        }
    );
}

const DEFAULT_WIDTH: i32 = 800;
const DEFAULT_HEIGHT: i32 = 600;
const DEFAULT_SIDEBAR_WIDTH: i32 = 200;

pub struct Ui {
    open_paths: Box<[String]>,
    initialized: bool,
    comps: Arc<UiMutex<Components>>,
    settings: Rc<RefCell<Settings>>,
    shell: Rc<RefCell<Shell>>,
    projects: Arc<UiMutex<Projects>>,
    plug_manager: Arc<UiMutex<plug_manager::Manager>>,
    file_browser: Arc<UiMutex<FileBrowserWidget>>,
}

pub struct Components {
    window: Option<ApplicationWindow>,
    window_state: WindowState,
    open_btn: Button,
}

impl Components {
    fn new() -> Components {
        let open_btn = Button::new();
        let open_btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        open_btn_box.pack_start(&gtk::Label::new(Some("Open")), false, false, 3);
        open_btn_box.pack_start(
            &gtk::Image::from_icon_name(Some("pan-down-symbolic"), gtk::IconSize::Menu),
            false,
            false,
            3,
        );
        open_btn.add(&open_btn_box);
        open_btn.set_can_focus(false);
        open_btn.set_sensitive(false);
        Components {
            open_btn,
            window: None,
            window_state: WindowState::load(),
        }
    }

    pub fn close_window(&self) {
        self.window.as_ref().unwrap().close();
    }

    pub fn window(&self) -> &ApplicationWindow {
        self.window.as_ref().unwrap()
    }
}

impl Ui {
    pub fn new(options: ShellOptions, open_paths: Box<[String]>) -> Ui {
        let plug_manager = plug_manager::Manager::new();

        let plug_manager = Arc::new(UiMutex::new(plug_manager));
        let comps = Arc::new(UiMutex::new(Components::new()));
        let settings = Rc::new(RefCell::new(Settings::new()));
        let shell = Rc::new(RefCell::new(Shell::new(settings.clone(), options)));
        let file_browser = Arc::new(UiMutex::new(FileBrowserWidget::new(&shell.borrow().state)));
        settings.borrow_mut().set_shell(Rc::downgrade(&shell));

        let projects = Projects::new(&comps.borrow().open_btn, shell.clone());

        Ui {
            initialized: false,
            comps,
            shell,
            settings,
            projects,
            plug_manager,
            file_browser,
            open_paths,
        }
    }

    pub fn init(&mut self, app: &gtk::Application, restore_win_state: bool) {
        if self.initialized {
            return;
        }
        self.initialized = true;

        let mut settings = self.settings.borrow_mut();
        settings.init();

        let window = ApplicationWindow::new(app);

        let main = Paned::new(Orientation::Horizontal);

        let comps_ref = &self.comps;
        let shell_ref = &self.shell;
        let file_browser_ref = &self.file_browser;

        {
            // initialize window from comps
            // borrowing of comps must be leaved
            // for event processing
            let mut comps = comps_ref.borrow_mut();

            self.shell.borrow_mut().init();

            comps.window = Some(window.clone());

            let prefer_dark_theme = env::var("NVIM_GTK_PREFER_DARK_THEME")
                .map(|opt| opt.trim() == "1")
                .unwrap_or(false);
            if prefer_dark_theme {
                if let Some(settings) = window.settings() {
                    settings.set_property("gtk-application-prefer-dark-theme", true).unwrap();
                }
            }

            if restore_win_state {
                if comps.window_state.is_maximized {
                    window.maximize();
                }

                window.set_default_size(
                    comps.window_state.current_width,
                    comps.window_state.current_height,
                );

                main.set_position(comps.window_state.sidebar_width);
            } else {
                window.set_default_size(DEFAULT_WIDTH, DEFAULT_HEIGHT);
                main.set_position(DEFAULT_SIDEBAR_WIDTH);
            }
        }

        // Client side decorations including the toolbar are disabled via NVIM_GTK_NO_HEADERBAR=1
        let use_header_bar = env::var("NVIM_GTK_NO_HEADERBAR")
            .map(|opt| opt.trim() != "1")
            .unwrap_or(true);

        let disable_window_decoration = env::var("NVIM_GTK_NO_WINDOW_DECORATION")
            .map(|opt| opt.trim() == "1")
            .unwrap_or(false);

        if disable_window_decoration {
            window.set_decorated(false);
        }

        let (update_subtitle, header_bar) = if use_header_bar {
            let (subscription, header_bar) = self.create_header_bar(app);
            (Some(subscription), Some(header_bar))
        } else {
            (None, None)
        };

        let show_sidebar_action =
            SimpleAction::new_stateful("show-sidebar", None, &false.to_variant());
        show_sidebar_action.connect_change_state(
            clone!(file_browser_ref, comps_ref => move |action, value| {
                if let Some(value) = value {
                    action.set_state(value);
                    let is_active = value.get::<bool>().unwrap();
                    file_browser_ref.borrow().set_visible(is_active);
                    comps_ref.borrow_mut().window_state.show_sidebar = is_active;
                }
            })
        );
        app.add_action(&show_sidebar_action);

        window.connect_size_allocate(clone!(main, comps_ref => move |window, _| {
            gtk_window_size_allocate(
                window,
                &mut *comps_ref.borrow_mut(),
                &main,
            );
        }));

        window.connect_window_state_event(clone!(comps_ref => move |_, event| {
            gtk_window_state_event(event, &mut *comps_ref.borrow_mut());
            Inhibit(false)
        }));

        window.connect_destroy(clone!(comps_ref => move |_| {
            comps_ref.borrow().window_state.save();
        }));

        let shell = self.shell.borrow();
        let file_browser = self.file_browser.borrow();
        main.pack1(&**file_browser, false, false);
        main.pack2(&**shell, true, false);

        window.add(&main);

        window.show_all();

        if restore_win_state {
            // Hide sidebar, if it wasn't shown last time.
            // Has to be done after show_all(), so it won't be shown again.
            let show_sidebar = comps_ref.borrow().window_state.show_sidebar;
            show_sidebar_action.change_state(&show_sidebar.to_variant());
        }

        let update_title = shell.state.borrow().subscribe(
            SubscriptionKey::from("BufEnter,DirChanged"),
            &["expand('%:p')", "getcwd()"],
            clone!(comps_ref => move |args| update_window_title(&comps_ref, args)),
        );

        let update_completeopt = shell.state.borrow().subscribe(
            SubscriptionKey::with_pattern("OptionSet", "completeopt"),
            &["&completeopt"],
            clone!(shell_ref => move |args| set_completeopts(&*shell_ref, args)),
        );

        let update_size = shell.state.borrow().subscribe(
            SubscriptionKey::from("VimResized"),
            &["&lines", "&columns"],
            clone!(comps_ref, shell_ref => move |args| update_window_size(&*comps_ref, &*shell_ref, args)),
        );

        window.connect_delete_event(clone!(comps_ref, shell_ref => move |_, _| {
            gtk_delete(&*comps_ref, &*shell_ref)
        }));

        shell.grab_focus();

        shell.set_detach_cb(Some(clone!(comps_ref => move || {
            glib::idle_add_once(clone!(comps_ref => move || comps_ref.borrow().close_window()));
        })));

        let state_ref = self.shell.borrow().state.clone();
        let plug_manager_ref = self.plug_manager.clone();
        let files_list = self.open_paths.clone();

        let (post_config_cmds, mode) = {
            let state_ref = state_ref.borrow();
            let mut options = state_ref.options.borrow_mut();

            (options.post_config_cmds(), options.mode)
        };

        state_ref.borrow().set_action_widgets(header_bar, file_browser_ref.borrow().clone());

        shell.set_nvim_started_cb(Some(clone!(file_browser_ref => move || {
            Ui::nvim_started(
                &state_ref.borrow(),
                &plug_manager_ref,
                &file_browser_ref,
                &files_list,
                &update_title,
                &update_subtitle,
                &update_completeopt,
                &update_size,
                &post_config_cmds,
                mode,
            );
        })));

        let sidebar_action = UiMutex::new(show_sidebar_action);
        let comps_ref = comps_ref.clone();
        let projects = self.projects.clone();
        shell.set_nvim_command_cb(Some(
            move |shell: &mut shell::State, command: NvimCommand| {
                Ui::nvim_command(shell, command, &sidebar_action, &projects, &comps_ref);
            },
        ));
    }

    fn nvim_started(
        shell: &shell::State,
        plug_manager: &UiMutex<plug_manager::Manager>,
        file_browser: &UiMutex<FileBrowserWidget>,
        files_list: &Box<[String]>,
        update_title: &SubscriptionHandle,
        update_subtitle: &Option<SubscriptionHandle>,
        update_completeopt: &SubscriptionHandle,
        update_size: &SubscriptionHandle,
        post_config_cmds: &Option<Box<[String]>>,
        mode: StartMode,
    ) {
        plug_manager
            .borrow_mut()
            .init_nvim_client(shell.nvim_clone());
        file_browser.borrow_mut().init();
        shell.set_autocmds();
        shell.run_now(&update_title);
        shell.run_now(&update_completeopt);
        shell.run_now(&update_size);
        if let Some(ref update_subtitle) = update_subtitle {
            shell.run_now(&update_subtitle);
        }

        let mut commands = Vec::<String>::new();
        let post_config_cmds = post_config_cmds.as_ref().map(|c| c.as_ref()).unwrap_or(&[]);

        if !files_list.is_empty() {
            if mode == StartMode::Normal {
                commands.reserve(1 + post_config_cmds.len());
                commands.push(format!(
                    r"try|ar {}|cat /^Vim(\a\+):E325:/|endt",
                    files_list
                    .iter()
                    .map(|f| misc::escape_filename(f))
                    .collect::<Box<_>>()
                    .join(" ")
                ));
            } else {
                commands.reserve(files_list.len() + post_config_cmds.len());
                commands.push(format!(
                    r"try|e {}|cat /^Vim(\a\+):E325:/|endt|difft",
                    misc::escape_filename(&files_list[0])
                ));
                for file in &files_list[1..] {
                    commands.push(format!(
                        r"try|vs {}|cat /^Vim(\a\+):E325:/|endt|difft",
                        misc::escape_filename(file)
                    ));
                }
            }
        }

        commands.extend(
            post_config_cmds.iter().map(|cmd| format!(r#"exec "{}""#, misc::viml_escape(cmd)))
        );
        debug!("{:?}", commands);

        // open files as last command
        // because it can generate user query
        let action_widgets = shell.action_widgets();
        if commands.is_empty() {
            action_widgets.borrow().as_ref().unwrap().set_enabled(true);
            return;
        }

        let commands = commands.join("|");
        let nvim = shell.nvim().unwrap();
        nvim.clone().spawn(async move {
            let res = nvim.command(&commands).await;

            glib::idle_add_once(move || {
                action_widgets.borrow().as_ref().unwrap().set_enabled(true)
            });

            if let Err(e) = res {
                if let Ok(e) = NormalError::try_from(&*e) {
                    if e == NormalError::KeyboardInterrupt {
                        nvim.shutdown().await;
                        return;
                    } else if !e.has_code(325) {
                        // Filter out errors we get if the user is presented with a prompt
                        e.print(&nvim).await;
                    }
                } else {
                    e.print();
                }
            }
        });
    }

    fn nvim_command(
        shell: &mut shell::State,
        command: NvimCommand,
        sidebar_action: &UiMutex<SimpleAction>,
        projects: &Arc<UiMutex<Projects>>,
        comps: &UiMutex<Components>,
    ) {
        match command {
            NvimCommand::ShowProjectView => {
                glib::idle_add_once(clone!(projects => move || projects.borrow_mut().show()));
            }
            NvimCommand::ToggleSidebar => {
                let action = sidebar_action.borrow();
                let state = !bool::from_variant(&action.state().unwrap()).unwrap();
                action.change_state(&state.to_variant());
            }
            NvimCommand::Transparency(background_alpha, filled_alpha) => {
                let comps = comps.borrow();
                let window = comps.window.as_ref().unwrap();

                let screen = window.screen().unwrap();
                if screen.is_composited() {
                    let enabled = shell.set_transparency(background_alpha, filled_alpha);
                    window.set_app_paintable(enabled);
                } else {
                    warn!("Screen is not composited");
                }
            }
            NvimCommand::PreferDarkTheme(prefer_dark_theme) => {
                let comps = comps.borrow();
                let window = comps.window.as_ref().unwrap();

                if let Some(settings) = window.settings() {
                    settings
                        .set_property("gtk-application-prefer-dark-theme", prefer_dark_theme)
                        .unwrap();
                }
            }
        }
    }

    fn create_header_bar(
        &self,
        app: &gtk::Application
    ) -> (SubscriptionHandle, Box<HeaderBarButtons>) {
        let header_bar = HeaderBar::new();
        let comps = self.comps.borrow();
        let window = comps.window.as_ref().unwrap();

        let projects = self.projects.clone();
        header_bar.pack_start(&comps.open_btn);
        comps
            .open_btn
            .connect_clicked(move |_| projects.borrow_mut().show());

        let new_tab_btn =
            Button::from_icon_name(Some("tab-new-symbolic"), gtk::IconSize::SmallToolbar);
        let shell_ref = Rc::clone(&self.shell);
        new_tab_btn.connect_clicked(move |_| shell_ref.borrow_mut().new_tab());
        new_tab_btn.set_can_focus(false);
        new_tab_btn.set_tooltip_text(Some("Open a new tab"));
        new_tab_btn.set_sensitive(false);
        header_bar.pack_start(&new_tab_btn);

        let primary_menu_btn = self.create_primary_menu_btn(app, &window);
        primary_menu_btn.set_sensitive(false);
        header_bar.pack_end(&primary_menu_btn);

        let paste_btn =
            Button::from_icon_name(Some("edit-paste-symbolic"), gtk::IconSize::SmallToolbar);
        let shell = self.shell.clone();
        paste_btn.connect_clicked(move |_| shell.borrow_mut().edit_paste());
        paste_btn.set_can_focus(false);
        paste_btn.set_tooltip_text(Some("Paste from clipboard"));
        paste_btn.set_sensitive(false);
        header_bar.pack_end(&paste_btn);

        let save_btn = Button::with_label("Save All");
        let shell = self.shell.clone();
        save_btn.connect_clicked(move |_| shell.borrow_mut().edit_save_all());
        save_btn.set_can_focus(false);
        save_btn.set_sensitive(false);
        header_bar.pack_end(&save_btn);

        header_bar.set_show_close_button(true);

        window.set_titlebar(Some(&header_bar));

        let shell = self.shell.borrow();

        let update_subtitle = shell.state.borrow().subscribe(
            SubscriptionKey::from("DirChanged"),
            &["getcwd()"],
            move |args| {
                header_bar.set_subtitle(Some(&*args[0]));
            },
        );

        (
            update_subtitle,
            Box::new(HeaderBarButtons::new(
                comps.open_btn.clone(),
                new_tab_btn,
                paste_btn,
                save_btn,
                primary_menu_btn,
            )),
        )
    }

    fn create_primary_menu_btn(
        &self,
        app: &gtk::Application,
        window: &gtk::ApplicationWindow,
    ) -> gtk::MenuButton {
        let plug_manager = self.plug_manager.clone();
        let btn = gtk::MenuButton::new();
        btn.set_can_focus(false);
        btn.set_image(Some(&gtk::Image::from_icon_name(
            Some("open-menu-symbolic"),
            gtk::IconSize::SmallToolbar,
        )));

        // note actions created in application menu
        let menu = Menu::new();

        let section = Menu::new();
        section.append_item(&MenuItem::new(Some("New Window"), Some("app.new-window")));
        menu.append_section(None, &section);

        let section = Menu::new();
        section.append_item(&MenuItem::new(Some("Sidebar"), Some("app.show-sidebar")));
        menu.append_section(None, &section);

        let section = Menu::new();
        section.append_item(&MenuItem::new(Some("Plugins"), Some("app.Plugins")));
        section.append_item(&MenuItem::new(Some("About"), Some("app.HelpAbout")));
        menu.append_section(None, &section);

        menu.freeze();

        let plugs_action = SimpleAction::new("Plugins", None);
        plugs_action.connect_activate(
            clone!(window => move |_, _| plug_manager::Ui::new(&plug_manager).show(&window)),
        );

        let about_action = SimpleAction::new("HelpAbout", None);
        about_action.connect_activate(clone!(window => move |_, _| on_help_about(&window)));
        about_action.set_enabled(true);

        app.add_action(&about_action);
        app.add_action(&plugs_action);

        btn.set_menu_model(Some(&menu));
        btn
    }
}

fn on_help_about(window: &gtk::ApplicationWindow) {
    let about = AboutDialog::new();
    about.set_transient_for(Some(window));
    about.set_program_name("NeovimGtk");
    about.set_version(Some(
        crate::GIT_BUILD_VERSION.unwrap_or(env!("CARGO_PKG_VERSION")),
    ));
    about.set_logo_icon_name(Some("org.daa.NeovimGtk"));
    about.set_authors(&[env!("CARGO_PKG_AUTHORS")]);
    about.set_comments(Some(misc::about_comments().as_str()));

    about.connect_response(|about, _| about.close());
    about.show();
}

fn gtk_delete(comps: &UiMutex<Components>, shell: &RefCell<Shell>) -> Inhibit {
    if !shell.borrow().is_nvim_initialized() {
        return Inhibit(false);
    }

    let nvim = shell.borrow().state.borrow().nvim_clone();
    Inhibit(if shell_dlg::can_close_window(comps, shell, &nvim) {
        let comps = comps.borrow();
        comps.close_window();
        shell.borrow_mut().detach_ui();
        false
    } else {
        true
    })
}

fn gtk_window_size_allocate(
    app_window: &gtk::ApplicationWindow,
    comps: &mut Components,
    main: &Paned,
) {
    if !app_window.is_maximized() {
        let (current_width, current_height) = app_window.size();
        comps.window_state.current_width = current_width;
        comps.window_state.current_height = current_height;
    }
    if comps.window_state.show_sidebar {
        comps.window_state.sidebar_width = main.position();
    }
}

fn gtk_window_state_event(event: &gdk::EventWindowState, comps: &mut Components) {
    comps.window_state.is_maximized = event
        .new_window_state()
        .contains(gdk::WindowState::MAXIMIZED);
}

fn set_completeopts(shell: &RefCell<Shell>, args: Vec<String>) {
    let options = &args[0];

    shell.borrow().set_completeopts(options);
}

fn update_window_title(comps: &Arc<UiMutex<Components>>, args: Vec<String>) {
    let comps_ref = comps.clone();
    let comps = comps_ref.borrow();
    let window = comps.window.as_ref().unwrap();

    let file_path = &args[0];
    let dir = Path::new(&args[1]);
    let filename = if file_path.is_empty() {
        "[No Name]"
    } else if let Some(rel_path) = Path::new(&file_path)
        .strip_prefix(&dir)
        .ok()
        .and_then(|p| p.to_str())
    {
        rel_path
    } else {
        &file_path
    };

    window.set_title(filename);
}

fn update_window_size(comps: &UiMutex<Components>, shell: &RefCell<Shell>, args: Vec<String>) {
    let lines = &args[0];
    let cols = &args[1];

    if let (Ok(lines), Ok(cols)) = (lines.parse::<usize>(), cols.parse::<usize>()) {
        let state_ref = shell.borrow().state.clone();
        let state = state_ref.borrow();

        if state.should_resize(cols, lines) {
            let (width, height) = state.calc_window_size(cols, lines);

            let comps_ref = comps.clone();
            let comps = comps_ref.borrow();
            let window = comps.window.as_ref().unwrap();

            window.resize(width as i32, height as i32);
        }
    }
}

#[derive(Serialize, Deserialize)]
struct WindowState {
    current_width: i32,
    current_height: i32,
    is_maximized: bool,
    show_sidebar: bool,
    sidebar_width: i32,
}

impl Default for WindowState {
    fn default() -> Self {
        WindowState {
            current_width: DEFAULT_WIDTH,
            current_height: DEFAULT_HEIGHT,
            is_maximized: false,
            show_sidebar: false,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

impl SettingsLoader for WindowState {
    const SETTINGS_FILE: &'static str = "window.toml";

    fn from_str(s: &str) -> Result<Self, String> {
        toml::from_str(&s).map_err(|e| format!("{}", e))
    }
}

pub struct UiMutex<T: ?Sized> {
    thread: thread::ThreadId,
    data: RefCell<T>,
}

unsafe impl<T: ?Sized> Send for UiMutex<T> {}
unsafe impl<T: ?Sized> Sync for UiMutex<T> {}

impl<T> UiMutex<T> {
    pub fn new(t: T) -> UiMutex<T> {
        UiMutex {
            thread: thread::current().id(),
            data: RefCell::new(t),
        }
    }
}

impl<T> UiMutex<T> {
    pub fn replace(&self, t: T) -> T {
        self.assert_ui_thread();
        self.data.replace(t)
    }
}

impl<T: ?Sized> UiMutex<T> {
    pub fn borrow(&self) -> Ref<T> {
        self.assert_ui_thread();
        self.data.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<T> {
        self.assert_ui_thread();
        self.data.borrow_mut()
    }

    #[inline]
    fn assert_ui_thread(&self) {
        if thread::current().id() != self.thread {
            panic!("Can access to UI only from main thread");
        }
    }
}
