use std::{cell::RefCell, collections::hash_map::HashMap, ops::Deref, rc::Rc};

use gtk::prelude::*;

use crate::{
    nvim::{self, ErrorReport, NvimSession, Tabpage},
    spawn_timeout_user_err,
};

struct State {
    tabpages: Vec<Tabpage>,
    selected: Option<Tabpage>,
    /// Since GTK only gives us the new index (at least afaict) of the tab page after a reorder
    /// operation, we keep a map of tab widgets to their last known position
    widget_map: HashMap<gtk::Widget, u32>,
    nvim: Option<Rc<nvim::NeovimClient>>,
}

impl State {
    pub fn new() -> Self {
        State {
            tabpages: Vec::new(),
            widget_map: HashMap::new(),
            selected: None,
            nvim: None,
        }
    }

    fn nvim(&self) -> NvimSession {
        self.nvim
            .as_ref()
            .and_then(|c| c.nvim())
            .expect("Tabline shouldn't be usable before nvim is initialized")
    }

    fn switch_page(&self, idx: u32) {
        let target = &self.tabpages[idx as usize];
        if Some(target) != self.selected.as_ref() {
            let nvim = self.nvim();
            nvim.block_timeout(nvim.set_current_tabpage(target))
                .report_err();
        }
    }

    fn reorder_page(&self, idx: u32, mut new_idx: u32) {
        let nvim = self.nvim();

        // :help :tabm - "N is counted before the move"
        if new_idx > idx {
            new_idx += 1;
        }

        spawn_timeout_user_err!(nvim.command(&format!("{}tabd tabm {new_idx}", idx + 1)));
    }

    fn close_tab(&self, idx: u32) {
        let nvim = self.nvim();
        spawn_timeout_user_err!(nvim.command(&format!("tabc {}", idx + 1)));
    }
}

pub struct Tabline {
    tabs: gtk::Notebook,
    state: Rc<RefCell<State>>,
    signal_handlers: [glib::SignalHandlerId; 2],
}

impl Tabline {
    pub fn new() -> Self {
        let tabs = gtk::Notebook::builder()
            .can_focus(false)
            .scrollable(true)
            .show_border(false)
            .hexpand(true)
            .sensitive(false)
            .visible(false)
            .build();

        let state = Rc::new(RefCell::new(State::new()));

        Tabline {
            tabs: tabs.clone(),
            state: state.clone(),
            signal_handlers: [
                tabs.connect_switch_page(glib::clone!(
                    #[strong]
                    state,
                    move |_, _, idx| state.borrow().switch_page(idx)
                )),
                tabs.connect_page_reordered(glib::clone!(
                    #[strong]
                    state,
                    move |_, tab, idx| {
                        let state = state.borrow();
                        state.reorder_page(state.widget_map[tab], idx);
                    }
                )),
            ],
        }
    }

    fn update_state(
        &self,
        nvim: &Rc<nvim::NeovimClient>,
        selected: &Tabpage,
        tabs: &[(Tabpage, Option<String>)],
    ) {
        let mut state = self.state.borrow_mut();

        if state.nvim.is_none() {
            state.nvim = Some(nvim.clone());
        }

        state.selected = Some(selected.clone());

        state.tabpages = tabs.iter().map(|item| item.0.clone()).collect();
        state.widget_map.clear();
    }

    pub fn update_tabs(
        &self,
        nvim: &Rc<nvim::NeovimClient>,
        selected: Tabpage,
        tabs: Vec<(Tabpage, Option<String>)>,
    ) {
        if tabs.len() <= 1 {
            self.tabs.hide();
            return;
        } else {
            self.tabs.show();
        }

        self.update_state(nvim, &selected, &tabs);
        for signal in &self.signal_handlers {
            self.block_signal(signal);
        }

        let count = self.tabs.n_pages() as usize;
        if count < tabs.len() {
            for _ in count..tabs.len() {
                let empty = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let title = gtk::Label::builder()
                    .ellipsize(pango::EllipsizeMode::Middle)
                    .width_chars(25)
                    .hexpand(true)
                    .build();

                let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
                close_btn.set_has_frame(false);
                close_btn.set_focus_on_click(false);

                let label_box = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .hexpand(true)
                    .build();
                label_box.append(&title);
                label_box.append(&close_btn);

                self.tabs.append_page(&empty, Some(&label_box));
                self.tabs.set_tab_reorderable(&empty, true);

                let tabs = self.tabs.clone();
                let state_ref = Rc::clone(&self.state);
                close_btn.connect_clicked(move |btn| {
                    let current_label = btn.parent().unwrap();
                    for i in 0..tabs.n_pages() {
                        let page = tabs.nth_page(Some(i)).unwrap();
                        let label = tabs.tab_label(&page).unwrap();
                        if label == current_label {
                            state_ref.borrow().close_tab(i);
                        }
                    }
                });
            }
        } else if count > tabs.len() {
            for _ in tabs.len()..count {
                self.tabs.remove_page(None);
            }
        }

        let mut state = self.state.borrow_mut();

        for (idx, tab) in tabs.iter().enumerate() {
            let tab_child = self.tabs.nth_page(Some(idx as u32)).unwrap();
            state.widget_map.insert(tab_child.clone(), idx as u32);

            let tab_label = self
                .tabs
                .tab_label(&tab_child)
                .unwrap()
                .first_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap();
            tab_label.set_text(tab.1.as_ref().unwrap_or(&"??".to_owned()));

            if selected == tab.0 {
                self.tabs.set_current_page(Some(idx as u32));
            }
        }

        drop(state);
        for signal in &self.signal_handlers {
            self.unblock_signal(signal);
        }
    }
}

impl Deref for Tabline {
    type Target = gtk::Notebook;

    fn deref(&self) -> &gtk::Notebook {
        &self.tabs
    }
}
