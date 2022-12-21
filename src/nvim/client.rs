use std::{cell::RefCell, rc::Rc, sync::RwLock};

use crate::nvim::*;

#[derive(Default)]
pub struct NeovimApiInfo {
    pub channel: i64,

    pub ext_cmdline: bool,
    pub ext_wildmenu: bool,
    pub ext_hlstate: bool,
    pub ext_linegrid: bool,
    pub ext_popupmenu: bool,
    pub ext_tabline: bool,
    pub ext_termcolors: bool,
}

impl NeovimApiInfo {
    pub fn new(api_info: Vec<Value>) -> Result<Self, String> {
        let mut self_ = Self::default();
        let mut api_info = api_info.into_iter();

        self_.channel = api_info
            .next()
            .ok_or("Channel is missing")?
            .as_i64()
            .ok_or("Channel is not i64")?;

        let metadata = match api_info.next().ok_or("Metadata is missing")? {
            Value::Map(pairs) => Ok(pairs),
            v @ _ => Err(format!("Metadata is wrong type, got {:?}", v)),
        }?;

        for (key, value) in metadata.into_iter() {
            match key
                .as_str()
                .ok_or(format!("Metadata key {:?} isn't string", key))?
            {
                "ui_options" => self_.parse_ui_options(value)?,
                _ => (),
            }
        }
        Ok(self_)
    }

    #[inline]
    fn parse_ui_options(&mut self, extensions: Value) -> Result<(), String> {
        for extension in extensions
            .as_array()
            .ok_or(format!("UI option list is invalid: {:?}", extensions))?
        {
            match extension
                .as_str()
                .ok_or(format!("UI option isn't string: {:?}", extensions))?
            {
                "ext_cmdline" => self.ext_cmdline = true,
                "ext_wildmenu" => self.ext_wildmenu = true,
                "ext_hlstate" => self.ext_hlstate = true,
                "ext_linegrid" => self.ext_linegrid = true,
                "ext_popupmenu" => self.ext_popupmenu = true,
                "ext_tabline" => self.ext_tabline = true,
                "ext_termcolors" => self.ext_termcolors = true,
                _ => (),
            };
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NeovimClientStatus {
    Uninitialized,
    InitInProgress,
    Initialized,
    Error,
}

struct NeovimClientState {
    status: NeovimClientStatus,
    api_info: Option<Rc<NeovimApiInfo>>,
}

pub struct NeovimClient {
    state: RefCell<NeovimClientState>,
    nvim: RwLock<Option<NvimSession>>,
}

impl NeovimClient {
    pub fn new() -> Self {
        NeovimClient {
            state: RefCell::new(NeovimClientState {
                status: NeovimClientStatus::Uninitialized,
                api_info: None,
            }),
            nvim: RwLock::new(None),
        }
    }

    pub fn clear(&self) {
        *self.nvim.write().unwrap() = None
    }

    pub fn set(&self, nvim: NvimSession) {
        self.nvim.write().unwrap().replace(nvim);
    }

    pub fn api_info(&self) -> Option<Rc<NeovimApiInfo>> {
        self.state.borrow().api_info.as_ref().cloned()
    }

    pub fn set_initialized(&self, api_info: NeovimApiInfo) {
        let mut state = self.state.borrow_mut();

        state.status = NeovimClientStatus::Initialized;
        state.api_info = Some(Rc::new(api_info));
    }

    pub fn set_error(&self) {
        self.state.borrow_mut().status = NeovimClientStatus::Error;
    }

    pub fn set_in_progress(&self) {
        self.state.borrow_mut().status = NeovimClientStatus::InitInProgress;
    }

    pub fn is_initialized(&self) -> bool {
        self.state.borrow().status == NeovimClientStatus::Initialized
    }

    pub fn is_uninitialized(&self) -> bool {
        self.state.borrow().status == NeovimClientStatus::Uninitialized
    }

    pub fn is_initializing(&self) -> bool {
        self.state.borrow().status == NeovimClientStatus::InitInProgress
    }

    pub fn nvim(&self) -> Option<NvimSession> {
        self.nvim.read().unwrap().clone()
    }
}
