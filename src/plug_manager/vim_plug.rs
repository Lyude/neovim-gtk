use std::rc::Rc;

use crate::nvim::{ErrorReport, NeovimClient, NvimSession};
use crate::spawn_timeout;
use crate::value::ValueMapExt;

pub struct Manager {
    nvim: Option<Rc<NeovimClient>>,
}

impl Manager {
    pub fn new() -> Self {
        Manager { nvim: None }
    }

    pub fn initialize(&mut self, nvim: Rc<NeovimClient>) {
        self.nvim = Some(nvim);
    }

    fn nvim(&self) -> Option<NvimSession> {
        self.nvim.as_ref().unwrap().nvim()
    }

    pub fn get_plugs(&self) -> Result<Box<[VimPlugInfo]>, String> {
        if let Some(nvim) = self.nvim() {
            let g_plugs = nvim
                .block_timeout(nvim.eval("g:plugs"))
                .map_err(|e| format!("Can't retrieve g:plugs map: {e}"))?;

            let plugs_map = g_plugs
                .as_map()
                .ok_or_else(|| "Can't retrieve g:plugs map".to_owned())?
                .to_attrs_map()?;

            let g_plugs_order = nvim
                .block_timeout(nvim.eval("g:plugs_order"))
                .map_err(|e| format!("{e}"))?;

            let order_arr = g_plugs_order
                .as_array()
                .ok_or_else(|| "Can't find g:plugs_order array".to_owned())?;

            let plugs_info: Vec<VimPlugInfo> = order_arr
                .iter()
                .map(|n| n.as_str())
                .filter_map(|name| {
                    if let Some(name) = name {
                        plugs_map
                            .get(name)
                            .and_then(|desc| desc.as_map())
                            .and_then(|desc| desc.to_attrs_map().ok())
                            .and_then(|desc| {
                                let uri = desc.get("uri").and_then(|uri| uri.as_str());
                                uri.map(|uri| VimPlugInfo::new(name.to_owned(), uri.to_owned()))
                            })
                    } else {
                        None
                    }
                })
                .collect();
            Ok(plugs_info.into_boxed_slice())
        } else {
            Err("Nvim not initialized".to_owned())
        }
    }

    pub fn is_loaded(&self) -> bool {
        if let Some(nvim) = self.nvim() {
            let loaded_plug = nvim.block_timeout(nvim.eval("exists('g:loaded_plug')"));
            loaded_plug
                .ok_and_report()
                .and_then(|loaded_plug| loaded_plug.as_i64())
                .is_some_and(|loaded_plug| loaded_plug > 0)
        } else {
            false
        }
    }

    pub fn reload(&self, path: &str) {
        let path = path.to_owned();
        if let Some(nvim) = self.nvim() {
            spawn_timeout!(nvim.command(&format!("source {path}")));
        }
    }
}

#[derive(Debug)]
pub struct VimPlugInfo {
    pub name: String,
    pub uri: String,
}

impl VimPlugInfo {
    pub fn new(name: String, uri: String) -> Self {
        VimPlugInfo { name, uri }
    }
}
