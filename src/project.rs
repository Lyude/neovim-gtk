use std::cell::RefCell;
use std::cmp::Ordering::{Equal, Greater, Less};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use log::error;

use gtk::prelude::*;
use gtk::{
    CellRendererPixbuf, CellRendererText, CellRendererToggle, ListStore, MenuButton, Orientation,
    PolicyType, ScrolledWindow, TreeIter, TreeModel, TreeView, TreeViewColumn,
};

use serde::{Deserialize, Serialize};

use crate::nvim::{ErrorReport, NvimSession};
use crate::shell::Shell;
use crate::ui::UiMutex;
use nvim_rs::Value;

use htmlescape::encode_minimal;

const MAX_VISIBLE_ROWS: usize = 5;

const BOOKMARKED_PIXBUF: &str = "user-bookmarks";
const CURRENT_DIR_PIXBUF: &str = "folder";
const PLAIN_FILE_PIXBUF: &str = "text-x-generic";

enum ProjectViewColumns {
    Name,
    Path,
    Uri,
    Pixbuf,
    Project,
    ProjectStored,
}

const COLUMN_COUNT: usize = 6;
const COLUMN_TYPES: [glib::Type; COLUMN_COUNT] = [
    glib::Type::STRING,
    glib::Type::STRING,
    glib::Type::STRING,
    glib::Type::STRING,
    glib::Type::BOOL,
    glib::Type::BOOL,
];
const COLUMN_IDS: [u32; COLUMN_COUNT] = [
    ProjectViewColumns::Name as u32,
    ProjectViewColumns::Path as u32,
    ProjectViewColumns::Uri as u32,
    ProjectViewColumns::Pixbuf as u32,
    ProjectViewColumns::Project as u32,
    ProjectViewColumns::ProjectStored as u32,
];

pub struct Projects {
    shell: Rc<RefCell<Shell>>,
    open_btn: MenuButton,
    tree: TreeView,
    scroll: ScrolledWindow,
    store: Option<EntryStore>,
    name_renderer: CellRendererText,
    path_renderer: CellRendererText,
    toggle_renderer: CellRendererToggle,
}

impl Projects {
    pub fn new(shell: Rc<RefCell<Shell>>) -> Arc<UiMutex<Projects>> {
        let tree = gtk::TreeView::builder()
            .activate_on_single_click(true)
            .hover_selection(true)
            .enable_grid_lines(gtk::TreeViewGridLines::Horizontal)
            .build();
        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .has_frame(true)
            .child(&tree)
            .build();
        let vbox = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(5)
            .margin_top(5)
            .margin_bottom(5)
            .margin_start(5)
            .margin_end(5)
            .build();

        let popup = gtk::Popover::builder().child(&vbox).build();
        let open_btn = MenuButton::builder()
            .focusable(false)
            .focus_on_click(false)
            .sensitive(false)
            .label("Open")
            .direction(gtk::ArrowType::Down)
            .popover(&popup)
            .build();

        /* Make sure the child button isn't focusable either, without breaking keyboard focus on the
         * popup */
        open_btn
            .first_child()
            .unwrap()
            .downcast::<gtk::ToggleButton>()
            .unwrap()
            .set_focusable(false);

        let projects = Projects {
            shell: shell.clone(),
            open_btn,
            tree,
            scroll,
            store: None,
            name_renderer: CellRendererText::new(),
            path_renderer: CellRendererText::new(),
            toggle_renderer: CellRendererToggle::new(),
        };

        projects.setup_tree();

        let search_box = gtk::Entry::new();
        search_box
            .set_icon_from_icon_name(gtk::EntryIconPosition::Primary, Some("edit-find-symbolic"));

        vbox.append(&search_box);
        vbox.append(&projects.scroll);

        let open_btn = gtk::Button::with_label("Other Documents…");
        vbox.append(&open_btn);

        let projects = Arc::new(UiMutex::new(projects));
        let projects_ref = projects.borrow();

        search_box.connect_changed(clone!(projects => move |search_box| {
            let projects = projects.borrow();
            let list_store = projects.get_list_store();

            list_store.clear();
            if let Some(ref store) = projects.store {
                store.populate(&list_store, Some(&search_box.text()));
            }
        }));

        search_box.connect_activate(clone!(projects => move |_| {
            let model = projects.borrow().tree.model().unwrap();
            if let Some(iter) = model.iter_first() {
                let projects = projects.borrow();
                projects.open_uri(&model, &iter);
                projects.set_active(false);
            }
        }));

        projects_ref
            .tree
            .connect_row_activated(clone!(projects => move |tree, _, column| {
                // Don't activate if the user clicked the checkbox.
                let toggle_column = tree.column(2).unwrap();
                if column == Some(&toggle_column) {
                    return;
                }
                let selection = tree.selection();
                if let Some((model, iter)) = selection.selected() {
                    let projects = projects.borrow();
                    projects.open_uri(&model, &iter);
                    projects.set_active(false);
                }
            }));

        open_btn.connect_clicked(clone!(projects => move |_| {
            let projects = projects.borrow();
            projects.show_open_file_dlg();
            projects.set_active(false);
        }));

        let drawing_area = shell.borrow().state.borrow().nvim_viewport.clone();
        popup.connect_closed(clone!(projects => move |_| {
            projects.borrow_mut().clear();
            drawing_area.grab_focus();
        }));

        projects_ref
            .toggle_renderer
            .connect_toggled(clone!(projects => move |_, path| {
                projects.borrow_mut().toggle_stored(&path)
            }));

        projects_ref.tree.connect_map(clone!(projects => move |_| {
            projects.borrow_mut().before_show()
        }));

        drop(projects_ref);
        projects
    }

    fn toggle_stored(&mut self, path: &gtk::TreePath) {
        let list_store = self.get_list_store();
        if let Some(iter) = list_store.iter(path) {
            let value: bool = list_store.get(&iter, ProjectViewColumns::ProjectStored as i32);

            list_store.set_value(
                &iter,
                ProjectViewColumns::ProjectStored as u32,
                &ToValue::to_value(&!value),
            );

            let pixbuf = if value {
                CURRENT_DIR_PIXBUF
            } else {
                BOOKMARKED_PIXBUF
            };

            list_store.set_value(
                &iter,
                ProjectViewColumns::Pixbuf as u32,
                &ToValue::to_value(pixbuf),
            );

            let uri: String = list_store.get(&iter, ProjectViewColumns::Uri as i32);
            let store = self.store.as_mut().unwrap();
            if let Some(entry) = store.find_mut(&uri) {
                entry.stored = !value;
            }

            store.changed();
        }
    }

    fn open_uri(&self, model: &TreeModel, iter: &TreeIter) {
        let uri: String = model.get(iter, ProjectViewColumns::Uri as i32);
        let project: bool = model.get(iter, ProjectViewColumns::Project as i32);

        let shell = self.shell.borrow();
        if project {
            shell.cd(&uri);
        }
        shell.open_file(&uri);
    }

    fn get_list_store(&self) -> ListStore {
        self.tree.model().unwrap().downcast::<ListStore>().unwrap()
    }

    fn show_open_file_dlg(&self) {
        let window = self.open_btn.root().unwrap().downcast::<gtk::Window>().ok();
        let dlg = gtk::FileChooserDialog::new(
            Some("Open Document"),
            window.as_ref(),
            gtk::FileChooserAction::Open,
            &[
                ("_Open", gtk::ResponseType::Ok),
                ("_Cancel", gtk::ResponseType::Cancel),
            ],
        );

        let shell = self.shell.clone();
        dlg.run_async(move |dlg, response| {
            if response == gtk::ResponseType::Ok {
                if let Some(filename) = dlg
                    .file()
                    .and_then(|f| f.path())
                    .and_then(|f| f.to_str().map(|s| s.to_owned()))
                {
                    shell.borrow().open_file(&filename);
                }
            }
            dlg.close();
        });
    }

    pub fn before_show(&mut self) {
        self.load_oldfiles();
        self.resize_treeview();
    }

    pub fn show(&mut self) {
        self.before_show();
        self.set_active(true);
    }

    fn load_oldfiles(&mut self) {
        let shell_borrow = self.shell.borrow();
        let shell_state = shell_borrow.state.borrow_mut();

        let nvim = shell_state.nvim();
        if let Some(nvim) = nvim {
            let store = EntryStore::load(&nvim);
            store.populate(&self.get_list_store(), None);
            self.store = Some(store);
        }
    }

    pub fn clear(&mut self) {
        if let Some(s) = self.store.take() {
            s.save()
        };
        self.get_list_store().clear();
    }

    fn setup_tree(&self) {
        self.tree.set_model(Some(&ListStore::new(&COLUMN_TYPES)));
        self.tree.set_headers_visible(false);

        let image_column = TreeViewColumn::new();

        let icon_renderer = CellRendererPixbuf::new();
        icon_renderer.set_padding(5, 0);
        image_column.pack_start(&icon_renderer, true);

        image_column.add_attribute(
            &icon_renderer,
            "icon-name",
            ProjectViewColumns::Pixbuf as i32,
        );

        self.tree.append_column(&image_column);

        let text_column = TreeViewColumn::new();

        self.name_renderer.set_width_chars(45);
        self.path_renderer.set_width_chars(45);
        self.name_renderer
            .set_ellipsize(pango::EllipsizeMode::Middle);
        self.path_renderer
            .set_ellipsize(pango::EllipsizeMode::Start);
        self.name_renderer.set_padding(0, 5);
        self.path_renderer.set_padding(0, 5);

        text_column.pack_start(&self.name_renderer, true);
        text_column.pack_start(&self.path_renderer, true);

        text_column.add_attribute(&self.name_renderer, "text", ProjectViewColumns::Name as i32);
        text_column.add_attribute(
            &self.path_renderer,
            "markup",
            ProjectViewColumns::Path as i32,
        );

        let area = text_column
            .area()
            .unwrap()
            .downcast::<gtk::CellAreaBox>()
            .expect("Error build tree view");
        area.set_orientation(gtk::Orientation::Vertical);

        self.tree.append_column(&text_column);

        let toggle_column = TreeViewColumn::new();
        self.toggle_renderer.set_activatable(true);
        self.toggle_renderer.set_padding(10, 0);

        toggle_column.pack_start(&self.toggle_renderer, true);
        toggle_column.add_attribute(
            &self.toggle_renderer,
            "visible",
            ProjectViewColumns::Project as i32,
        );
        toggle_column.add_attribute(
            &self.toggle_renderer,
            "active",
            ProjectViewColumns::ProjectStored as i32,
        );

        self.tree.append_column(&toggle_column);
    }

    fn calc_treeview_height(&self) -> i32 {
        let (_, name_renderer_natural_size) = self.name_renderer.preferred_height(&self.tree);
        let (_, path_renderer_natural_size) = self.path_renderer.preferred_height(&self.tree);
        let (_, ypad) = self.name_renderer.padding();

        let row_height = name_renderer_natural_size + path_renderer_natural_size + ypad;

        row_height * MAX_VISIBLE_ROWS as i32
    }

    fn resize_treeview(&self) {
        let treeview_height = self.calc_treeview_height();
        let previous_height = self.scroll.max_content_height();

        // strange solution to make gtk assertions happy
        match previous_height.cmp(&treeview_height) {
            Less => {
                self.scroll.set_max_content_height(treeview_height);
                self.scroll.set_min_content_height(treeview_height);
            }
            Greater => {
                self.scroll.set_min_content_height(treeview_height);
                self.scroll.set_max_content_height(treeview_height);
            }
            Equal => (),
        }
    }

    pub fn open_btn(&self) -> &MenuButton {
        &self.open_btn
    }

    fn set_active(&self, active: bool) {
        /* We might be getting called from a signal handler, so open/close the projects menu with an
         * idle callback
         */
        let open_btn = self.open_btn.clone();
        glib::idle_add_local_once(move || {
            if active {
                open_btn.popup();
            } else {
                open_btn.popdown();
            }
        });
    }
}

fn list_old_files(nvim: &NvimSession) -> Vec<String> {
    let oldfiles_var = nvim.block_timeout(nvim.get_vvar("oldfiles"));

    match oldfiles_var {
        Ok(files) => {
            if let Some(files) = files.as_array() {
                files
                    .iter()
                    .map(Value::as_str)
                    .filter(Option::is_some)
                    .map(|path| path.unwrap().to_owned())
                    .filter(|path| !path.starts_with("term:"))
                    .collect()
            } else {
                vec![]
            }
        }
        err @ Err(_) => {
            err.report_err();
            vec![]
        }
    }
}

pub struct EntryStore {
    entries: Vec<Entry>,
    changed: bool,
}

impl EntryStore {
    pub fn find_mut(&mut self, uri: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.project && e.uri == uri)
    }

    pub fn load(nvim: &NvimSession) -> EntryStore {
        let mut entries = Vec::new();

        for project in ProjectSettings::load().projects {
            entries.push(project.to_entry());
        }

        match nvim.block_timeout(nvim.call_function("getcwd", vec![])) {
            Ok(pwd) => {
                if let Some(pwd) = pwd.as_str() {
                    if !entries.iter().any(|e| e.project && e.uri == pwd) {
                        entries.insert(0, Entry::new_current_project(pwd));
                    }
                } else {
                    error!("Error get current directory");
                }
            }
            err @ Err(_) => err.report_err(),
        }

        let old_files = list_old_files(nvim);
        entries.extend(old_files.iter().map(|p| Entry::new_from_path(p)));

        EntryStore {
            entries,
            changed: false,
        }
    }

    pub fn save(&self) {
        if self.changed {
            ProjectSettings::new(
                self.entries
                    .iter()
                    .filter(|e| e.project && e.stored)
                    .map(|p| p.to_entry_settings())
                    .collect(),
            )
            .save();
        }
    }

    pub fn populate(&self, list_store: &ListStore, filter: Option<&glib::GString>) {
        for file in &self.entries {
            if match filter.map(|f| f.to_uppercase()) {
                Some(ref filter) => {
                    file.file_name.to_uppercase().contains(filter)
                        || file.path.to_uppercase().contains(filter)
                }
                None => true,
            } {
                let files = file.to_values();
                list_store.insert_with_values(
                    None,
                    COLUMN_IDS
                        .iter()
                        .enumerate()
                        .map(|(i, id)| (*id, files[i]))
                        .collect::<Box<_>>()
                        .as_ref(),
                );
            }
        }
    }

    fn changed(&mut self) {
        self.changed = true;
    }
}

pub struct Entry {
    uri: String,
    path: String,
    file_name: String,
    name: String,
    pixbuf: &'static str,
    project: bool,
    stored: bool,
}

impl Entry {
    fn new_project(name: &str, uri: &str) -> Entry {
        let path = Path::new(uri);

        Entry {
            uri: uri.to_owned(),
            path: path
                .parent()
                .map(|s| format!("<small>{}</small>", encode_minimal(&s.to_string_lossy())))
                .unwrap_or_else(|| "".to_owned()),
            file_name: encode_minimal(name),
            name: name.to_owned(),
            pixbuf: BOOKMARKED_PIXBUF,
            project: true,
            stored: true,
        }
    }

    fn new_current_project(uri: &str) -> Entry {
        let path = Path::new(uri);
        let name = path
            .file_name()
            .map(|f| f.to_string_lossy().as_ref().to_owned())
            .unwrap_or_else(|| path.to_string_lossy().as_ref().to_owned());

        Entry {
            uri: uri.to_owned(),
            path: path
                .parent()
                .map(|s| format!("<small>{}</small>", encode_minimal(&s.to_string_lossy())))
                .unwrap_or_else(|| "".to_owned()),
            file_name: encode_minimal(&name),
            name,
            pixbuf: CURRENT_DIR_PIXBUF,
            project: true,
            stored: false,
        }
    }

    fn new_from_path(uri: &str) -> Entry {
        let path = Path::new(uri);
        let name = path
            .file_name()
            .map(|f| f.to_string_lossy().as_ref().to_owned())
            .unwrap_or_else(|| "<empty>".to_owned());

        Entry {
            uri: uri.to_owned(),
            path: path
                .parent()
                .map(|s| format!("<small>{}</small>", encode_minimal(&s.to_string_lossy())))
                .unwrap_or_else(|| "".to_owned()),
            file_name: encode_minimal(&name),
            name,
            pixbuf: PLAIN_FILE_PIXBUF,
            project: false,
            stored: false,
        }
    }

    fn to_values(&self) -> Box<[&dyn glib::ToValue]> {
        Box::new([
            &self.file_name,
            &self.path,
            &self.uri,
            &self.pixbuf,
            &self.project,
            &self.stored,
        ])
    }

    fn to_entry_settings(&self) -> ProjectEntrySettings {
        ProjectEntrySettings::new(&self.name, &self.uri)
    }
}

// ----- Store / Load settings
//
use crate::settings::SettingsLoader;

#[derive(Serialize, Deserialize, Default)]
struct ProjectSettings {
    projects: Vec<ProjectEntrySettings>,
}

#[derive(Serialize, Deserialize)]
struct ProjectEntrySettings {
    name: String,
    path: String,
}

impl ProjectEntrySettings {
    fn new(name: &str, path: &str) -> ProjectEntrySettings {
        ProjectEntrySettings {
            name: name.to_owned(),
            path: path.to_owned(),
        }
    }

    fn to_entry(&self) -> Entry {
        Entry::new_project(&self.name, &self.path)
    }
}

impl SettingsLoader for ProjectSettings {
    const SETTINGS_FILE: &'static str = "projects.toml";

    fn from_str(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| format!("{e}"))
    }
}

impl ProjectSettings {
    fn new(projects: Vec<ProjectEntrySettings>) -> ProjectSettings {
        ProjectSettings { projects }
    }
}
