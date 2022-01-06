use std::cell::{Ref, RefCell, RefMut};
use std::convert::TryFrom;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::{env, thread};

use gio::prelude::*;
use gio::{ApplicationCommandLine, Menu, MenuItem, SimpleAction};
use glib::variant::FromVariant;
use gtk::{
    self,
    prelude::*,
    AboutDialog, ApplicationWindow, Button, HeaderBar, Orientation, Paned, Inhibit
};

use toml;

use crate::file_browser::FileBrowserWidget;
use crate::highlight::BackgroundState;
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
    window_state: ToplevelState,
    title_label: Option<gtk::Label>,
}

impl Components {
    fn new() -> Components {
        Components {
            window: None,
            window_state: ToplevelState::load(),
            title_label: None,
        }
    }

    pub fn close_window(&self) {
        self.window.as_ref().unwrap().close();
    }

    pub fn window(&self) -> &ApplicationWindow {
        self.window.as_ref().unwrap()
    }

    pub fn set_title(&self, title: &str) {
        self.window.as_ref().unwrap().set_title(Some(title));
        if let Some(ref title_label) = self.title_label {
            title_label.set_label(title);
        }
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

        let projects = Projects::new(shell.clone());

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

    pub fn init(&mut self, app: &gtk::Application, restore_win_state: bool, app_cmdline: Arc<Mutex<Option<ApplicationCommandLine>>>) {
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

            self.shell.borrow_mut().init(app_cmdline);

            comps.window = Some(window.clone());

            let prefer_dark_theme = env::var("NVIM_GTK_PREFER_DARK_THEME")
                .map(|opt| opt.trim() == "1")
                .unwrap_or(false);
            if prefer_dark_theme {
                window
                    .settings()
                    .set_property("gtk-application-prefer-dark-theme", true);
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

        window.connect_default_width_notify(clone!(main, comps_ref => move |window| {
            gtk_window_resize(
                window,
                &mut *comps_ref.borrow_mut(),
                &main,
                gtk::Orientation::Horizontal,
            );
        }));
        window.connect_default_height_notify(clone!(main, comps_ref => move |window| {
            gtk_window_resize(
                window,
                &mut *comps_ref.borrow_mut(),
                &main,
                gtk::Orientation::Vertical,
            );
        }));

        window.connect_maximized_notify(clone!(comps_ref => move |window| {
            comps_ref.borrow_mut().window_state.is_maximized = window.is_maximized();
        }));

        window.connect_destroy(clone!(comps_ref => move |_| {
            comps_ref.borrow().window_state.save();
        }));

        let shell = self.shell.borrow();
        let file_browser = self.file_browser.borrow();
        main.set_start_child(&**file_browser);
        main.set_end_child(&**shell);
        window.set_child(Some(&main));

        window.show();

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

        let update_background = shell.state.borrow().subscribe(
            SubscriptionKey::with_pattern("OptionSet", "background"),
            &["&background"],
            clone!(shell_ref => move |args| set_background(&*shell_ref, args)),
        );

        shell.state.borrow().subscribe(
            SubscriptionKey::from("VimLeave"),
            &["v:exiting ? v:exiting : 0"],
            clone!(shell_ref => move |args| set_exit_status(&*shell_ref, args)),
        );

        window.connect_close_request(clone!(comps_ref, shell_ref => move |_| {
            gtk_close_request(&*comps_ref, &*shell_ref)
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
                &update_background,
                post_config_cmds.as_ref(),
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
        update_background: &SubscriptionHandle,
        post_config_cmds: &[String],
        mode: StartMode,
    ) {
        plug_manager
            .borrow_mut()
            .init_nvim_client(shell.nvim_clone());
        file_browser.borrow_mut().init();
        shell.set_autocmds();
        shell.run_now(&update_title);
        shell.run_now(&update_completeopt);
        shell.run_now(&update_background);
        if let Some(ref update_subtitle) = update_subtitle {
            shell.run_now(&update_subtitle);
        }

        let mut commands = Vec::<String>::new();
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
            NvimCommand::ShowGtkInspector => {
                comps.borrow().window.as_ref().unwrap().emit_enable_debugging(false);
            }
            NvimCommand::ToggleSidebar => {
                let action = sidebar_action.borrow();
                let state = !bool::from_variant(&action.state().unwrap()).unwrap();
                action.change_state(&state.to_variant());
            }
            NvimCommand::Transparency(background_alpha, filled_alpha) => {
                let comps = comps.borrow();
                let window = comps.window.as_ref().unwrap();

                let display = window.display();
                if display.is_composited() {
                    /* TODO: This relied on gtk_set_app_paintable, but that has been removed in GTK4
                     * with no direct replacement. This may still work anyway though since we can
                     * probably just rely on the compositor to handle partially transparent
                     * surfaces. Confirm that's the case before removing this comment.
                     */
                    shell.set_transparency(background_alpha, filled_alpha);
                    // TODO: We probably don't need this anymore, confirm
                    //window.set_app_paintable(enabled);
                } else {
                    warn!("Screen is not composited");
                }
            }
            NvimCommand::PreferDarkTheme(prefer_dark_theme) => {
                let comps = comps.borrow();
                let window = comps.window.as_ref().unwrap();

                window
                    .settings()
                    .set_property("gtk-application-prefer-dark-theme", prefer_dark_theme);
            }
        }
    }

    fn create_header_bar(
        &self,
        app: &gtk::Application
    ) -> (SubscriptionHandle, Box<HeaderBarButtons>) {
        let header_bar_title = gtk::Label::builder()
            .css_classes(vec!["title".to_string()])
            .vexpand(true)
            .yalign(1.0)
            .build();
        let header_bar_subtitle = gtk::Label::builder()
            .css_classes(vec!["subtitle".to_string()])
            .vexpand(true)
            .yalign(0.0)
            .build();
        let header_bar_box = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        header_bar_box.append(&header_bar_title);
        header_bar_box.append(&header_bar_subtitle);
        let header_bar = HeaderBar::builder()
            .title_widget(&header_bar_box)
            .focusable(false)
            .build();

        let mut comps = self.comps.borrow_mut();
        comps.title_label = Some(header_bar_title);

        let window = comps.window.as_ref().unwrap();

        let projects = self.projects.borrow();
        let open_btn = projects.open_btn();
        header_bar.pack_start(open_btn);

        let new_tab_btn = Button::from_icon_name("tab-new-symbolic");
        let shell_ref = Rc::clone(&self.shell);
        new_tab_btn.connect_clicked(move |_| shell_ref.borrow_mut().new_tab());
        new_tab_btn.set_focusable(false);
        new_tab_btn.set_tooltip_text(Some("Open a new tab"));
        new_tab_btn.set_sensitive(false);
        header_bar.pack_start(&new_tab_btn);

        let primary_menu_btn = self.create_primary_menu_btn(app, &window);
        primary_menu_btn.set_sensitive(false);
        header_bar.pack_end(&primary_menu_btn);

        let paste_btn = Button::from_icon_name("edit-paste-symbolic");
        let shell = self.shell.clone();
        paste_btn.connect_clicked(move |_| shell.borrow_mut().edit_paste());
        paste_btn.set_focusable(false);
        paste_btn.set_tooltip_text(Some("Paste from clipboard"));
        paste_btn.set_sensitive(false);
        header_bar.pack_end(&paste_btn);

        let save_btn = Button::with_label("Save All");
        let shell = self.shell.clone();
        save_btn.connect_clicked(move |_| shell.borrow_mut().edit_save_all());
        save_btn.set_focusable(false);
        save_btn.set_sensitive(false);
        header_bar.pack_end(&save_btn);

        window.set_titlebar(Some(&header_bar));

        let shell = self.shell.borrow();

        let update_subtitle = shell.state.borrow().subscribe(
            SubscriptionKey::from("DirChanged"),
            &["getcwd()"],
            move |args| header_bar_subtitle.set_label(&*args[0]),
        );

        (
            update_subtitle,
            Box::new(HeaderBarButtons::new(
                open_btn.clone(),
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
        let btn = gtk::MenuButton::builder()
            .focusable(false)
            .focus_on_click(false)
            .icon_name("open-menu-symbolic")
            .build();

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
    about.set_program_name(Some("NeovimGtk"));
    about.set_version(Some(crate::GIT_BUILD_VERSION.unwrap_or(env!("CARGO_PKG_VERSION"))));
    about.set_logo_icon_name(Some("org.daa.NeovimGtk"));
    about.set_authors(env!("CARGO_PKG_AUTHORS").split(":").collect::<Vec<_>>().as_slice());
    about.set_comments(Some(misc::about_comments().as_str()));

    // TODO: Figure out if we need to rework this to GTK4, since GtkAboutDialog longer derives
    // from GtkDialog
    //about.connect_response(|about, _| about.close());
    about.show();
}

fn gtk_close_request(comps: &UiMutex<Components>, shell: &RefCell<Shell>) -> Inhibit {
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

fn gtk_window_resize(
    app_window: &gtk::ApplicationWindow,
    comps: &mut Components,
    main: &Paned,
    orientation: gtk::Orientation,
) {
    if !app_window.is_maximized() {
        match orientation {
            gtk::Orientation::Horizontal =>
                comps.window_state.current_width = app_window.size(gtk::Orientation::Horizontal),
            gtk::Orientation::Vertical =>
                comps.window_state.current_height = app_window.size(gtk::Orientation::Vertical),
            _ => unreachable!(),
        }
    }
    if comps.window_state.show_sidebar {
        comps.window_state.sidebar_width = main.position();
    }
}

fn set_completeopts(shell: &RefCell<Shell>, args: Vec<String>) {
    let options = &args[0];

    shell.borrow().set_completeopts(options);
}

fn set_background(shell: &RefCell<Shell>, args: Vec<String>) {
    let background = match args[0].as_str() {
        "light" => BackgroundState::Light,
        "dark" => BackgroundState::Dark,
        val => panic!("Unexpected 'background' value received: {}", val),
    };

    let state = &shell.borrow().state;
    state.borrow().set_background(background);

    // Neovim won't send us a redraw to update the default colors on the screen, so do it ourselves
    glib::idle_add_once(clone!(state => move || state.borrow_mut().on_redraw(&RepaintMode::All)));
}

fn update_window_title(comps: &Arc<UiMutex<Components>>, args: Vec<String>) {
    let comps_ref = comps.clone();
    let comps = comps_ref.borrow();

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

    comps.set_title(filename);
}

fn set_exit_status(shell: &RefCell<Shell>, args: Vec<String>) {
    let status = args[0].parse().unwrap();
    shell.borrow().set_exit_status(status);
}

#[derive(Serialize, Deserialize)]
struct ToplevelState {
    current_width: i32,
    current_height: i32,
    is_maximized: bool,
    show_sidebar: bool,
    sidebar_width: i32,
}

impl Default for ToplevelState {
    fn default() -> Self {
        ToplevelState {
            current_width: DEFAULT_WIDTH,
            current_height: DEFAULT_HEIGHT,
            is_maximized: false,
            show_sidebar: false,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

impl SettingsLoader for ToplevelState {
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
