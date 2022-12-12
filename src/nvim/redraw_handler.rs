use std::{
    convert::*,
    result,
    num::ParseFloatError,
    sync::Arc,
};

use nvim_rs::Value;

use crate::shell;
use crate::nvim::Tabpage;
use crate::ui::UiMutex;

use rmpv;

use crate::value::ValueMapExt;

/// Indicates whether we should queue a draw and if so, whether we should invalidate any internal
/// caches before doing so
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RedrawMode {
    /// No redraw required
    Nothing,
    /// A redraw is required, but only the state of the cursor has changed
    Cursor,
    /// A redraw is required and no glyphs have changed, but the snapshot cache must be cleared
    /// anyway
    ClearCache,
    /// A redraw is required and glyphs have changed
    All,
}

macro_rules! try_str {
    ($exp:expr) => {
        $exp.as_str()
            .ok_or_else(|| "Can't convert argument to string".to_owned())?
    };
}

macro_rules! try_string {
    ($exp:expr) => {
        if let Value::String(s) = $exp {
            if let Some(s) = s.into_str() {
                Ok(s)
            } else {
                Err("Can't convert to utf8 string".to_owned())
            }
        } else {
            Err("Can't convert to string".to_owned())
        }
    };
}

macro_rules! try_int {
    ($expr:expr) => {
        $expr
            .as_i64()
            .ok_or_else(|| "Can't convert argument to int".to_owned())?
    };
}

macro_rules! try_uint {
    ($exp:expr) => {
        $exp.as_u64()
            .ok_or_else(|| "Can't convert argument to u64".to_owned())?
    };
}

macro_rules! try_bool {
    ($exp:expr) => {
        $exp.as_bool()
            .ok_or_else(|| "Can't convert argument to bool".to_owned())?
    };
}

macro_rules! map_array {
    ($arg:expr, $err:expr, | $item:ident | $exp:expr) => {
        $arg.as_array().ok_or_else(|| $err).and_then(|items| {
            items
                .iter()
                .map(|$item| $exp)
                .collect::<Result<Vec<_>, _>>()
        })
    };
    ($arg:expr, $err:expr, | $item:ident |  { $exp:expr }) => {
        $arg.as_array().ok_or_else(|| $err).and_then(|items| {
            items
                .iter()
                .map(|$item| $exp)
                .collect::<Result<Vec<_>, _>>()
        })
    };
}

macro_rules! into_array {
    ($arg:expr, $err:expr, | $item:ident | $exp:expr) => {
        if let Value::Array(items) = $arg {
            items
                .into_iter()
                .map(|$item| $exp)
                .collect::<Result<Vec<_>, _>>()
        } else {
            Err($err.to_owned())
        }
    };
}

macro_rules! try_arg {
    ($value:expr,val_ref) => {
        &$value
    };
    ($value:expr,val) => {
        $value
    };
    ($value:expr,bool) => {
        try_bool!($value)
    };
    ($value:expr,uint) => {
        try_uint!($value)
    };
    ($value:expr,int) => {
        try_int!($value)
    };
    ($value:expr,float) => {
        try_float!($value)
    };
    ($value:expr,str) => {
        match $value {
            Value::String(s) => {
                if let Some(s) = s.into_str() {
                    Ok(s)
                } else {
                    Err("Can't convert to utf8 string".to_owned())
                }
            }
            _ => Err("Can't convert to string".to_owned()),
        }?
    };
    ($value:expr,ext) => {
        rmpv::ext::from_value($value).map_err(|e| e.to_string())?
    };
}

macro_rules! call {
    ($s:ident -> $c:ident ($args:ident : $($arg_type:ident),+ )) => (
        {
            let mut iter = $args.into_iter();
            $s.$c($(
                try_arg!(iter.next()
                             .ok_or_else(|| format!("No such argument for {}", stringify!($c)))?,
                         $arg_type
                        )
            ),+ )
        }
    )
}

fn set_ui_opt(ui: &mut shell::State, opts: &[&str], val: bool) -> Result<(), String> {
    let nvim = ui.nvim().ok_or_else(|| "Nvim should be initialized by now, but isn't".to_owned())?;

    for opt in opts {
        nvim.block_timeout(nvim.ui_set_option(opt, val.into())).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub enum NvimCommand {
    ToggleSidebar,
    ShowProjectView,
    ShowGtkInspector,
    Transparency(f64, f64),
    PreferDarkTheme(bool),
}

pub fn call_gui_event(
    ui: &mut shell::State,
    method: &str,
    args: Vec<Value>,
) -> result::Result<(), String> {
    match method {
        "Font" => call!(ui->set_font(args: str)),
        "FontFeatures" => call!(ui->set_font_features(args: str)),
        "Linespace" => call!(ui->set_line_space(args: str)),
        "Clipboard" => match try_str!(args[0]) {
            "Set" => match try_str!(args[1]) {
                "*" => ui.clipboard_primary_set(try_str!(args[2])),
                _ => ui.clipboard_clipboard_set(try_str!(args[2])),
            },
            opt => error!("Unknown option {}", opt),
        },
        "Option" => match try_str!(args[0]) {
            "Popupmenu" => set_ui_opt(ui, &["ext_popupmenu"], try_uint!(args[1]) == 1)?,
            "Tabline" => {
                let arg = try_uint!(args[1]) == 1;

                set_ui_opt(ui, &["ext_tabline"], arg)?;
                ui.set_tabline(arg);
            },
            "Cmdline" => set_ui_opt(ui, &["ext_cmdline", "ext_wildmenu"], try_uint!(args[1]) == 1)?,
            opt => error!("Unknown option {}", opt),
        },
        "Command" => {
            match try_str!(args[0]) {
                "ToggleSidebar" => ui.on_command(NvimCommand::ToggleSidebar),
                "ShowProjectView" => ui.on_command(NvimCommand::ShowProjectView),
                "ShowGtkInspector" => ui.on_command(NvimCommand::ShowGtkInspector),
                "Transparency" => ui.on_command(NvimCommand::Transparency(
                    try_str!(args.get(1).cloned().unwrap_or_else(|| "1.0".into()))
                        .parse()
                        .map_err(|e: ParseFloatError| e.to_string())?,
                    try_str!(args.get(2).cloned().unwrap_or_else(|| "1.0".into()))
                        .parse()
                        .map_err(|e: ParseFloatError| e.to_string())?,
                )),
                "PreferDarkTheme" => {
                    let prefer_dark_theme =
                        match try_str!(args.get(1).cloned().unwrap_or_else(|| Value::from("off"))) {
                            "on" => true,
                            _ => false,
                        };

                    ui.on_command(NvimCommand::PreferDarkTheme(prefer_dark_theme))
                }
                "SetCursorBlink" => {
                    let blink_count =
                        match try_str!(args.get(1).cloned().unwrap_or_else(|| Value::from(-1)))
                            .parse::<i32>()
                        {
                            Ok(val) => val,
                            Err(_) => -1,
                        };
                    ui.set_cursor_blink(blink_count);
                }
                _ => error!("Unknown command"),
            };
        }
        _ => return Err(format!("Unsupported event {}({:?})", method, args)),
    }
    Ok(())
}

pub fn call_gui_request(
    ui: &Arc<UiMutex<shell::State>>,
    method: &str,
    args: &Vec<Value>,
) -> result::Result<Value, Value> {
    match method {
        "Clipboard" => {
            match try_str!(args[0]) {
                "Get" => {
                    // NOTE: wait_for_text waits on the main loop. We can't have the ui borrowed
                    // while it runs, otherwise ui callbacks will get called and try to borrow
                    // mutably twice!
                    let clipboard = {
                        let ui = &mut ui.borrow_mut();
                        match try_str!(args[1]) {
                            "*" => ui.clipboard_primary.clone(),
                            _ => ui.clipboard_clipboard.clone(),
                        }
                    };
                    let t = glib::MainContext::default()
                        .block_on(clipboard.read_text_future())
                        .unwrap_or(None)
                        .unwrap_or("".into());

                    Ok(Value::Array(
                        t.split('\n').map(|s| s.into()).collect::<Vec<Value>>(),
                    ))
                }
                opt => Err(format!("Unknown option: {}", opt).into()),
            }
        }
        _ => Err(format!("Unsupported request {}({:?})", method, args).into()),
    }
}

pub fn call(
    ui: &mut shell::State,
    method: &str,
    args: Vec<Value>,
) -> result::Result<RedrawMode, String> {
    let mut flush = false;
    let repaint_mode = match method {
        "grid_line" => call!(ui->grid_line(args: uint, uint, uint, ext)),
        "grid_clear" => call!(ui->grid_clear(args: uint)),
        "grid_destroy" => call!(ui->grid_destroy(args: uint)),
        "grid_cursor_goto" => call!(ui->grid_cursor_goto(args: uint, uint, uint)),
        "grid_scroll" => call!(ui->grid_scroll(args: uint, uint, uint, uint, uint, int, int)),
        "grid_resize" => call!(ui->grid_resize(args: uint, uint, uint)),
        "default_colors_set" => call!(ui->default_colors_set(args: int, int, int, int, int)),
        "hl_attr_define" => call!(ui->hl_attr_define(args: uint, ext, val_ref, ext)),
        "mode_change" => call!(ui->on_mode_change(args: str, uint)),
        "mouse_on" => ui.on_mouse(true),
        "mouse_off" => ui.on_mouse(false),
        "busy_start" => ui.on_busy(true),
        "busy_stop" => ui.on_busy(false),
        "popupmenu_show" => {
            /* Complete lists can be enormous, so we want to be careful to avoid duplicating strings
             * by consuming the argument list */
            let mut iter = args.into_iter();
            let menu_items_in = into_array!(
                iter.next().ok_or("Menu list array is missing")?,
                "Failed to get menu list array",
                |item| into_array!(
                    item,
                    "Failed to get menu item array",
                    |col| try_string!(col)
                )
            )?;

            // XXX: Use try_collect() when that stabilizes
            let mut menu_items = Vec::with_capacity(menu_items_in.len());
            for menu_item in menu_items_in.into_iter() {
                menu_items.push(PopupMenuItem::new(menu_item)?);
            }

            let selected = try_int!(iter.next().ok_or("Failed to get selected popupmenu row")?);
            ui.set_pending_popupmenu(PendingPopupMenu::Show {
                items: menu_items,
                selected: if selected != -1 {
                    Some(u32::try_from(selected).map_err(|e| e.to_string())?)
                } else {
                    None
                },
                pos: (
                    try_uint!(iter.next().ok_or("Failed to get popupmenu row")?),
                    try_uint!(iter.next().ok_or("Failed to get popupmenu col")?),
                ),
            })
        }
        "popupmenu_hide" => ui.set_pending_popupmenu(PendingPopupMenu::Hide),
        "popupmenu_select" => call!(ui->popupmenu_select(args: int)),
        "tabline_update" => {
            let nvim = ui.nvim().ok_or_else(|| "Nvim not initialized".to_owned())?;
            let tabs_out = map_array!(args[1], "Error get tabline list".to_owned(), |tab| tab
                .as_map()
                .ok_or_else(|| "Error get map for tab".to_owned())
                .and_then(|tab_map| tab_map.to_attrs_map())
                .map(|tab_attrs| {
                    let name_attr = tab_attrs
                        .get("name")
                        .and_then(|n| n.as_str().map(|s| s.to_owned()));
                    let tab_attr = tab_attrs
                        .get("tab")
                        .map(|&tab_id| Tabpage::new(tab_id.clone(), (*nvim).clone()))
                        .unwrap();

                    (tab_attr, name_attr)
                }))?;
            ui.tabline_update(Tabpage::new(args[0].clone(), (*nvim).clone()), tabs_out)
        }
        "mode_info_set" => call!(ui->mode_info_set(args: bool, ext)),
        "option_set" => call!(ui->option_set(args: str, val)),
        "cmdline_show" => call!(ui->cmdline_show(args: ext, uint, str, str, uint, uint)),
        "cmdline_block_show" => call!(ui->cmdline_block_show(args: ext)),
        "cmdline_block_append" => call!(ui->cmdline_block_append(args: ext)),
        "cmdline_hide" => call!(ui->cmdline_hide(args: uint)),
        "cmdline_block_hide" => ui.cmdline_block_hide(),
        "cmdline_pos" => call!(ui->cmdline_pos(args: uint, uint)),
        "cmdline_special_char" => call!(ui->cmdline_special_char(args: str, bool, uint)),
        "wildmenu_show" => call!(ui->wildmenu_show(args: ext)),
        "wildmenu_hide" => ui.wildmenu_hide(),
        "wildmenu_select" => call!(ui->wildmenu_select(args: int)),
        "flush" => {
            debug!("Flush ({:?})", ui.pending_redraw);
            flush = true;
            ui.pending_redraw
        },
        _ => {
            warn!("Event {}({:?})", method, args);
            RedrawMode::Nothing
        }
    };

    if flush {
        ui.pending_redraw = RedrawMode::Nothing;
        Ok(repaint_mode)
    } else {
        ui.pending_redraw = ui.pending_redraw.max(repaint_mode);
        Ok(RedrawMode::Nothing)
    }
}

pub enum PendingPopupMenu {
    Show {
        items: Vec<PopupMenuItem>,
        selected: Option<u32>,
        pos: (u64, u64),
    },
    Select(Option<u32>),
    Hide,
}

pub struct PopupMenuItem {
    pub word: String,
    pub kind: String,
    pub menu: String,
    pub info: String,
}

impl PopupMenuItem {
    fn new(menu: Vec<String>) -> Result<Self, String> {
        let mut iter = menu.into_iter();
        Ok(PopupMenuItem {
            word: iter.next().ok_or("Complete item is missing word")?,
            kind: iter.next().ok_or("Complete item is missing kind")?,
            menu: iter.next().ok_or("Complete item is missing menu")?,
            info: iter.next().ok_or("Complete item is missing info")?,
        })
    }
}
