use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use log::error;

use crate::ui::UiMutex;

use gtk::{prelude::*, Inhibit};

use super::manager;
use super::plugin_settings_dlg;
use super::store::{PlugInfo, Store};
use super::vimawesome;
use crate::nvim_config::NvimConfig;

pub struct Ui<'a> {
    manager: &'a Arc<UiMutex<manager::Manager>>,
}

impl<'a> Ui<'a> {
    pub fn new(manager: &'a Arc<UiMutex<manager::Manager>>) -> Ui<'a> {
        manager.borrow_mut().reload_store();

        Ui { manager }
    }

    pub fn show<T: IsA<gtk::Window>>(&mut self, parent: &T) {
        let dlg = gtk::Dialog::with_buttons(
            Some("Plug"),
            Some(parent),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[
                ("Cancel", gtk::ResponseType::Cancel),
                ("Ok", gtk::ResponseType::Ok),
            ],
        );

        dlg.set_default_size(800, 600);
        dlg.set_modal(true);
        let content = dlg.content_area();
        content.set_vexpand(true);

        let header_bar_title = gtk::Label::builder()
            .label("Plug")
            .css_classes(vec!["title".to_string()])
            .build();
        let header_bar = gtk::HeaderBar::builder()
            .title_widget(&header_bar_title)
            .build();

        let add_plug_btn = gtk::Button::with_label("Add..");
        add_plug_btn.style_context().add_class("suggested-action");
        header_bar.pack_end(&add_plug_btn);

        let enable_swc = gtk::Switch::new();
        enable_swc.set_valign(gtk::Align::Center);

        header_bar.pack_end(&enable_swc);

        dlg.set_titlebar(Some(&header_bar));

        let pages = SettingsPages::new(
            clone!(add_plug_btn => move |row_name| if row_name == "plugins" {
                add_plug_btn.show();
            } else {
                add_plug_btn.hide();
            }),
        );

        enable_swc.set_state(self.manager.borrow().store.is_enabled());

        let plugins = gtk::Box::new(gtk::Orientation::Vertical, 3);
        let plugs_panel = self.fill_plugin_list(&plugins, &self.manager.borrow().store);

        add_vimawesome_tab(&pages, self.manager, &plugs_panel);

        let plugins_lbl = gtk::Label::new(Some("Plugins"));
        pages.add_page(&plugins_lbl, &plugins, "plugins");

        add_help_tab(
            &pages,
            &format!(
                "NeovimGtk plugin manager is a GUI for vim-plug.\n\
            It can load plugins from vim-plug configuration if vim-plug sarted and NeovimGtk manager settings is empty.\n\
            When enabled it generate and load vim-plug as simple vim file at startup before init.vim is processed.\n\
            So <b>after</b> enabling this manager <b>you must disable vim-plug</b> configuration in init.vim.\n\
            This manager currently only manage vim-plug configuration and do not any actions on plugin management.\n\
            So you must call all vim-plug (PlugInstall, PlugUpdate, PlugClean) commands manually.\n\
            Current configuration source is <b>{}</b>",
                match self.manager.borrow().plug_manage_state {
                    manager::PlugManageState::NvimGtk => "NeovimGtk config file",
                    manager::PlugManageState::VimPlug => "loaded from vim-plug",
                    manager::PlugManageState::Unknown => "Unknown",
                }
            ),
        );

        let manager_ref = self.manager.clone();
        enable_swc.connect_state_set(move |_, state| {
            manager_ref.borrow_mut().store.set_enabled(state);
            Inhibit(false)
        });

        let manager_ref = self.manager.clone();
        add_plug_btn.connect_clicked(clone!(dlg => move |_| {
            show_add_plug_dlg(&dlg, &manager_ref, &plugs_panel);
        }));

        content.append(&*pages);

        let manager = self.manager.clone();
        dlg.run_async(move |dlg, id| {
            if id == gtk::ResponseType::Ok {
                let mut manager = manager.borrow_mut();
                manager.clear_removed();
                manager.save();
                if let Some(path) = NvimConfig::new(manager.generate_config()).generate_config() {
                    manager.vim_plug.reload(path.to_str().unwrap());
                }
            }

            dlg.close();
        });
    }

    fn fill_plugin_list(&self, panel: &gtk::Box, store: &Store) -> gtk::ListBox {
        let plugs_panel = gtk::ListBox::new();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&plugs_panel)
            .vexpand(true)
            .build();

        for (idx, plug_info) in store.get_plugs().iter().enumerate() {
            let row = create_plug_row(idx, plug_info, self.manager);

            plugs_panel.append(&row);
        }

        panel.append(&scroll);
        panel.append(&create_up_down_btns(&plugs_panel, self.manager));

        plugs_panel
    }
}

fn create_up_down_btns(
    plugs_panel: &gtk::ListBox,
    manager: &Arc<UiMutex<manager::Manager>>,
) -> gtk::Box {
    let buttons_panel = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let up_btn = gtk::Button::from_icon_name("go-up-symbolic");
    let down_btn = gtk::Button::from_icon_name("go-down-symbolic");

    up_btn.connect_clicked(clone!(plugs_panel, manager => move |_| {
        if let Some(row) = plugs_panel.selected_row() {
            let idx = row.index();
            if idx > 0 {
                plugs_panel.unselect_row(&row);
                plugs_panel.remove(&row);
                plugs_panel.insert(&row, idx - 1);
                plugs_panel.select_row(Some(&row));
                manager.borrow_mut().move_item(idx as usize, -1);
            }
        }
    }));

    down_btn.connect_clicked(clone!(plugs_panel, manager => move |_| {
        if let Some(row) = plugs_panel.selected_row() {
            let idx = row.index();
            let mut manager = manager.borrow_mut();
            if idx >= 0 && idx < manager.store.plugs_count() as i32 {
                plugs_panel.unselect_row(&row);
                plugs_panel.remove(&row);
                plugs_panel.insert(&row, idx + 1);
                plugs_panel.select_row(Some(&row));
                manager.move_item(idx as usize, 1);
            }
        }
    }));

    buttons_panel.append(&up_btn);
    buttons_panel.append(&down_btn);
    buttons_panel.set_halign(gtk::Align::Center);

    buttons_panel
}

fn populate_get_plugins(
    query: Option<String>,
    get_plugins: &gtk::Box,
    manager: Arc<UiMutex<manager::Manager>>,
    plugs_panel: gtk::ListBox,
) {
    let plugs_panel = Arc::new(UiMutex::new(plugs_panel));
    let get_plugins = Arc::new(UiMutex::new(get_plugins.clone()));
    vimawesome::call(query, move |res| {
        let panel = get_plugins.borrow();
        while let Some(ref child) = panel.first_child() {
            panel.remove(child);
        }
        match res {
            Ok(list) => {
                let result = vimawesome::build_result_panel(&list, move |new_plug| {
                    glib::MainContext::new().spawn_local(
                        clone!(manager, plugs_panel => async move {
                            add_plugin(&manager, &plugs_panel.borrow(), new_plug).await;
                        }),
                    );
                });
                panel.append(&result);
            }
            Err(e) => {
                panel.append(&gtk::Label::new(Some(format!("{e}").as_str())));
                error!("{}", e)
            }
        }
    });
}

fn create_plug_row(
    plug_idx: usize,
    plug_info: &PlugInfo,
    manager: &Arc<UiMutex<manager::Manager>>,
) -> gtk::ListBoxRow {
    let row_container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .margin_top(5)
        .margin_bottom(5)
        .margin_start(5)
        .margin_end(5)
        .build();
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let label_box = create_plug_label(plug_info);

    let button_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    button_box.set_halign(gtk::Align::End);

    let exists_button_box = gtk::Box::new(gtk::Orientation::Horizontal, 5);

    let remove_btn = gtk::Button::with_label("Remove");
    exists_button_box.append(&remove_btn);

    let undo_btn = gtk::Button::with_label("Undo");

    row_container.append(&hbox);
    hbox.append(&label_box);
    button_box.append(&exists_button_box);
    hbox.append(&button_box);

    #[rustfmt::skip]
    let row = gtk::ListBoxRow::builder()
        .child(&row_container)
        .build();

    remove_btn.connect_clicked(
        clone!(manager, label_box, button_box, exists_button_box, undo_btn => move |_| {
            label_box.set_sensitive(false);
            button_box.remove(&exists_button_box);
            button_box.append(&undo_btn);
            manager.borrow_mut().store.remove_plug(plug_idx);
        }),
    );

    undo_btn.connect_clicked(
        clone!(manager, label_box, button_box, exists_button_box, undo_btn => move |_| {
            label_box.set_sensitive(true);
            button_box.remove(&undo_btn);
            button_box.append(&exists_button_box);
            manager.borrow_mut().store.restore_plug(plug_idx);
        }),
    );

    row
}

fn show_add_plug_dlg<F: IsA<gtk::Window>>(
    parent: &F,
    manager: &Arc<UiMutex<manager::Manager>>,
    plugs_panel: &gtk::ListBox,
) {
    glib::MainContext::new().spawn_local(clone!(parent, manager, plugs_panel => async move {
        if let Some(new_plugin) = plugin_settings_dlg::Builder::new("Add plugin")
            .show(&parent)
            .await
        {
            add_plugin(&manager, &plugs_panel, new_plugin).await;
        }
    }));
}

async fn add_plugin(
    manager: &Arc<UiMutex<manager::Manager>>,
    plugs_panel: &gtk::ListBox,
    new_plugin: PlugInfo,
) -> bool {
    let row = create_plug_row(manager.borrow().store.plugs_count(), &new_plugin, manager);

    if manager.borrow_mut().add_plug(new_plugin) {
        plugs_panel.append(&row);
        true
    } else {
        let dlg = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::empty(),
            gtk::MessageType::Error,
            gtk::ButtonsType::Ok,
            "Plugin with this name or path already exists",
        );
        dlg.run_future().await;
        dlg.close();
        false
    }
}

fn create_plug_label(plug_info: &PlugInfo) -> gtk::Box {
    let label_box = gtk::Box::new(gtk::Orientation::Vertical, 5);

    let name_lbl = gtk::Label::new(None);
    name_lbl.set_markup(&format!("<b>{}</b>", plug_info.name));
    name_lbl.set_halign(gtk::Align::Start);
    let url_lbl = gtk::Label::new(Some(plug_info.get_plug_path().as_str()));
    url_lbl.set_halign(gtk::Align::Start);

    label_box.append(&name_lbl);
    label_box.append(&url_lbl);
    label_box
}

fn add_vimawesome_tab(
    pages: &SettingsPages,
    manager: &Arc<UiMutex<manager::Manager>>,
    plugs_panel: &gtk::ListBox,
) {
    let get_plugins = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let spinner = gtk::Spinner::new();
    let get_plugins_lbl = gtk::Label::new(Some("Get Plugins"));
    pages.add_page(&get_plugins_lbl, &get_plugins, "get_plugins");

    let list_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let link_button = gtk::Label::new(None);
    link_button.set_markup(
        "Plugins are taken from: <a href=\"https://vimawesome.com\">https://vimawesome.com</a>",
    );
    let search_entry = gtk::SearchEntry::new();

    get_plugins.append(&link_button);
    get_plugins.append(&search_entry);
    get_plugins.append(&list_panel);
    list_panel.append(&spinner);
    spinner.start();

    search_entry.connect_activate(clone!(list_panel, manager, plugs_panel => move |se| {
        let spinner = gtk::Spinner::new();
        list_panel.append(&spinner);
        spinner.show();
        spinner.start();
        populate_get_plugins(
            Some(se.text().to_string()),
            &list_panel,
            manager.clone(),
            plugs_panel.clone()
        );
    }));

    glib::idle_add_local_once(clone!(manager, plugs_panel => move || {
        populate_get_plugins(None, &list_panel, manager.clone(), plugs_panel.clone());
    }));
}

fn add_help_tab(pages: &SettingsPages, markup: &str) {
    let help = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let label = gtk::Label::new(None);
    label.set_markup(markup);
    help.append(&label);

    let help_lbl = gtk::Label::new(Some("Help"));
    pages.add_page(&help_lbl, &help, "help");
}

struct SettingsPages {
    categories: gtk::ListBox,
    stack: gtk::Stack,
    content: gtk::Box,
    rows: Rc<RefCell<Vec<(gtk::ListBoxRow, &'static str)>>>,
}

impl SettingsPages {
    pub fn new<F: Fn(&str) + 'static>(row_selected: F) -> Self {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let categories = gtk::ListBox::new();
        categories.add_css_class("navigation-sidebar");
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        let rows: Rc<RefCell<Vec<(gtk::ListBoxRow, &'static str)>>> =
            Rc::new(RefCell::new(Vec::new()));

        content.append(&categories);
        content.append(&stack);

        categories.connect_row_selected(
            clone!(stack, rows => move |_, row| if let Some(row) = row {
                if let Some(r) = rows.borrow().iter().find(|r| r.0 == *row) {
                    if let Some(child) = stack.child_by_name(r.1) {
                        stack.set_visible_child(&child);
                        row_selected(r.1);
                    }

                }
            }),
        );

        SettingsPages {
            categories,
            stack,
            content,
            rows,
        }
    }

    fn add_page<W: glib::IsA<gtk::Widget>>(
        &self,
        label: &gtk::Label,
        widget: &W,
        name: &'static str,
    ) {
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        hbox.append(label);

        #[rustfmt::skip]
        let row = gtk::ListBoxRow::builder()
            .child(&hbox)
            .build();

        self.categories.append(&row);
        self.stack.add_named(widget, Some(name));
        self.rows.borrow_mut().push((row, name));
    }
}

impl Deref for SettingsPages {
    type Target = gtk::Box;

    fn deref(&self) -> &gtk::Box {
        &self.content
    }
}
