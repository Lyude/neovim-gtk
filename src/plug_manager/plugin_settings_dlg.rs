use gtk;
use gtk::prelude::*;

use super::store;

pub struct Builder<'a> {
    title: &'a str,
}

impl<'a> Builder<'a> {
    pub fn new(title: &'a str) -> Self {
        Builder { title }
    }

    pub fn show<F: IsA<gtk::Window>>(&self, parent: &F) -> Option<store::PlugInfo> {
        let dlg = gtk::Dialog::with_buttons(
            Some(self.title),
            Some(parent),
            gtk::DialogFlags::USE_HEADER_BAR | gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[
                ("Cancel", gtk::ResponseType::Cancel),
                ("Ok", gtk::ResponseType::Ok),
            ],
        );

        let content = dlg.content_area();
        let border = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .build();

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);

        let path = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(5)
            .margin_start(5)
            .margin_bottom(5)
            .margin_top(5)
            .margin_end(5)
            .build();
        let path_lbl = gtk::Label::new(Some("Repo"));
        let path_e = gtk::Entry::new();
        path_e.set_placeholder_text(Some("user_name/repo_name"));

        path.pack_start(&path_lbl, true, true, 0);
        path.pack_end(&path_e, false, true, 0);

        list.add(&path);

        let name = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(5)
            .margin_start(5)
            .margin_end(5)
            .margin_top(5)
            .margin_bottom(5)
            .build();
        let name_lbl = gtk::Label::new(Some("Name"));
        let name_e = gtk::Entry::new();

        name.pack_start(&name_lbl, true, true, 0);
        name.pack_end(&name_e, false, true, 0);

        list.add(&name);

        border.pack_start(&list, true, true, 0);
        content.add(&border);
        content.show_all();

        path_e.connect_changed(clone!(name_e => move |p| {
            if let Some(name) = extract_name(p.text().as_str()) {
                name_e.set_text(&name);
            }
        }));

        let res = if dlg.run() == gtk::ResponseType::Ok {
            let path = path_e.text().to_string();
            let name = name_e.text();

            let name = if name.trim().is_empty() {
                match extract_name(&path) {
                    Some(name) => name,
                    None => path.clone(),
                }
            } else {
                name.to_string()
            };

            Some(store::PlugInfo::new(name, path))
        } else {
            None
        };

        dlg.close();

        res
    }
}

fn extract_name(path: &str) -> Option<String> {
    if let Some(idx) = path.rfind(|c| c == '/' || c == '\\') {
        if idx < path.len() - 1 {
            let path = path.trim_end_matches(".git");
            Some(path[idx + 1..].to_owned())
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_name() {
        assert_eq!(
            Some("plugin_name".to_owned()),
            extract_name("http://github.com/somebody/plugin_name.git")
        );
    }
}
