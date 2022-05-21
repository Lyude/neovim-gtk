#![windows_subsystem = "windows"]

extern crate dirs as env_dirs;
extern crate glib_sys as glib_ffi;
extern crate gobject_sys as gobject_ffi;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

mod sys;

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
mod plug_manager;
mod popup_menu;
mod project;
mod render;
mod settings;
mod shell;
mod shell_dlg;
mod subscriptions;
mod tabline;
mod drawing_area;

use gio::prelude::*;
use gio::ApplicationCommandLine;
use std::cell::RefCell;
use std::io::Read;
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use unix_daemonize::{daemonize_redirect, ChdirMode};

use crate::ui::Ui;
use crate::shell::ShellOptions;

use clap::{App, Arg, ArgMatches};

include!(concat!(env!("OUT_DIR"), "/version.rs"));

fn main() {
    env_logger::init();

    let matches = App::new("NeovimGtk")
        .version(GIT_BUILD_VERSION.unwrap_or(env!("CARGO_PKG_VERSION")))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(misc::about_comments().as_str())
        .arg(Arg::with_name("no-fork")
             .long("no-fork")
             .help("Prevent detach from console"))
        .arg(Arg::with_name("disable-win-restore")
             .long("disable-win-restore")
             .help("Don't restore window size at start"))
        .arg(Arg::with_name("timeout")
             .long("timeout")
             .default_value("10")
             .help("Wait timeout in seconds. If nvim does not response in given time NvimGtk stops")
             .takes_value(true))
        .arg(Arg::with_name("cterm-colors")
             .long("cterm-colors")
             .help("Use ctermfg/ctermbg instead of guifg/guibg"))
        .arg(Arg::with_name("diff-mode")
             .help("Open two or more files in diff mode")
             .short("d"))
        .arg(Arg::with_name("files")
             .help("Files to open")
             .multiple(true))
        .arg(Arg::with_name("nvim-bin-path")
             .long("nvim-bin-path")
             .help("Path to nvim binary")
             .takes_value(true))
        .arg(Arg::with_name("post-config-cmds")
             .help("Execute <cmd> after config and first file")
             .value_name("cmd")
             .short("c")
             .multiple(true)
             .takes_value(true)
             .number_of_values(1))
        .arg(Arg::with_name("nvim-args")
             .help("Args will be passed to nvim")
             .last(true)
             .multiple(true))
        .get_matches();

    let input_data = RefCell::new(read_piped_input());

    // Additional argument parsing
    if matches.is_present("diff-mode") {
        if let Some(files) = matches.values_of("files") {
            if files.len() < 2 {
                clap::Error::with_description("Diff mode (-d) requires 2 or more files",
                                              clap::ErrorKind::TooFewValues).exit();
            }
        } else {
            clap::Error::with_description("Diff mode (-d) specified but no files provided. 2 or \
                                           more files must be provided",
                                          clap::ErrorKind::MissingRequiredArgument).exit();
        }
    }

    #[cfg(unix)]
    {
        // fork to background by default
        if !matches.is_present("no-fork") {
            daemonize_redirect(
                None::<String>,
                None::<String>,
                ChdirMode::NoChdir,
            )
            .unwrap();
        }
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
    let app_cmdline_copy = app_cmdline.clone();
    let matches_copy = matches.clone();
    app.connect_command_line(move |app, cmdline| {
        app_cmdline_copy.lock().unwrap().replace(cmdline.clone());
        let input_data = input_data
            .replace(None)
            .filter(|_input| !matches_copy.is_present("files"));

        match input_data {
            Some(input_data) => activate(
                app,
                &matches_copy,
                Some(input_data),
                app_cmdline_copy.clone(),
            ),
            None => {
                let files = matches_copy
                    .values_of("files")
                    .unwrap_or_default()
                    .map(str::to_owned)
                    .collect::<Vec<String>>();
                open(app, files.into_boxed_slice(), &matches_copy, app_cmdline_copy.clone());
            }
        }
        0
    });

    // Setup our global style provider
    let css_provider = gtk::CssProvider::new();
    css_provider.load_from_data(include_bytes!("style.css"));
    gtk::StyleContext::add_provider_for_display(
        gdk::Display::default().as_ref().expect("Cannot find default GDK Display"),
        &css_provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let app_ref = app.clone();
    let matches_copy = matches.clone();
    let app_cmdline_copy = Arc::clone(&app_cmdline);
    let new_window_action = gio::SimpleAction::new("new-window", None);
    new_window_action.connect_activate(move |_, _| {
        activate(&app_ref, &matches_copy, None, app_cmdline_copy.clone())
    });
    app.add_action(&new_window_action);

    gtk::Window::set_default_icon_name("org.daa.NeovimGtk");

    app.run();
    let lock = app_cmdline.lock().unwrap();
    std::process::exit(lock.as_ref().unwrap().exit_status());
}

fn open(
    app: &gtk::Application,
    files: Box<[String]>,
    matches: &ArgMatches,
    app_cmdline: Arc<Mutex<Option<ApplicationCommandLine>>>,
) {

    let mut ui = Ui::new(
        ShellOptions::new(matches, None),
        files,
    );

    ui.init(app, !matches.is_present("disable-win-restore"), app_cmdline);
}

fn activate(
    app: &gtk::Application,
    matches: &ArgMatches,
    input_data: Option<String>,
    app_cmdline: Arc<Mutex<Option<ApplicationCommandLine>>>,
) {
    let mut ui = Ui::new(ShellOptions::new(matches, input_data), Box::new([]));

    ui.init(app, !matches.is_present("disable-win-restore"), app_cmdline);
}

fn read_piped_input() -> Option<String> {
    if atty::isnt(atty::Stream::Stdin) {
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
