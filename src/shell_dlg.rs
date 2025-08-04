use std::{cell::RefCell, convert::*, rc::Rc, sync::Arc};

use log::{error, warn};

use gtk::prelude::*;
use gtk::{ButtonsType, MessageDialog, MessageType};

use crate::nvim::{NeovimClient, NormalError, NvimSession, SessionError};
use crate::shell::Shell;
use crate::ui::{Components, UiMutex};
use nvim_rs::Value;

pub fn can_close_window(
    comps: &Arc<UiMutex<Components>>,
    shell: &Rc<RefCell<Shell>>,
    nvim: &Rc<NeovimClient>,
) -> bool {
    if comps.borrow().exit_confirmed {
        return true;
    }

    if let Some(ref nvim) = nvim.nvim() {
        if nvim.is_blocked() {
            return false;
        }

        match get_changed_buffers(nvim) {
            Ok(vec) => {
                if !vec.is_empty() {
                    let comps = comps.clone();
                    let shell = shell.clone();
                    glib::MainContext::default().spawn_local(async move {
                        let res = {
                            let mut comps = comps.borrow_mut();
                            let res = show_not_saved_dlg(&comps, shell, &vec).await;

                            comps.exit_confirmed = res;
                            res
                        };

                        if res {
                            comps.borrow().close_window();
                        }
                    });
                    false
                } else {
                    true
                }
            }
            Err(ref err) => {
                error!("Error getting info from nvim: {err}");
                true
            }
        }
    } else {
        true
    }
}

async fn show_not_saved_dlg(
    comps: &Components,
    shell: Rc<RefCell<Shell>>,
    changed_bufs: &[String],
) -> bool {
    let mut changed_files = changed_bufs
        .iter()
        .map(|n| if n.is_empty() { "<No name>" } else { n })
        .fold(String::new(), |acc, v| acc + v + "\n");
    changed_files.pop();

    let flags = gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT;
    let dlg = MessageDialog::new(
        Some(comps.window()),
        flags,
        MessageType::Question,
        ButtonsType::None,
        format!("Save changes to '{changed_files}'?"),
    );

    dlg.add_buttons(&[
        ("_Yes", gtk::ResponseType::Yes),
        ("_No", gtk::ResponseType::No),
        ("_Cancel", gtk::ResponseType::Cancel),
    ]);

    let res = match dlg.run_future().await {
        gtk::ResponseType::Yes => {
            if let Some(nvim) = shell.borrow().state.borrow().nvim() {
                // FIXME: Figure out a way to use timeouts with nvim interactions when using glib for
                // async execution, either that or just don't use timeouts
                match nvim.command("wa").await {
                    Err(err) => {
                        match NormalError::try_from(&*err) {
                            Ok(err) => err.print(&nvim).await,
                            Err(_) => error!("Error: {err}"),
                        };
                        false
                    }
                    _ => true,
                }
            } else {
                false
            }
        }
        gtk::ResponseType::No => true,
        _ => false,
    };

    dlg.close();

    res
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
                        error!("Something going wrong while getting buffer option: {err}");
                        false
                    }
                },
                match nvim.block_timeout(buf.get_name()) {
                    Ok(name) => name,
                    Err(ref err) => {
                        error!("Something going wrong while getting buffer name: {err}");
                        "<Error>".to_owned()
                    }
                },
            )
        })
        .filter(|e| e.0)
        .map(|e| e.1)
        .collect())
}
