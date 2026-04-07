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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRange {
    pub start: usize,
    pub end: usize,
}

impl DirtyRange {
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "dirty range start={} end={}", start, end);
        Self { start, end }
    }

    fn extend(&mut self, other: Self) {
        self.start = self.start.min(other.start);
        self.end = self.end.max(other.end);
    }
}

pub struct Line {
    pub line: Box<[Cell]>,

    // format of item line is
    // [[Item1], [Item2], [], [], [Item3_1, Item3_2],]
    // Item2 takes 3 cells and renders as one
    // Item3_1 and Item3_2 share 1 cell and render as one
    pub item_line: Box<[Box<[Item]>]>,
    cell_to_item: Box<[i32]>,

    dirty_range: Option<DirtyRange>,
}

impl Line {
    pub fn new(columns: usize) -> Self {
        Line {
            line: vec![Cell::new_empty(); columns].into_boxed_slice(),
            item_line: vec![Box::default(); columns].into_boxed_slice(),
            cell_to_item: vec![-1; columns].into_boxed_slice(),
            dirty_range: (columns > 0).then_some(DirtyRange::new(0, columns - 1)),
        }
    }

    pub fn swap_with(&mut self, target: &mut Self, left: usize, right: usize) {
        // swap is faster then clone
        target.line[left..=right].swap_with_slice(&mut self.line[left..=right]);

        // this is because copy can change Item layout
        target.mark_dirty(left, right);
        for cell in &mut target.line[left..=right] {
            cell.dirty = true;
        }
    }

    pub fn clear(&mut self, left: usize, right: usize, default_hl: &Rc<Highlight>) {
        for cell in &mut self.line[left..=right] {
            cell.clear(default_hl.clone());
        }
        self.mark_dirty(left, right);
    }

    pub fn clear_glyphs(&mut self) {
        for i in 0..self.item_line.len() {
            self.item_line[i] = Box::default();
            self.cell_to_item[i] = -1;
        }
        if !self.line.is_empty() {
            self.dirty_range = Some(DirtyRange::new(0, self.line.len() - 1));
        }
    }

    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.dirty_range.is_some()
    }

    #[inline]
    pub fn dirty_range(&self) -> Option<DirtyRange> {
        self.dirty_range
    }

    pub fn expanded_dirty_range(&self) -> Option<DirtyRange> {
        let mut range = self.dirty_range?;
        range = self.include_double_width_cells(range);

        loop {
            let prev = range;
            range = self.expand_to_item_bounds(range);
            range = self.expand_to_ascii_runs(range);
            range = self.include_double_width_cells(range);

            if range == prev {
                return Some(range);
            }
        }
    }

    pub fn take_dirty_range(&mut self) -> Option<DirtyRange> {
        self.dirty_range.take()
    }

    pub fn clear_dirty(&mut self) {
        self.dirty_range = None;
    }

    pub fn mark_dirty(&mut self, start: usize, end: usize) {
        if self.line.is_empty() {
            return;
        }

        let start = start.min(self.line.len() - 1);
        let end = end.min(self.line.len() - 1);
        debug_assert!(start <= end, "dirty range start={} end={}", start, end);

        let range = DirtyRange::new(start, end);
        match &mut self.dirty_range {
            Some(existing) => existing.extend(range),
            None => self.dirty_range = Some(range),
        }
    }

    fn set_cell_to_empty(&mut self, cell_idx: usize) -> bool {
        if self.is_binded_to_item(cell_idx) {
            self.item_line[cell_idx] = Box::default();
            self.cell_to_item[cell_idx] = -1;
            self.line[cell_idx].dirty = true;
            self.mark_dirty(cell_idx, cell_idx);
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
                self.mark_dirty(new_item.start_cell, new_item.start_cell);
                true
            } else {
                false
            }
        }
    }

    pub fn merge(&mut self, old_items: &StyledLine, pango_items: &[pango::Item]) {
        if self.line.is_empty() {
            return;
        }

        self.merge_range(
            old_items,
            pango_items,
            DirtyRange::new(0, self.line.len() - 1),
        );
    }

    pub fn merge_range(
        &mut self,
        old_items: &StyledLine,
        pango_items: &[pango::Item],
        range: DirtyRange,
    ) {
        let mut pango_item_iter = PangoItemPositionIterator::new(pango_items, old_items);
        let mut next_item = pango_item_iter.next();
        let mut move_to_next_item = false;
        let mut cell_idx = range.start;

        while cell_idx <= range.end {
            match next_item {
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
        self.mark_dirty(new_item.start_cell, new_item.end_cell);
    }

    fn include_double_width_cells(&self, mut range: DirtyRange) -> DirtyRange {
        while range.start > 0 && self.line[range.start].double_width {
            range.start -= 1;
        }

        while range.end + 1 < self.line.len() && self.line[range.end + 1].double_width {
            range.end += 1;
        }

        range
    }

    fn expand_to_item_bounds(&self, range: DirtyRange) -> DirtyRange {
        let mut expanded = range;
        let mut cell_idx = range.start;

        while cell_idx <= expanded.end {
            if let Some(item_range) = self.item_bounds(cell_idx) {
                expanded.start = expanded.start.min(item_range.start);
                expanded.end = expanded.end.max(item_range.end);
                cell_idx = item_range.end + 1;
            } else {
                cell_idx += 1;
            }
        }

        expanded
    }

    fn expand_to_ascii_runs(&self, range: DirtyRange) -> DirtyRange {
        let mut expanded = range;

        for cell_idx in range.start..=range.end {
            if self.line[cell_idx].double_width {
                continue;
            }

            if cell_itemize_kind(&self.line[cell_idx]) != ItemizeCellKind::AsciiWord {
                continue;
            }

            let mut left = cell_idx;
            while let Some(prev_idx) = self.prev_render_cell(left) {
                if cell_itemize_kind(&self.line[prev_idx]) != ItemizeCellKind::AsciiWord {
                    break;
                }
                left = prev_idx;
            }

            let mut right = cell_idx;
            while let Some(next_idx) = self.next_render_cell(right) {
                if cell_itemize_kind(&self.line[next_idx]) != ItemizeCellKind::AsciiWord {
                    break;
                }
                right = next_idx;
            }

            expanded.start = expanded.start.min(left);
            expanded.end = expanded.end.max(right);
        }

        expanded
    }

    fn prev_render_cell(&self, cell_idx: usize) -> Option<usize> {
        let mut idx = cell_idx.checked_sub(1)?;
        loop {
            if !self.line[idx].double_width {
                return Some(idx);
            }
            idx = idx.checked_sub(1)?;
        }
    }

    fn next_render_cell(&self, cell_idx: usize) -> Option<usize> {
        let mut idx = cell_idx + 1;
        while idx < self.line.len() {
            if !self.line[idx].double_width {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    fn item_bounds(&self, cell_idx: usize) -> Option<DirtyRange> {
        let item_idx = self.cell_to_item(cell_idx);
        if item_idx < 0 {
            return None;
        }

        let item_idx = item_idx as usize;
        let cells_count = self.item_line[item_idx]
            .iter()
            .map(|item| item.cells_count)
            .max()
            .unwrap_or(1);

        Some(DirtyRange::new(item_idx, item_idx + cells_count - 1))
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemizeCellKind {
    Whitespace,
    AsciiWord,
    Complex,
}

fn cell_itemize_kind(cell: &Cell) -> ItemizeCellKind {
    let text = if cell.ch.is_empty() {
        " "
    } else {
        cell.ch.as_str()
    };

    let mut whitespace = true;
    let mut ascii = true;
    for ch in text.chars() {
        if whitespace && !ch.is_whitespace() {
            whitespace = false;
        }
        if !ch.is_ascii() {
            ascii = false;
        }
    }

    if whitespace {
        ItemizeCellKind::Whitespace
    } else if ascii {
        ItemizeCellKind::AsciiWord
    } else {
        ItemizeCellKind::Complex
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

impl PangoItemPosition<'_> {
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
        let first_item = self.iter.next()?;
        let mut items = vec![first_item];
        let length = first_item.length() as usize;
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
        if line.line.is_empty() {
            return StyledLine {
                line_str: String::new(),
                cell_to_byte: Box::default(),
                attr_list: pango::AttrList::new(),
            };
        }

        Self::from_range(
            line,
            DirtyRange::new(0, line.line.len() - 1),
            hl,
            font_features,
        )
    }

    pub fn from_range(
        line: &Line,
        range: DirtyRange,
        hl: &HighlightMap,
        font_features: &render::FontFeatures,
    ) -> Self {
        let average_capacity = (range.end - range.start + 1) * 4 * 2; // code bytes * grapheme cluster

        let mut line_str = String::with_capacity(average_capacity);
        let mut cell_to_byte = Vec::with_capacity(average_capacity);
        let attr_list = pango::AttrList::new();
        let mut byte_offset = 0;
        let mut style_attr = StyleAttr::new();

        for (offset, cell) in line.line[range.start..=range.end].iter().enumerate() {
            if cell.double_width {
                continue;
            }

            let cell_idx = range.start + offset;
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

impl PartialEq for StyleAttr<'_> {
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

    #[test]
    fn test_styled_line_range() {
        let mut line = Line::new(5);
        line[0].ch = "a".to_owned();
        line[1].ch = "b".to_owned();
        line[2].ch = "c".to_owned();
        line[3].ch = "d".to_owned();
        line[4].ch = "e".to_owned();

        let styled_line = StyledLine::from_range(
            &line,
            DirtyRange::new(1, 3),
            &HighlightMap::new(),
            &render::FontFeatures::new(),
        );
        assert_eq!("bcd", styled_line.line_str);
        assert_eq!(3, styled_line.cell_to_byte.len());
        assert_eq!(1, styled_line.cell_to_byte[0]);
        assert_eq!(2, styled_line.cell_to_byte[1]);
        assert_eq!(3, styled_line.cell_to_byte[2]);
    }

    #[test]
    fn test_dirty_range_expands_to_ascii_run() {
        let mut line = Line::new(11);
        for (idx, ch) in "hello world".chars().enumerate() {
            line[idx].ch = ch.to_string();
        }
        line.clear_dirty();
        line.mark_dirty(1, 1);

        assert_eq!(Some(DirtyRange::new(0, 4)), line.expanded_dirty_range(),);
    }

    #[test]
    fn test_dirty_range_expands_from_double_width_tail() {
        let mut line = Line::new(3);
        line[0].ch = "好".to_owned();
        line[1].double_width = true;
        line[2].ch = "a".to_owned();
        line.clear_dirty();
        line.mark_dirty(1, 1);

        assert_eq!(Some(DirtyRange::new(0, 1)), line.expanded_dirty_range(),);
    }
}
