#![windows_subsystem = "windows"]
#![allow(clippy::new_without_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::comparison_chain)]
#![allow(clippy::await_holding_refcell_ref)]

mod color;
mod dirs;
mod mode;
mod nvim_config;
mod ui_model;
mod value;
#[macro_use]
mod ui;
mod cmd_line;
mod cursor;
mod error;
mod file_browser;
mod grid;
mod highlight;
mod input;
mod misc;
mod nvim;
mod nvim_viewport;
mod plug_manager;
mod popup_menu;
mod project;
mod render;
mod settings;
mod shell;
mod shell_dlg;
mod subscriptions;
mod tabline;

use log::error;

use gio::prelude::*;
use gio::ApplicationCommandLine;

use std::net::SocketAddr;
use std::{
    cell::RefCell,
    convert::*,
    io::Read,
    mem,
    num::ParseIntError,
    ops::Deref,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(unix)]
use fork::{daemon, Fork};

use crate::ui::Ui;

use clap::*;

use is_terminal::IsTerminal;

include!(concat!(env!("OUT_DIR"), "/version.rs"));

#[derive(Debug, Copy, Clone)]
pub struct TimeoutDuration(Option<Duration>);

impl TimeoutDuration {
    fn new(secs: u64) -> Self {
        Self(if secs == 0 {
            None
        } else {
            Some(Duration::from_secs(secs))
        })
    }
}

impl Deref for TimeoutDuration {
    type Target = Option<Duration>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromStr for TimeoutDuration {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s.parse()?))
    }
}

impl std::fmt::Display for TimeoutDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(duration) = self.0 {
            duration.as_secs().fmt(f)
        } else {
            f.write_str("0")
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "neovim-gtk",
    version = GIT_BUILD_VERSION.unwrap_or(env!("CARGO_PKG_VERSION")),
    author = env!("CARGO_PKG_AUTHORS"),
    about = misc::about_comments(),
)]
pub struct Args {
    /// Execute <CMD> after config and first file (same as 'nvim -c <CMD>')
    ///
    /// May be specified more then once.
    #[arg(short = 'c', value_name = "CMD")]
    post_config_cmds: Vec<String>,

    /// Open two or more files in diff mode (same as 'nvim -d ...')
    #[arg(short)]
    pub diff_mode: bool,

    /// Don't detach from the console (!= Windows only)
    #[arg(long)]
    pub no_fork: bool,

    /// Don't restore any previously saved window state
    ///
    /// This includes:
    ///
    /// * The size of the window
    ///
    /// * Whether or not the window was maximized
    ///
    /// * The visibility of the sidebar (will be shown by default, use --hide-sidebar to disable)
    #[arg(long)]
    pub disable_win_restore: bool,

    /// Hide the sidebar by default on start
    #[arg(long)]
    pub hide_sidebar: bool,

    /// RPC timeout (0 for none)
    ///
    /// If nvim doesn't respond to an RPC call unexpectedly within <SECONDS>, we give up.
    #[arg(long, default_value_t = TimeoutDuration::new(10), value_name = "SECONDS")]
    pub timeout: TimeoutDuration,

    #[arg(long)]
    /// Use ctermfg/ctermbg instead of guifg/guibg
    pub cterm_colors: bool,

    #[arg(long)]
    /// Path to the nvim binary
    pub nvim_bin_path: Option<String>,

    #[arg(long)]
    /// Nvim server to connect to (currently TCP only)
    pub server: Option<SocketAddr>,

    #[arg()]
    /// Files to open
    pub files: Vec<String>,

    /// Arguments that will be passed to nvim (see more with '--help' before using!)
    ///
    /// Note that due to current limitations, the arguments that may be passed through this are
    /// limited to arguments that:
    ///
    /// * Don't cause a user prompt, e.g. anything that makes nvim go "Hit ENTER...", either
    ///   directly or indirectly
    ///
    /// * Don't interfere with stdio output (since we start nvim with --embed, we need stdio
    ///   reserved for RPC)
    ///
    /// * Are not filenames
    ///
    /// Trying to pass arguments which match any of the above criteria may result in hangs. As such,
    /// the equivalent neovim-gtk arguments should be used instead of being passed via this option
    /// whenever possible.
    #[arg(last = true)]
    pub nvim_args: Vec<String>,

    /// Input data from stdin
    /// TODO: Get rid of this (#57)
    #[arg(skip)]
    input_data: Option<String>,
}

impl Args {
    /// Steal the post config commands, since they're only needed once
    pub fn post_config_cmds(&mut self) -> Vec<String> {
        mem::take(&mut self.post_config_cmds)
    }

    /// Steal the input data, since it's only used once
    pub fn input_data(&mut self) -> Self {
        let ret = self.clone();
        self.input_data = None;
        ret
    }
}

fn main() {
    env_logger::init();

    let mut command = Args::command();
    let args = Args::from_arg_matches(&command.get_matches_mut()).unwrap_or_else(|e| e.exit());

    let input_data = RefCell::new(read_piped_input());

    // Additional argument parsing
    if args.diff_mode {
        if args.files.is_empty() {
            command.error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "Diff mode (-d) specified but no files provided. 2 or more files must be provided",
            ).exit();
        } else if args.files.len() < 2 {
            command
                .error(
                    clap::error::ErrorKind::TooFewValues,
                    "Diff mode (-d) requires 2 or more files",
                )
                .exit();
        }
    }

    command.build();

    // fork to background by default
    #[cfg(unix)]
    if !args.no_fork {
        match daemon(true, true) {
            Ok(Fork::Parent(_)) => return,
            Ok(Fork::Child) => (),
            Err(code) => panic!("Failed to fork, got {}", code),
        };
    }

    // Debugging mode for CLI test runs
    #[cfg(debug_assertions)]
    if std::env::var("NVIM_GTK_CLI_TEST_MODE") == Ok("1".to_string()) {
        println!("Testing the CLI");
        if !args.post_config_cmds.is_empty() {
            println!(
                "Commands passed: [{}]",
                args.post_config_cmds
                    .iter()
                    .map(|c| format!("'{c}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return;
    }

    gtk::init().expect("Failed to initialize GTK+");

    let app_flags = gio::ApplicationFlags::HANDLES_OPEN
        | gio::ApplicationFlags::HANDLES_COMMAND_LINE
        | gio::ApplicationFlags::NON_UNIQUE;

    glib::set_program_name(Some("NeovimGtk"));

    let app = if cfg!(debug_assertions) {
        gtk::Application::new(Some("org.daa.NeovimGtkDebug"), app_flags)
    } else {
        gtk::Application::new(Some("org.daa.NeovimGtk"), app_flags)
    };

    let app_cmdline = Arc::new(Mutex::new(None));
    app.connect_command_line(
        glib::clone!(@strong app_cmdline, @strong args => move |app, cmdline| {
            app_cmdline.lock().unwrap().replace(cmdline.clone());
            let input_data = input_data
                .replace(None)
                .filter(|_input| !args.files.is_empty());

            match input_data {
                Some(_) => {
                    let mut args = args.clone();
                    args.input_data = input_data;
                    activate(
                        app,
                        &args,
                        app_cmdline.clone(),
                    );
                }
                None => {
                    let files = args.files.iter().cloned().collect::<Box<[String]>>();
                    open(app, files, &args, app_cmdline.clone());
                }
            }
            0
        }),
    );

    // Setup our global style provider
    let css_provider = gtk::CssProvider::new();
    css_provider.load_from_data(include_str!("style.css"));
    gtk::StyleContext::add_provider_for_display(
        gdk::Display::default()
            .as_ref()
            .expect("Cannot find default GDK Display"),
        &css_provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let new_window_action = gio::SimpleAction::new("new-window", None);
    new_window_action.connect_activate(glib::clone!(
        @strong app, @strong args, @strong app_cmdline => move |_, _| {
            activate(&app, &args, app_cmdline.clone())
        }
    ));
    app.add_action(&new_window_action);

    gtk::Window::set_default_icon_name("org.daa.NeovimGtk");

    app.run();
    let lock = app_cmdline.lock().unwrap();
    std::process::exit(lock.as_ref().unwrap().exit_status());
}

fn open(
    app: &gtk::Application,
    files: Box<[String]>,
    args: &Args,
    app_cmdline: Arc<Mutex<Option<ApplicationCommandLine>>>,
) {
    let mut ui = Ui::new(args.clone(), files);

    ui.init(app, args, app_cmdline);
}

fn activate(
    app: &gtk::Application,
    args: &Args,
    app_cmdline: Arc<Mutex<Option<ApplicationCommandLine>>>,
) {
    let mut ui = Ui::new(args.clone(), Box::new([]));

    ui.init(app, args, app_cmdline);
}

fn read_piped_input() -> Option<String> {
    if !std::io::stdout().is_terminal() {
        let mut buf = String::new();
        match std::io::stdin().read_to_string(&mut buf) {
            Ok(size) if size > 0 => Some(buf),
            Ok(_) => None,
            Err(err) => {
                error!("Error read stdin {}", err);
                None
            }
        }
    } else {
        None
    }
}
