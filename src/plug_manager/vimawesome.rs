use std::io;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::thread;

use serde::Deserialize;
use serde_json;

use glib;
use gtk;
use gtk::prelude::*;

use super::store::PlugInfo;

pub fn call<F>(query: Option<String>, cb: F)
where
    F: FnOnce(io::Result<DescriptionList>) + Send + 'static,
{
    thread::spawn(move || {
        let mut result = Some(request(query.as_ref().map(|s| s.as_ref())));
        let mut cb = Some(cb);

        glib::idle_add_once(move || cb.take().unwrap()(result.take().unwrap()))
    });
}

fn request(query: Option<&str>) -> io::Result<DescriptionList> {
    let child = Command::new("curl")
        .arg("-s")
        .arg(format!(
            "https://vimawesome.com/api/plugins?query={}&page=1",
            query.unwrap_or("")
        ))
        .stdout(Stdio::piped())
        .spawn()?;

    let out = child.wait_with_output()?;

    if out.status.success() {
        if out.stdout.is_empty() {
            Ok(DescriptionList::empty())
        } else {
            let description_list: DescriptionList = serde_json::from_slice(&out.stdout)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(description_list)
        }
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "curl exit with error:\n{}",
                match out.status.code() {
                    Some(code) => format!("Exited with status code: {}", code),
                    None => "Process terminated by signal".to_owned(),
                }
            ),
        ))
    }
}

pub fn build_result_panel<F: Fn(PlugInfo) + 'static>(
    list: &DescriptionList,
    add_cb: F,
) -> gtk::ScrolledWindow {
    let panel = gtk::ListBox::new();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&panel)
        .vexpand(true)
        .build();

    let cb_ref = Rc::new(add_cb);
    for plug in list.plugins.iter() {
        let row = create_plug_row(plug, cb_ref.clone());

        panel.append(&row);
    }

    scroll.show();
    scroll
}

fn create_plug_row<F: Fn(PlugInfo) + 'static>(
    plug: &Description,
    add_cb: Rc<F>,
) -> gtk::ListBoxRow {
    let row_container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .margin_start(5)
        .margin_bottom(5)
        .margin_top(5)
        .margin_end(5)
        .build();

    #[rustfmt::skip]
    let row = gtk::ListBoxRow::builder()
        .child(&row_container)
        .build();

    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let label_box = create_plug_label(plug);

    let button_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    button_box.set_halign(gtk::Align::End);

    let add_btn = gtk::Button::with_label("Install");
    button_box.append(&add_btn);

    row_container.append(&hbox);
    hbox.append(&label_box);
    hbox.append(&button_box);

    add_btn.connect_clicked(clone!(plug => move |btn| {
        if let Some(ref github_url) = plug.github_url {
            btn.set_sensitive(false);
            add_cb(PlugInfo::new(plug.name.clone(), github_url.clone()));
        }
    }));

    row
}

fn create_plug_label(plug: &Description) -> gtk::Box {
    let label_box = gtk::Box::new(gtk::Orientation::Vertical, 5);

    let name_lbl = gtk::Label::new(None);
    name_lbl.set_markup(&format!(
        "<b>{}</b> by {}",
        plug.name,
        plug.author
            .as_ref()
            .map(|s| s.as_ref())
            .unwrap_or("unknown",)
    ));
    name_lbl.set_halign(gtk::Align::Start);
    let url_lbl = gtk::Label::new(None);
    if let Some(url) = plug.github_url.as_ref() {
        url_lbl.set_markup(&format!("<a href=\"{}\">{}</a>", url, url));
    }
    url_lbl.set_halign(gtk::Align::Start);

    label_box.append(&name_lbl);
    label_box.append(&url_lbl);
    label_box
}

#[derive(Deserialize, Debug)]
pub struct DescriptionList {
    pub plugins: Box<[Description]>,
}

impl DescriptionList {
    fn empty() -> DescriptionList {
        DescriptionList {
            plugins: Box::new([]),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Description {
    pub name: String,
    pub github_url: Option<String>,
    pub author: Option<String>,
    pub github_stars: Option<i64>,
}
