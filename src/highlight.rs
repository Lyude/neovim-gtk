use std::{borrow::Cow, collections::HashMap, rc::Rc};

use log::error;

use fnv::FnvHashMap;

use crate::color::*;
use crate::ui_model::Cell;
use nvim_rs::Value;

pub struct HighlightMap {
    highlights: FnvHashMap<u64, Rc<Highlight>>,
    default_hl: Rc<Highlight>,
    background_state: BackgroundState,
    bg_color: Option<Color>,
    fg_color: Option<Color>,
    sp_color: Option<Color>,

    cterm_bg_color: Color,
    cterm_fg_color: Color,
    cterm_color: bool,

    pmenu: Rc<Highlight>,
    pmenu_sel: Rc<Highlight>,
    cursor: Rc<Highlight>,
}

/// Enum for the 'background' setting in neovim, which we track to determine default colors
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackgroundState {
    Light,
    Dark,
}

#[derive(Clone, Copy, Default)]
pub struct HighlightUpdates {
    pub pmenu: bool,
    pub cursor: bool,
}

impl HighlightMap {
    pub fn new() -> Self {
        let default_hl = Rc::new(Highlight::new());
        HighlightMap {
            highlights: FnvHashMap::default(),
            background_state: BackgroundState::Dark,
            bg_color: None,
            fg_color: None,
            sp_color: None,

            cterm_bg_color: COLOR_BLACK,
            cterm_fg_color: COLOR_WHITE,
            cterm_color: false,

            pmenu: default_hl.clone(),
            pmenu_sel: default_hl.clone(),
            cursor: default_hl.clone(),

            default_hl,
        }
    }

    pub fn default_hl(&self) -> Rc<Highlight> {
        self.default_hl.clone()
    }

    pub fn set_defaults(
        &mut self,
        fg: Option<Color>,
        bg: Option<Color>,
        sp: Option<Color>,
        cterm_fg: Color,
        cterm_bg: Color,
    ) {
        self.fg_color = fg;
        self.bg_color = bg;
        self.sp_color = sp;
        self.cterm_fg_color = cterm_fg;
        self.cterm_bg_color = cterm_bg;
    }

    pub fn set_use_cterm(&mut self, cterm_color: bool) {
        self.cterm_color = cterm_color;
    }

    pub fn set_background_state(&mut self, state: BackgroundState) {
        self.background_state = state;
    }

    fn default_fg(&self) -> &Color {
        match self.background_state {
            BackgroundState::Light => &COLOR_BLACK,
            BackgroundState::Dark => &COLOR_WHITE,
        }
    }

    fn default_bg(&self) -> &Color {
        match self.background_state {
            BackgroundState::Light => &COLOR_WHITE,
            BackgroundState::Dark => &COLOR_BLACK,
        }
    }

    pub fn bg(&self) -> &Color {
        if self.cterm_color {
            &self.cterm_bg_color
        } else {
            self.bg_color.as_ref().unwrap_or_else(|| self.default_bg())
        }
    }

    pub fn fg(&self) -> &Color {
        if self.cterm_color {
            &self.cterm_fg_color
        } else {
            self.fg_color.as_ref().unwrap_or_else(|| self.default_fg())
        }
    }

    pub fn get(&self, idx: Option<u64>) -> Rc<Highlight> {
        idx.and_then(|idx| self.highlights.get(&idx))
            .map(Rc::clone)
            .unwrap_or_else(|| {
                self.highlights
                    .get(&0)
                    .map(Rc::clone)
                    .unwrap_or_else(|| self.default_hl.clone())
            })
    }

    #[must_use]
    pub fn set(
        &mut self,
        idx: u64,
        hl: &HashMap<String, Value>,
        info: &[HashMap<String, Value>],
    ) -> HighlightUpdates {
        let hl = Rc::new(Highlight::from_value_map(hl));
        let mut updates = HighlightUpdates::default();

        for item in info {
            if item.get("kind").unwrap().as_str().unwrap() != "syntax" {
                continue;
            }

            let (updated_ref, hl_ref) = match item.get("hi_name").and_then(Value::as_str) {
                Some("Pmenu") => (&mut updates.pmenu, &mut self.pmenu),
                Some("PmenuSel") => (&mut updates.pmenu, &mut self.pmenu_sel),
                Some("Cursor") => (&mut updates.cursor, &mut self.cursor),
                _ => continue,
            };

            if *hl_ref != hl {
                *updated_ref = true;
                *hl_ref = hl.clone();
            }
        }

        self.highlights.insert(idx, hl);
        updates
    }

    pub fn cell_fg<'a>(&'a self, cell: &'a Cell) -> Option<&'a Color> {
        if !cell.hl.reverse {
            cell.hl.foreground.as_ref()
        } else {
            cell.hl.background.as_ref().or_else(|| Some(self.bg()))
        }
    }

    pub fn actual_cell_fg<'a>(&'a self, cell: &'a Cell) -> &'a Color {
        if !cell.hl.reverse {
            cell.hl.foreground.as_ref().unwrap_or_else(|| self.fg())
        } else {
            cell.hl.background.as_ref().unwrap_or_else(|| self.bg())
        }
    }

    pub fn cell_bg<'a>(&'a self, cell: &'a Cell) -> Option<&'a Color> {
        if !cell.hl.reverse {
            cell.hl.background.as_ref()
        } else {
            cell.hl.foreground.as_ref().or_else(|| Some(self.fg()))
        }
    }

    pub fn actual_cell_bg<'a>(&'a self, cell: &'a Cell) -> &'a Color {
        if !cell.hl.reverse {
            cell.hl.background.as_ref().unwrap_or_else(|| self.bg())
        } else {
            cell.hl.foreground.as_ref().unwrap_or_else(|| self.fg())
        }
    }

    #[inline]
    pub fn cell_sp<'a>(&'a self, cell: &'a Cell) -> Option<&'a Color> {
        cell.hl.special.as_ref().or(self.sp_color.as_ref())
    }

    pub fn pmenu_bg(&self) -> &Color {
        if !self.pmenu.reverse {
            self.pmenu.background.as_ref().unwrap_or_else(|| self.bg())
        } else {
            self.pmenu.foreground.as_ref().unwrap_or_else(|| self.fg())
        }
    }

    pub fn pmenu_fg(&self) -> &Color {
        if !self.pmenu.reverse {
            self.pmenu.foreground.as_ref().unwrap_or_else(|| self.fg())
        } else {
            self.pmenu.background.as_ref().unwrap_or_else(|| self.bg())
        }
    }

    pub fn pmenu_bg_sel(&self) -> &Color {
        if !self.pmenu_sel.reverse {
            self.pmenu_sel
                .background
                .as_ref()
                .unwrap_or_else(|| self.bg())
        } else {
            self.pmenu_sel
                .foreground
                .as_ref()
                .unwrap_or_else(|| self.fg())
        }
    }

    pub fn pmenu_fg_sel(&self) -> &Color {
        if !self.pmenu_sel.reverse {
            self.pmenu_sel
                .foreground
                .as_ref()
                .unwrap_or_else(|| self.fg())
        } else {
            self.pmenu_sel
                .background
                .as_ref()
                .unwrap_or_else(|| self.bg())
        }
    }

    pub fn cursor_bg(&self) -> Cow<Color> {
        if self.cursor.reverse {
            Cow::Borrowed(self.cursor.foreground.as_ref().unwrap_or_else(|| self.fg()))
        } else {
            self.cursor
                .background
                .as_ref()
                .map(Cow::Borrowed)
                .unwrap_or_else(|| Cow::Owned(self.bg().invert()))
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Highlight {
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
    pub undercurl: bool,
    pub strikethrough: bool,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub special: Option<Color>,
    pub reverse: bool,
}

impl Highlight {
    pub fn new() -> Self {
        Highlight {
            foreground: None,
            background: None,
            special: None,
            italic: false,
            bold: false,
            underline: false,
            undercurl: false,
            strikethrough: false,
            reverse: false,
        }
    }

    pub fn from_value_map(attrs: &HashMap<String, Value>) -> Self {
        let mut model_attrs = Highlight::new();

        for (ref key, val) in attrs {
            match key.as_ref() {
                "foreground" => {
                    if let Some(fg) = val.as_u64() {
                        model_attrs.foreground = Some(Color::from_indexed_color(fg));
                    }
                }
                "background" => {
                    if let Some(bg) = val.as_u64() {
                        model_attrs.background = Some(Color::from_indexed_color(bg));
                    }
                }
                "special" => {
                    if let Some(bg) = val.as_u64() {
                        model_attrs.special = Some(Color::from_indexed_color(bg));
                    }
                }
                "standout" => {
                    model_attrs.bold = true;
                    model_attrs.reverse = true;
                }
                "reverse" => model_attrs.reverse = true,
                "bold" => model_attrs.bold = true,
                "italic" => model_attrs.italic = true,
                "underline" => model_attrs.underline = true,
                "undercurl" => model_attrs.undercurl = true,
                "strikethrough" => model_attrs.strikethrough = true,
                "blend" => (),
                attr_key => error!("unknown attribute {}", attr_key),
            };
        }

        model_attrs
    }
}
