use crate::highlight::Highlight;
use std::rc::Rc;

mod cell;
mod item;
mod line;
mod model_layout;
mod model_rect;

pub use self::cell::Cell;
pub use self::item::Item;
pub use self::line::{Line, StyledLine};
pub use self::model_layout::{HighlightedLine, HighlightedRange, ModelLayout};
pub use self::model_rect::ModelRect;

#[derive(Default)]
pub struct UiModel {
    pub columns: usize,
    pub rows: usize,
    /// (row, col)
    cur_pos: (usize, usize),
    /// (row, col)
    flushed_pos: (usize, usize),
    model: Box<[Line]>,
}

impl UiModel {
    pub fn new(rows: u64, columns: u64) -> UiModel {
        let mut model = Vec::with_capacity(rows as usize);
        for _ in 0..rows as usize {
            model.push(Line::new(columns as usize));
        }

        UiModel {
            columns: columns as usize,
            rows: rows as usize,
            cur_pos: (0, 0),
            flushed_pos: (0, 0),
            model: model.into_boxed_slice(),
        }
    }

    pub fn resize(&mut self, rows: u64, columns: u64) {
        let rows = rows as usize;
        let columns = columns as usize;
        if self.rows == rows && self.columns == columns {
            return;
        }

        let mut model = std::mem::take(&mut self.model).into_vec();
        model.truncate(rows);
        for line in &mut model {
            line.resize(columns);
        }
        model.resize_with(rows, || Line::new(columns));

        self.rows = rows;
        self.columns = columns;
        self.cur_pos = Self::clamp_pos(self.cur_pos, rows, columns);
        self.flushed_pos = Self::clamp_pos(self.flushed_pos, rows, columns);
        self.model = model.into_boxed_slice();
    }

    #[inline]
    pub fn model(&self) -> &[Line] {
        &self.model
    }

    #[inline]
    pub fn model_mut(&mut self) -> &mut [Line] {
        &mut self.model
    }

    /// Get the current point where the cursor is located. Note that this isn't what you want to use
    /// if you
    pub fn cur_real_point(&self) -> ModelRect {
        let (row, col) = self.cur_pos;
        ModelRect::point(col, row)
    }

    #[inline]
    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.cur_pos = (row, col);
    }

    #[inline]
    pub fn flush_cursor(&mut self) {
        self.flushed_pos = self.cur_pos;
    }

    /// Get the "real" cursor position, e.g. use the intermediate position if there is one. This is
    /// usually what you want for UI model operations
    #[inline]
    pub fn get_real_cursor(&self) -> (usize, usize) {
        self.cur_pos
    }

    /// Get the position of the cursor from the last 'flush' event. This is usually what you want
    /// for snapshot generation
    #[inline]
    pub fn get_flushed_cursor(&self) -> (usize, usize) {
        self.flushed_pos
    }

    pub fn put_one(
        &mut self,
        row: usize,
        col: usize,
        ch: &str,
        double_width: bool,
        hl: Rc<Highlight>,
    ) {
        self.put(row, col, ch, double_width, 1, hl);
    }

    pub fn put(
        &mut self,
        row: usize,
        col: usize,
        ch: &str,
        double_width: bool,
        repeat: usize,
        hl: Rc<Highlight>,
    ) {
        let line = &mut self.model[row];
        line.mark_dirty(col, col + repeat - 1);

        for offset in 0..repeat {
            let cell = &mut line[col + offset];
            cell.ch.clear();
            cell.ch.push_str(ch);
            cell.hl = hl.clone();
            cell.double_width = double_width;
            cell.dirty = true;
        }
    }

    /// Copy rows from 0 to to_row, col from 0 self.columns
    ///
    /// Don't do any validation!
    pub fn swap_rows(&mut self, target: &mut UiModel, to_row: usize) {
        for (row_idx, line) in self.model[0..to_row + 1].iter_mut().enumerate() {
            let target_row = &mut target.model[row_idx];
            line.swap_with(target_row, 0, self.columns - 1);
        }
    }

    fn swap_row(&mut self, target_row: i64, offset: i64, left_col: usize, right_col: usize) {
        debug_assert_ne!(0, offset);

        let from_row = (target_row + offset) as usize;

        let (left, right) = if offset > 0 {
            self.model.split_at_mut(from_row)
        } else {
            self.model.split_at_mut(target_row as usize)
        };

        let (source_row, target_row) = if offset > 0 {
            (&mut right[0], &mut left[target_row as usize])
        } else {
            (&mut left[from_row], &mut right[0])
        };

        source_row.swap_with(target_row, left_col, right_col);
    }

    pub fn scroll(
        &mut self,
        top: i64,
        bot: i64,
        left: usize,
        right: usize,
        count: i64,
        default_hl: &Rc<Highlight>,
    ) {
        if count > 0 {
            for row in top..(bot - count + 1) {
                self.swap_row(row, count, left, right);
            }
        } else {
            for row in ((top - count)..(bot + 1)).rev() {
                self.swap_row(row, count, left, right);
            }
        }

        if count > 0 {
            self.clear_region(
                (bot - count + 1) as usize,
                bot as usize,
                left,
                right,
                default_hl,
            );
        } else {
            self.clear_region(
                top as usize,
                (top - count - 1) as usize,
                left,
                right,
                default_hl,
            );
        }
    }

    pub fn clear(&mut self, default_hl: &Rc<Highlight>) {
        let (rows, columns) = (self.rows, self.columns);
        self.clear_region(0, rows - 1, 0, columns - 1, default_hl);
    }

    fn clear_region(
        &mut self,
        top: usize,
        bot: usize,
        left: usize,
        right: usize,
        default_hl: &Rc<Highlight>,
    ) {
        for row in &mut self.model[top..bot + 1] {
            row.clear(left, right, default_hl);
        }
    }

    pub fn clear_glyphs(&mut self) {
        for row in &mut self.model.iter_mut() {
            row.clear_glyphs();
        }
    }

    #[inline]
    fn clamp_pos((row, col): (usize, usize), rows: usize, columns: usize) -> (usize, usize) {
        (
            row.min(rows.saturating_sub(1)),
            col.min(columns.saturating_sub(1)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_area() {
        let mut model = UiModel::new(10, 20);

        model.scroll(1, 5, 1, 5, 3, &Rc::new(Highlight::new()));
    }

    #[test]
    fn test_resize_preserves_existing_cells() {
        let hl = Rc::new(Highlight::new());
        let mut model = UiModel::new(2, 2);

        model.put_one(0, 0, "a", false, hl.clone());
        model.put_one(1, 1, "b", false, hl);
        model.set_cursor(1, 1);
        model.flush_cursor();

        model.resize(3, 4);

        assert_eq!(3, model.rows);
        assert_eq!(4, model.columns);
        assert_eq!("a", model.model[0].line[0].ch);
        assert_eq!("b", model.model[1].line[1].ch);
        assert_eq!("", model.model[2].line[0].ch);
        assert_eq!((1, 1), model.get_real_cursor());
        assert_eq!((1, 1), model.get_flushed_cursor());
    }

    #[test]
    fn test_resize_clamps_cursor_and_truncates_cells() {
        let hl = Rc::new(Highlight::new());
        let mut model = UiModel::new(3, 3);

        model.put_one(0, 0, "a", false, hl.clone());
        model.put_one(2, 2, "z", false, hl);
        model.set_cursor(2, 2);
        model.flush_cursor();

        model.resize(1, 1);

        assert_eq!(1, model.rows);
        assert_eq!(1, model.columns);
        assert_eq!("a", model.model[0].line[0].ch);
        assert_eq!((0, 0), model.get_real_cursor());
        assert_eq!((0, 0), model.get_flushed_cursor());
    }

    #[test]
    fn test_resize_noop_preserves_model_state() {
        let hl = Rc::new(Highlight::new());
        let mut model = UiModel::new(2, 2);

        model.put_one(1, 1, "x", false, hl);
        model.set_cursor(1, 1);
        model.flush_cursor();
        model.model[0].line[0].dirty = false;
        model.model[0].clear_dirty();

        let model_ptr = model.model.as_ptr();
        let line_ptr = model.model[0].line.as_ptr();

        model.resize(2, 2);

        assert_eq!(model_ptr, model.model.as_ptr());
        assert_eq!(line_ptr, model.model[0].line.as_ptr());
        assert_eq!(2, model.rows);
        assert_eq!(2, model.columns);
        assert_eq!("x", model.model[1].line[1].ch);
        assert!(!model.model[0].line[0].dirty);
        assert!(!model.model[0].is_dirty());
        assert_eq!((1, 1), model.get_real_cursor());
        assert_eq!((1, 1), model.get_flushed_cursor());
    }
}
