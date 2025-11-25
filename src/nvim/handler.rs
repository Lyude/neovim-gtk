use std::{
    result,
    sync::{mpsc, Arc},
};

use log::{debug, error};

use nvim_rs::{compat::tokio::Compat, Handler, Value};

use async_trait::async_trait;

use crate::nvim::{Neovim, NvimWriter};
use crate::shell;
use crate::ui::UiMutex;

use super::redraw_handler::{self, PendingPopupMenu, RedrawMode};

pub struct NvimHandler {
    shell: Arc<UiMutex<shell::State>>,
    resize_status: Arc<shell::ResizeState>,
}

impl NvimHandler {
    pub fn new(shell: Arc<UiMutex<shell::State>>, resize_status: Arc<shell::ResizeState>) -> Self {
        NvimHandler {
            shell,
            resize_status,
        }
    }

    async fn nvim_cb(&self, method: String, params: Vec<Value>) {
        match method.as_ref() {
            "redraw" => self.safe_call(move |ui| call_redraw_handler(params, ui)),
            "Gui" => {
                if !params.is_empty() {
                    let mut params_iter = params.into_iter();
                    if let Some(ev_name) = params_iter.next() {
                        if let Value::String(ev_name) = ev_name {
                            let args = params_iter.collect();
                            self.safe_call(move |ui| {
                                let ui = &mut ui.borrow_mut();
                                redraw_handler::call_gui_event(
                                    ui,
                                    ev_name.as_str().ok_or("Event name does not exists")?,
                                    args,
                                )?;
                                ui.queue_draw(RedrawMode::All);
                                Ok(())
                            });
                        } else {
                            error!("Unsupported event");
                        }
                    } else {
                        error!("Event name does not exists");
                    }
                } else {
                    error!("Unsupported event {params:?}");
                }
            }
            "subscription" => {
                self.safe_call(move |ui| {
                    let ui = &ui.borrow();
                    ui.notify(params)
                });
            }
            "resized" => {
                debug!("Received resized notification");
                self.resize_status.notify_finished();
            }
            _ => {
                error!("Notification {method}({params:?})");
            }
        }
    }

    fn nvim_cb_req(&self, method: String, params: Vec<Value>) -> result::Result<Value, Value> {
        match method.as_ref() {
            "Gui" => {
                if !params.is_empty() {
                    let mut params_iter = params.into_iter();
                    if let Some(req_name) = params_iter.next() {
                        if let Value::String(req_name) = req_name {
                            let args = params_iter.collect();
                            let (sender, receiver) = mpsc::channel();
                            self.safe_call(move |ui| {
                                sender
                                    .send(redraw_handler::call_gui_request(
                                        &ui.clone(),
                                        req_name.as_str().ok_or("Event name does not exists")?,
                                        &args,
                                    ))
                                    .unwrap();
                                {
                                    let ui = &mut ui.borrow_mut();
                                    ui.queue_draw(RedrawMode::All);
                                }
                                Ok(())
                            });
                            Ok(receiver.recv().unwrap()?)
                        } else {
                            error!("Unsupported request");
                            Err(Value::Nil)
                        }
                    } else {
                        error!("Request name does not exist");
                        Err(Value::Nil)
                    }
                } else {
                    error!("Unsupported request {params:?}");
                    Err(Value::Nil)
                }
            }
            _ => {
                error!("Request {method}({params:?})");
                Err(Value::Nil)
            }
        }
    }

    fn safe_call<F>(&self, cb: F)
    where
        F: FnOnce(&Arc<UiMutex<shell::State>>) -> result::Result<(), String> + 'static + Send,
    {
        safe_call(self.shell.clone(), cb);
    }
}

fn call_redraw_handler(
    params: Vec<Value>,
    ui: &Arc<UiMutex<shell::State>>,
) -> result::Result<(), String> {
    let mut repaint_mode = RedrawMode::Nothing;
    let mut pending_popupmenu = PendingPopupMenu::None;

    let mut ui_ref = ui.borrow_mut();
    for ev in params {
        let ev_args = match ev {
            Value::Array(args) => args,
            _ => {
                error!("Unsupported event type: {ev:?}");
                continue;
            }
        };
        let mut args_iter = ev_args.into_iter();
        let ev_name = match args_iter.next() {
            Some(ev_name) => ev_name,
            None => {
                error!(
                    "No name provided with redraw event, args: {:?}",
                    args_iter.as_slice()
                );
                continue;
            }
        };
        let ev_name = match ev_name.as_str() {
            Some(ev_name) => ev_name,
            None => {
                error!(
                    "Expected event name to be str, instead got {:?}. Args: {:?}",
                    ev_name,
                    args_iter.as_slice()
                );
                continue;
            }
        };

        for local_args in args_iter {
            let args = match local_args {
                Value::Array(ar) => ar,
                _ => vec![],
            };

            let (call_repaint_mode, call_popupmenu) =
                match redraw_handler::call(&mut ui_ref, ev_name, args) {
                    Ok(mode) => mode,
                    Err(desc) => return Err(format!("Event {ev_name}\n{desc}")),
                };
            repaint_mode = repaint_mode.max(call_repaint_mode);
            pending_popupmenu.update(call_popupmenu);
        }
    }

    ui_ref.queue_draw(repaint_mode);
    drop(ui_ref);
    ui.borrow().popupmenu_flush(pending_popupmenu);
    Ok(())
}

fn safe_call<F>(shell: Arc<UiMutex<shell::State>>, cb: F)
where
    F: FnOnce(&Arc<UiMutex<shell::State>>) -> result::Result<(), String> + 'static + Send,
{
    glib::idle_add_once(move || {
        if let Err(msg) = cb(&shell) {
            error!("Error call function: {msg}");
        }
    });
}

impl Clone for NvimHandler {
    fn clone(&self) -> Self {
        NvimHandler {
            shell: self.shell.clone(),
            resize_status: self.resize_status.clone(),
        }
    }
}

#[async_trait]
impl Handler for NvimHandler {
    type Writer = Compat<NvimWriter>;

    async fn handle_notify(&self, name: String, args: Vec<Value>, _: Neovim) {
        self.nvim_cb(name, args).await;
    }

    async fn handle_request(
        &self,
        name: String,
        args: Vec<Value>,
        _: Neovim,
    ) -> result::Result<Value, Value> {
        self.nvim_cb_req(name, args)
    }
}
