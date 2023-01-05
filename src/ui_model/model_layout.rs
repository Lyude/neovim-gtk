use std::cmp::max;
use std::rc::Rc;

use unicode_width::UnicodeWidthStr;

use crate::highlight::Highlight;
use crate::ui_model::UiModel;

#[derive(Clone)]
pub struct HighlightedRange {
    pub highlight: Rc<Highlight>,
    pub graphemes: Vec<String>,
}

impl HighlightedRange {
    pub fn new(highlight: Rc<Highlight>, graphemes: Vec<String>) -> Self {
        Self {
            highlight,
            graphemes,
        }
    }
}

pub type HighlightedLine = Vec<HighlightedRange>;

pub struct ModelLayout {
    pub model: UiModel,
    rows_filled: usize,
    cols_filled: usize,
    lines: Vec<HighlightedLine>,
}

impl ModelLayout {
    const ROWS_STEP: usize = 10;

    pub fn new(columns: u64) -> Self {
        ModelLayout {
            model: UiModel::new(ModelLayout::ROWS_STEP as u64, columns),
            rows_filled: 0,
            cols_filled: 0,
            lines: Vec::new(),
        }
    }

    pub fn layout_append(&mut self, mut lines: Vec<HighlightedLine>) {
        let rows_filled = self.rows_filled;
        let take_from = self.lines.len();

        self.lines.append(&mut lines);

        self.layout_replace(rows_filled, take_from);
    }

    pub fn layout(&mut self, lines: Vec<HighlightedLine>) {
        self.lines = lines;
        self.layout_replace(0, 0);
    }

    pub fn set_cursor(&mut self, col: usize) {
        let row = if self.rows_filled > 0 {
            self.rows_filled - 1
        } else {
            0
        };

        self.model.set_cursor(row, col);
    }

    pub fn size(&self) -> (usize, usize) {
        (
            max(self.cols_filled, self.model.get_real_cursor().1 + 1),
            self.rows_filled,
        )
    }

    fn check_model_size(&mut self, rows: usize) {
        if rows > self.model.rows {
            let model_cols = self.model.columns;
            let model_rows = ((rows / (ModelLayout::ROWS_STEP + 1)) + 1) * ModelLayout::ROWS_STEP;
            let (cur_row, cur_col) = self.model.get_real_cursor();

            let mut model = UiModel::new(model_rows as u64, model_cols as u64);
            self.model.swap_rows(&mut model, self.rows_filled - 1);
            model.set_cursor(cur_row, cur_col);
            self.model = model;
        }
    }

    pub fn insert_char(&mut self, ch: String, shift: bool, hl: Rc<Highlight>) {
        if ch.is_empty() {
            return;
        }

        let (row, col) = self.model.get_real_cursor();

        if shift {
            self.insert_into_lines(ch);
            self.layout_replace(0, 0);
        } else {
            self.model.put_one(row, col, &ch, false, hl);
        }
    }

    fn insert_into_lines(&mut self, ch: String) {
        let highlight_ranges = &mut self.lines[0];

        let (_, cur_col) = self.model.get_real_cursor();

        let mut col_idx = 0;
        for range in highlight_ranges.iter_mut() {
            if cur_col < col_idx + range.graphemes.len() {
                let col_sub_idx = cur_col - col_idx;
                range.graphemes.insert(col_sub_idx, ch);
                break;
            } else {
                col_idx += range.graphemes.len();
            }
        }
    }

    /// Wrap all lines into model
    ///
    /// returns actual width
    fn layout_replace(&mut self, row_offset: usize, take_from: usize) {
        let rows = ModelLayout::count_lines(&self.lines[take_from..], self.model.columns);

        self.check_model_size(rows + row_offset);
        self.rows_filled = rows + row_offset;

        let lines = &self.lines[take_from..];

        let mut max_col_idx = 0;
        let mut col_idx = 0;
        let mut row_idx = row_offset;
        for highlight_ranges in lines {
            for HighlightedRange {
                highlight: hl,
                graphemes: ch_list,
            } in highlight_ranges
            {
                for ch in ch_list {
                    let ch_width = max(1, ch.width());

                    if col_idx + ch_width > self.model.columns {
                        col_idx = 0;
                        row_idx += 1;
                    }

                    self.model.put_one(row_idx, col_idx, ch, false, hl.clone());
                    if ch_width > 1 {
                        self.model.put_one(row_idx, col_idx, "", true, hl.clone());
                    }

                    if max_col_idx < col_idx {
                        max_col_idx = col_idx + ch_width - 1;
                    }

                    col_idx += ch_width;
                }

                if col_idx < self.model.columns {
                    self.model.model[row_idx].clear(
                        col_idx,
                        self.model.columns - 1,
                        &Rc::new(Highlight::new()),
                    );
                }
            }
            col_idx = 0;
            row_idx += 1;
        }

        if self.rows_filled == 1 {
            self.cols_filled = max_col_idx + 1;
        } else {
            self.cols_filled = max(self.cols_filled, max_col_idx + 1);
        }
    }

    fn count_lines(lines: &[HighlightedLine], max_columns: usize) -> usize {
        let mut row_count = 0;

        for line in lines {
            let len: usize = line.iter().map(|range| range.graphemes.len()).sum();
            row_count += len / (max_columns + 1) + 1;
        }

        row_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_lines() {
        let lines = vec![vec![HighlightedRange {
            highlight: Rc::new(Highlight::new()),
            graphemes: vec!["a".to_owned(); 5],
        }]];

        let rows = ModelLayout::count_lines(&lines, 4);
        assert_eq!(2, rows);
    }

    #[test]
    fn test_resize() {
        let lines = vec![
            vec![HighlightedRange {
                highlight: Rc::new(Highlight::new()),
                graphemes: vec!["a".to_owned(); 5]
            }];
            ModelLayout::ROWS_STEP
        ];
        let mut model = ModelLayout::new(5);

        model.layout(lines.clone());
        let (cols, rows) = model.size();
        assert_eq!(5, cols);
        assert_eq!(ModelLayout::ROWS_STEP, rows);

        model.layout_append(lines);
        let (cols, rows) = model.size();
        assert_eq!(5, cols);
        assert_eq!(ModelLayout::ROWS_STEP * 2, rows);
        assert_eq!(ModelLayout::ROWS_STEP * 2, model.model.rows);
    }

    #[test]
    fn test_cols_filled() {
        let lines = vec![
            vec![HighlightedRange::new(
                Rc::new(Highlight::new()),
                vec!["a".to_owned(); 3]
            )];
            1
        ];
        let mut model = ModelLayout::new(5);

        model.layout(lines);
        // cursor is not moved by newgrid api
        // so set it manual
        model.set_cursor(3);
        let (cols, _) = model.size();
        assert_eq!(4, cols); // size is 3 and 4 - is with cursor position

        let lines = vec![
            vec![HighlightedRange::new(
                Rc::new(Highlight::new()),
                vec!["a".to_owned(); 2]
            )];
            1
        ];

        model.layout_append(lines);
        model.set_cursor(2);
        let (cols, _) = model.size();
        assert_eq!(3, cols);
    }

    #[test]
    fn test_insert_shift() {
        let lines = vec![
            vec![HighlightedRange::new(
                Rc::new(Highlight::new()),
                vec!["a".to_owned(); 3]
            )];
            1
        ];
        let mut model = ModelLayout::new(5);
        model.layout(lines);
        model.set_cursor(1);

        model.insert_char("b".to_owned(), true, Rc::new(Highlight::new()));

        let (cols, _) = model.size();
        assert_eq!(4, cols);
        assert_eq!("b", model.model.model()[0].line[1].ch);
    }

    #[test]
    fn test_insert_no_shift() {
        let lines = vec![
            vec![HighlightedRange::new(
                Rc::new(Highlight::new()),
                vec!["a".to_owned(); 3]
            )];
            1
        ];
        let mut model = ModelLayout::new(5);
        model.layout(lines);
        model.set_cursor(1);

        model.insert_char("b".to_owned(), false, Rc::new(Highlight::new()));

        let (cols, _) = model.size();
        assert_eq!(3, cols);
        assert_eq!("b", model.model.model()[0].line[1].ch);
    }

    #[test]
    fn test_double_width() {
        let lines = vec![
            vec![HighlightedRange::new(
                Rc::new(Highlight::new()),
                vec!["あ".to_owned(); 3]
            )];
            1
        ];
        let mut model = ModelLayout::new(7);
        model.layout(lines);
        model.set_cursor(1);

        let (cols, rows) = model.size();
        assert_eq!(1, rows);
        assert_eq!(6, cols);
    }
}
