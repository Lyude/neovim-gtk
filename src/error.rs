use std::ops::Deref;

use log::error;

use html_escape::encode_text_minimal;

use gtk::prelude::*;

use crate::shell;

pub struct ErrorArea {
    base: gtk::Box,
    label: gtk::Label,
}

impl ErrorArea {
    pub fn new() -> Self {
        let base = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .vexpand(true)
            .hexpand(true)
            .build();

        let label = gtk::Label::builder()
            .wrap(true)
            .selectable(true)
            .hexpand(true)
            .vexpand(true)
            .build();

        let error_image = gtk::Image::from_icon_name("dialog-error");
        error_image.set_icon_size(gtk::IconSize::Large);
        error_image.set_halign(gtk::Align::End);
        error_image.set_valign(gtk::Align::Center);
        error_image.set_hexpand(true);
        error_image.set_vexpand(true);

        base.append(&error_image);
        base.append(&label);

        ErrorArea { base, label }
    }

    pub fn show_nvim_init_error(&self, err: &str) {
        error!("Can't initialize nvim: {}", err);
        self.label.set_markup(&format!(
            "<big>Can't initialize nvim:</big>\n\
             <span foreground=\"red\"><i>{}</i></span>\n\n\
             <big>Possible error reasons:</big>\n\
             &#9679; Not supported nvim version (minimum supported version is <b>{}</b>)\n\
             &#9679; Error in configuration file (init.vim or ginit.vim)",
            encode_text_minimal(err),
            shell::MINIMUM_SUPPORTED_NVIM_VERSION
        ));
        self.base.show();
    }

    pub fn show_nvim_start_error(&self, err: &str, cmd: &str) {
        error!("Can't start nvim: {}\nCommand line: {}", err, cmd);
        self.label.set_markup(&format!(
            "<big>Can't start nvim instance:</big>\n\
             <i>{}</i>\n\
             <span foreground=\"red\"><i>{}</i></span>\n\n\
             <big>Possible error reasons:</big>\n\
             &#9679; Not supported nvim version (minimum supported version is <b>{}</b>)\n\
             &#9679; Error in configuration file (init.vim or ginit.vim)\n\
             &#9679; Wrong nvim binary path \
             (right path can be passed with <i>--nvim-bin-path=path_here</i>)",
            encode_text_minimal(cmd),
            encode_text_minimal(err),
            shell::MINIMUM_SUPPORTED_NVIM_VERSION
        ));
        self.base.show();
    }

    pub fn show_nvim_tcp_connect_error(&self, err: &str, addr: &str) {
        error!("Can't connect to nvim on TCP address {}: {}\n", addr, err);
        self.label.set_markup(&format!(
            "<big>Can't connect to nvim instance on TCP address {}:</big>\n\
             <i>{}</i>\n\
             <big>Possible error reasons:</big>\n\
             &#9679; Not supported nvim version (minimum supported version is <b>{}</b>)\n\
             &#9679; Error in configuration file (init.vim or ginit.vim)\n\
             &#9679; Invalid TCP address",
            encode_text_minimal(addr),
            encode_text_minimal(err),
            shell::MINIMUM_SUPPORTED_NVIM_VERSION
        ));
        self.base.show();
    }

    #[cfg(unix)]
    pub fn show_nvim_unix_connect_error(&self, err: &str, addr: &str) {
        error!("Can't connect to nvim on Unix pipe {}: {}\n", addr, err);
        self.label.set_markup(&format!(
            "<big>Can't connect to nvim instance on Unix pipe {}:</big>\n\
             <i>{}</i>\n\
             <big>Possible error reasons:</big>\n\
             &#9679; Not supported nvim version (minimum supported version is <b>{}</b>)\n\
             &#9679; Error in configuration file (init.vim or ginit.vim)\n\
             &#9679; Invalid Unix pipe",
            encode_text_minimal(addr),
            encode_text_minimal(err),
            shell::MINIMUM_SUPPORTED_NVIM_VERSION
        ));
        self.base.show();
    }
}

impl Deref for ErrorArea {
    type Target = gtk::Box;

    fn deref(&self) -> &gtk::Box {
        &self.base
    }
}
