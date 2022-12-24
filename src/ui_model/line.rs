use std::{
    iter::Peekable,
    ops::{Index, IndexMut},
    rc::Rc,
    slice::Iter,
};

use super::cell::Cell;
use super::item::Item;
use crate::color;
use crate::highlight::{Highlight, HighlightMap};
use crate::render;

pub struct Line {
    pub line: Box<[Cell]>,

    // format of item line is
    // [[Item1], [Item2], [], [], [Item3_1, Item3_2],]
    // Item2 takes 3 cells and renders as one
    // Item3_1 and Item3_2 share 1 cell and render as one
    pub item_line: Box<[Box<[Item]>]>,
    cell_to_item: Box<[i32]>,

    pub dirty_line: bool,
}

impl Line {
    pub fn new(columns: usize) -> Self {
        Line {
            line: vec![Cell::new_empty(); columns].into_boxed_slice(),
            item_line: vec![Box::default(); columns].into_boxed_slice(),
            cell_to_item: vec![-1; columns].into_boxed_slice(),
            dirty_line: true,
        }
    }

    pub fn swap_with(&mut self, target: &mut Self, left: usize, right: usize) {
        // swap is faster then clone
        target.line[left..=right].swap_with_slice(&mut self.line[left..=right]);

        // this is because copy can change Item layout
        target.dirty_line = true;
        for cell in &mut target.line[left..=right] {
            cell.dirty = true;
        }
    }

    pub fn clear(&mut self, left: usize, right: usize, default_hl: &Rc<Highlight>) {
        for cell in &mut self.line[left..=right] {
            cell.clear(default_hl.clone());
        }
        self.dirty_line = true;
    }

    pub fn clear_glyphs(&mut self) {
        for i in 0..self.item_line.len() {
            self.item_line[i] = Box::default();
            self.cell_to_item[i] = -1;
        }
        self.dirty_line = true;
    }

    fn set_cell_to_empty(&mut self, cell_idx: usize) -> bool {
        if self.is_binded_to_item(cell_idx) {
            self.item_line[cell_idx] = Box::default();
            self.cell_to_item[cell_idx] = -1;
            self.line[cell_idx].dirty = true;
            true
        } else {
            false
        }
    }

    fn set_cell_to_item(&mut self, new_item: &PangoItemPosition) -> bool {
        let start_item_idx = self.cell_to_item(new_item.start_cell);
        let start_item_cells_count = if start_item_idx >= 0 {
            let items = &self.item_line[start_item_idx as usize];
            if items.is_empty() {
                -1
            } else {
                items.iter().map(|i| i.cells_count as i32).max().unwrap()
            }
        } else {
            -1
        };

        let end_item_idx = self.cell_to_item(new_item.end_cell);

        // start_item == idx of item start cell
        // in case different item length was in previous iteration
        // mark all item as dirty
        let cell_count = new_item.cells_count();
        if start_item_idx != new_item.start_cell as i32
            || cell_count != start_item_cells_count
            || start_item_idx == -1
            || end_item_idx == -1
        {
            self.initialize_cell_item(new_item);
            true
        } else {
            // update only if cell marked as dirty
            if self.line[new_item.start_cell..=new_item.end_cell]
                .iter()
                .any(|c| c.dirty)
            {
                self.item_line[new_item.start_cell] = new_item
                    .items
                    .iter()
                    .map(|i| Item::new((*i).clone(), cell_count as usize))
                    .collect();
                self.line[new_item.start_cell].dirty = true;
                true
            } else {
                false
            }
        }
    }

    pub fn merge(&mut self, old_items: &StyledLine, pango_items: &[pango::Item]) {
        let mut pango_item_iter = PangoItemPositionIterator::new(pango_items, old_items);
        let mut next_item = pango_item_iter.next();
        let mut move_to_next_item = false;
        let mut cell_idx = 0;

        while cell_idx < self.line.len() {
            let dirty = match next_item {
                None => self.set_cell_to_empty(cell_idx),
                Some(ref new_item) => {
                    if cell_idx < new_item.start_cell {
                        self.set_cell_to_empty(cell_idx)
                    } else if cell_idx == new_item.start_cell {
                        move_to_next_item = true;
                        self.set_cell_to_item(new_item)
                    } else {
                        false
                    }
                }
            };

            self.dirty_line = self.dirty_line || dirty;
            if move_to_next_item {
                let new_item = next_item.unwrap();
                cell_idx += new_item.end_cell - new_item.start_cell + 1;
                next_item = pango_item_iter.next();
                move_to_next_item = false;
            } else {
                cell_idx += 1;
            }
        }
    }

    fn initialize_cell_item(&mut self, new_item: &PangoItemPosition) {
        for i in new_item.start_cell..=new_item.end_cell {
            self.line[i].dirty = true;
            self.cell_to_item[i] = new_item.start_cell as i32;
        }
        self.item_line[new_item.start_cell + 1..=new_item.end_cell].fill(Box::default());
        let cells_count = new_item.end_cell - new_item.start_cell + 1;
        self.item_line[new_item.start_cell] = new_item
            .items
            .iter()
            .map(|i| Item::new((*i).clone(), cells_count))
            .collect();
    }

    pub fn get_items(&self, cell_idx: usize) -> &[Item] {
        let item_idx = self.cell_to_item(cell_idx);
        if item_idx >= 0 {
            self.item_line[item_idx as usize].as_ref()
        } else {
            &[]
        }
    }

    #[inline]
    pub fn cell_to_item(&self, cell_idx: usize) -> i32 {
        self.cell_to_item[cell_idx]
    }

    pub fn item_len_from_idx(&self, start_idx: usize) -> usize {
        debug_assert!(
            start_idx < self.line.len(),
            "idx={}, len={}",
            start_idx,
            self.line.len()
        );

        let item_idx = self.cell_to_item(start_idx);

        if item_idx >= 0 {
            let item_idx = item_idx as usize;
            let cells_count: usize = self.item_line[item_idx]
                .iter()
                .map(|i| i.cells_count)
                .max()
                .unwrap();
            let offset = start_idx - item_idx;

            cells_count - offset
        } else {
            1
        }
    }

    #[inline]
    pub fn is_binded_to_item(&self, cell_idx: usize) -> bool {
        self.cell_to_item[cell_idx] >= 0
    }
}

impl Index<usize> for Line {
    type Output = Cell;

    fn index(&self, index: usize) -> &Cell {
        &self.line[index]
    }
}

impl IndexMut<usize> for Line {
    fn index_mut(&mut self, index: usize) -> &mut Cell {
        &mut self.line[index]
    }
}

struct PangoItemPosition<'a> {
    items: Vec<&'a pango::Item>,
    start_cell: usize,
    end_cell: usize,
}

impl<'a> PangoItemPosition<'a> {
    #[inline]
    fn cells_count(&self) -> i32 {
        (self.end_cell - self.start_cell) as i32 + 1
    }
}

struct PangoItemPositionIterator<'a> {
    iter: Peekable<Iter<'a, pango::Item>>,
    styled_line: &'a StyledLine,
}

impl<'a> PangoItemPositionIterator<'a> {
    pub fn new(items: &'a [pango::Item], styled_line: &'a StyledLine) -> Self {
        Self {
            iter: items.iter().peekable(),
            styled_line,
        }
    }
}

impl<'a> Iterator for PangoItemPositionIterator<'a> {
    type Item = PangoItemPosition<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let first_item = match self.iter.next() {
            Some(next) => next,
            None => return None,
        };
        let mut items = vec![first_item];
        let mut length = first_item.length() as usize;
        let offset = first_item.offset() as usize;
        let start_cell = self.styled_line.cell_to_byte[offset];
        let mut end_cell = self.styled_line.cell_to_byte[offset + length - 1];

        while let Some(next_item) = self.iter.peek() {
            let next_offset = next_item.offset() as usize;
            if self.styled_line.cell_to_byte[next_offset] > end_cell {
                break;
            }

            let next_len = next_item.length() as usize;
            let next_end_cell = self.styled_line.cell_to_byte[next_offset + next_len - 1];
            if next_end_cell > end_cell {
                end_cell = next_end_cell;
            }

            length += next_len;
            items.push(next_item);
            self.iter.next();
        }

        Some(PangoItemPosition {
            items,
            start_cell,
            end_cell,
        })
    }
}

pub struct StyledLine {
    pub line_str: String,
    cell_to_byte: Box<[usize]>,
    pub attr_list: pango::AttrList,
}

impl StyledLine {
    pub fn from(line: &Line, hl: &HighlightMap, font_features: &render::FontFeatures) -> Self {
        let average_capacity = line.line.len() * 4 * 2; // code bytes * grapheme cluster

        let mut line_str = String::with_capacity(average_capacity);
        let mut cell_to_byte = Vec::with_capacity(average_capacity);
        let attr_list = pango::AttrList::new();
        let mut byte_offset = 0;
        let mut style_attr = StyleAttr::new();

        for (cell_idx, cell) in line.line.iter().enumerate() {
            if cell.double_width {
                continue;
            }

            if !cell.ch.is_empty() {
                line_str.push_str(&cell.ch);
            } else {
                line_str.push(' ');
            }
            let len = line_str.len() - byte_offset;

            for _ in 0..len {
                cell_to_byte.push(cell_idx);
            }

            let next = style_attr.next(byte_offset, byte_offset + len, cell, hl);
            if let Some(next) = next {
                style_attr.insert_into(&attr_list);
                style_attr = next;
            }

            byte_offset += len;
        }

        style_attr.insert_into(&attr_list);
        font_features.insert_into(&attr_list);

        StyledLine {
            line_str,
            cell_to_byte: cell_to_byte.into_boxed_slice(),
            attr_list,
        }
    }
}

struct StyleAttr<'c> {
    italic: bool,
    bold: bool,
    foreground: Option<&'c color::Color>,
    background: Option<&'c color::Color>,
    empty: bool,
    space: bool,

    start_idx: usize,
    end_idx: usize,
}

impl<'c> StyleAttr<'c> {
    fn new() -> Self {
        StyleAttr {
            italic: false,
            bold: false,
            foreground: None,
            background: None,
            empty: true,
            space: false,

            start_idx: 0,
            end_idx: 0,
        }
    }

    fn from(start_idx: usize, end_idx: usize, cell: &'c Cell, hl: &'c HighlightMap) -> Self {
        StyleAttr {
            italic: cell.hl.italic,
            bold: cell.hl.bold,
            foreground: hl.cell_fg(cell),
            background: hl.cell_bg(cell),
            empty: false,
            space: cell.ch.is_empty(),

            start_idx,
            end_idx,
        }
    }

    fn next(
        &mut self,
        start_idx: usize,
        end_idx: usize,
        cell: &'c Cell,
        hl: &'c HighlightMap,
    ) -> Option<StyleAttr<'c>> {
        // don't check attr for space
        if self.space && cell.ch.is_empty() {
            self.end_idx = end_idx;
            return None;
        }

        let style_attr = Self::from(start_idx, end_idx, cell, hl);

        if self != &style_attr {
            Some(style_attr)
        } else {
            self.end_idx = end_idx;
            None
        }
    }

    fn insert_into(&self, attr_list: &pango::AttrList) {
        if self.empty {
            return;
        }

        if self.italic {
            self.insert_attr(
                attr_list,
                pango::AttrInt::new_style(pango::Style::Italic).into(),
            );
        }

        if self.bold {
            self.insert_attr(
                attr_list,
                pango::AttrInt::new_weight(pango::Weight::Bold).into(),
            );
        }

        if let Some(fg) = self.foreground {
            let (r, g, b) = fg.to_u16();
            self.insert_attr(attr_list, pango::AttrColor::new_foreground(r, g, b).into());
        }

        if let Some(bg) = self.background {
            let (r, g, b) = bg.to_u16();
            self.insert_attr(attr_list, pango::AttrColor::new_background(r, g, b).into());
        }
    }

    #[inline]
    fn insert_attr(&self, attr_list: &pango::AttrList, mut attr: pango::Attribute) {
        attr.set_start_index(self.start_idx as u32);
        attr.set_end_index(self.end_idx as u32);
        attr_list.insert(attr);
    }
}

impl<'c> PartialEq for StyleAttr<'c> {
    fn eq(&self, other: &Self) -> bool {
        self.italic == other.italic
            && self.bold == other.bold
            && self.foreground == other.foreground
            && self.empty == other.empty
            && self.background == other.background
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_styled_line() {
        let mut line = Line::new(3);
        line[0].ch = "a".to_owned();
        line[1].ch = "b".to_owned();
        line[2].ch = "c".to_owned();

        let styled_line =
            StyledLine::from(&line, &HighlightMap::new(), &render::FontFeatures::new());
        assert_eq!("abc", styled_line.line_str);
        assert_eq!(3, styled_line.cell_to_byte.len());
        assert_eq!(0, styled_line.cell_to_byte[0]);
        assert_eq!(1, styled_line.cell_to_byte[1]);
        assert_eq!(2, styled_line.cell_to_byte[2]);
    }
}
