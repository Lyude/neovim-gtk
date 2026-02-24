use super::UiModel;
use crate::render::CellMetrics;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModelRect {
    pub top: usize,
    pub bot: usize,
    pub left: usize,
    pub right: usize,
}

impl ModelRect {
    pub fn new(top: usize, bot: usize, left: usize, right: usize) -> ModelRect {
        // FIXME: Add back formatted messages here once new clippy updates propagate downstream
        debug_assert!(top <= bot);
        debug_assert!(left <= right);

        ModelRect {
            top,
            bot,
            left,
            right,
        }
    }

    pub fn point(x: usize, y: usize) -> ModelRect {
        ModelRect {
            top: y,
            bot: y,
            left: x,
            right: x,
        }
    }

    /// Extend rect to left and right to make changed Item rerendered
    pub fn extend_by_items(&mut self, model: Option<&UiModel>) {
        if model.is_none() {
            return;
        }
        let model = model.unwrap();

        let mut left = self.left;
        let mut right = self.right;

        for i in self.top..self.bot + 1 {
            let line = &model.model[i];
            let item_idx = line.cell_to_item(self.left);
            if item_idx >= 0 {
                let item_idx = item_idx as usize;
                if item_idx < left {
                    left = item_idx;
                }
            }

            let len_since_right = line.item_len_from_idx(self.right) - 1;
            if right < self.right + len_since_right {
                right = self.right + len_since_right;
            }

            // extend also double_width chars
            let cell = &line.line[self.left];
            if self.left > 0 && cell.double_width {
                let dw_char_idx = self.left - 1;
                if dw_char_idx < left {
                    left = dw_char_idx;
                }
            }

            let dw_char_idx = self.right + 1;
            if let Some(cell) = line.line.get(dw_char_idx)
                && cell.double_width
                && right < dw_char_idx
            {
                right = dw_char_idx;
            }
        }

        self.left = left;
        self.right = right;
    }

    pub fn join(&mut self, rect: &ModelRect) {
        self.top = if self.top < rect.top {
            self.top
        } else {
            rect.top
        };
        self.left = if self.left < rect.left {
            self.left
        } else {
            rect.left
        };

        self.bot = if self.bot > rect.bot {
            self.bot
        } else {
            rect.bot
        };
        self.right = if self.right > rect.right {
            self.right
        } else {
            rect.right
        };

        debug_assert!(self.top <= self.bot);
        debug_assert!(self.left <= self.right);
    }

    pub fn to_area(&self, cell_metrics: &CellMetrics) -> (i32, i32, i32, i32) {
        let &CellMetrics {
            char_width,
            line_height,
            ..
        } = cell_metrics;

        // when convert to i32 area must be bigger then original f64 version
        (
            (self.left as f64 * char_width).floor() as i32,
            (self.top as f64 * line_height).floor() as i32,
            ((self.right - self.left + 1) as f64 * char_width).ceil() as i32,
            ((self.bot - self.top + 1) as f64 * line_height).ceil() as i32,
        )
    }
}

impl AsRef<ModelRect> for ModelRect {
    fn as_ref(&self) -> &ModelRect {
        self
    }
}
