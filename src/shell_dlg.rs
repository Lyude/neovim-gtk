use std::cell::RefCell;
use std::rc::Rc;

use gtk;
use gtk::prelude::*;
use gtk::{ButtonsType, MessageDialog, MessageType};

use nvim_rs::Value;
use crate::nvim::{NvimSession, SessionError, NeovimClient};
use crate::shell::Shell;
use crate::ui::{Components, UiMutex};

pub fn can_close_window(
    comps: &UiMutex<Components>,
    shell: &RefCell<Shell>,
    nvim: &Rc<NeovimClient>
) -> bool {
    let shell = shell.borrow();

    if let Some(ref nvim) = nvim.nvim() {
        if nvim.is_blocked() {
            return false
        }

        match get_changed_buffers(nvim) {
            Ok(vec) => {
                if !vec.is_empty() {
                    show_not_saved_dlg(comps, &*shell, &vec)
                } else {
                    true
                }
            }
            Err(ref err) => {
                error!("Error getting info from nvim: {}", err);
                true
            }
        }
    } else {
        true
    }
}

fn show_not_saved_dlg(comps: &UiMutex<Components>, shell: &Shell, changed_bufs: &[String]) -> bool {
    let mut changed_files = changed_bufs
        .iter()
        .map(|n| if n.is_empty() { "<No name>" } else { n })
        .fold(String::new(), |acc, v| acc + v + "\n");
    changed_files.pop();

    let flags = gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT;
    let dlg = MessageDialog::new(
        Some(comps.borrow().window()),
        flags,
        MessageType::Question,
        ButtonsType::None,
        &format!("Save changes to '{}'?", changed_files),
    );

    dlg.add_buttons(&[
        ("_Yes", gtk::ResponseType::Yes),
        ("_No", gtk::ResponseType::No),
        ("_Cancel", gtk::ResponseType::Cancel),
    ]);

    glib::MainContext::new().block_on(async move {
        let res = match dlg.run_future().await {
            gtk::ResponseType::Yes => {
                let state = shell.state.borrow();
                let nvim = state.nvim().unwrap();
                match nvim.block_timeout(nvim.command("wa")) {
                    Err(err) => {
                        error!("Error: {}", err);
                        false
                    }
                    _ => true,
                }
            }
            gtk::ResponseType::No => true,
            gtk::ResponseType::Cancel | _ => false,
        };

        dlg.close();

        res
    })
}

fn get_changed_buffers(nvim: &NvimSession) -> Result<Vec<String>, SessionError> {
    let buffers = nvim.block_timeout(nvim.list_bufs()).unwrap();

    Ok(buffers
        .iter()
        .map(|buf| {
            (
                match nvim.block_timeout(buf.get_option("modified")) {
                    Ok(Value::Boolean(val)) => val,
                    Ok(_) => {
                        warn!("Value must be boolean");
                        false
                    }
                    Err(ref err) => {
                        error!("Something going wrong while getting buffer option: {}", err);
                        false
                    }
                },
                match nvim.block_timeout(buf.get_name()) {
                    Ok(name) => name,
                    Err(ref err) => {
                        error!("Something going wrong while getting buffer name: {}", err);
                        "<Error>".to_owned()
                    }
                },
            )
        })
        .filter(|e| e.0)
        .map(|e| e.1)
        .collect())
}
