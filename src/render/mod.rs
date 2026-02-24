mod context;
mod itemize;

pub use self::context::{CellMetrics, Context, FontFeatures};

use log::warn;

use crate::{
    color,
    cursor::{Cursor, CursorRedrawCb, cursor_rect},
    highlight::HighlightMap,
    shell::TransparencySettings,
    ui_model,
};

use gtk::{graphene::Rect, prelude::*};

/// A single step in a render plan
#[derive(Clone, Copy)]
struct RenderStep<'a> {
    color: &'a color::Color,
    kind: RenderStepKind,
    len: usize,          // (in cells)
    pos: (usize, usize), // (rows, cols)
}

impl<'a> RenderStep<'a> {
    pub fn new(kind: RenderStepKind, color: &'a color::Color, pos: (usize, usize)) -> Self {
        Self {
            color,
            kind,
            pos,
            len: 1,
        }
    }

    // (until match_arm_wrapping stabilizes https://github.com/rust-lang/rustfmt/pull/4924 )
    #[rustfmt::skip]
    fn to_snapshot(
        self,
        snapshot: &gtk::Snapshot,
        cell_metrics: &CellMetrics
    ) {
        let pos = cell_metrics.get_pixel_coords(self.pos);
        let len = cell_metrics.get_cell_len(self.len);
        match self.kind {
            RenderStepKind::Background =>
                snapshot_bg(snapshot, cell_metrics, self.color, pos, len),
            RenderStepKind::Strikethrough =>
                snapshot_strikethrough(snapshot, cell_metrics, self.color, pos, len),
            RenderStepKind::Underline =>
                snapshot_underline(snapshot, cell_metrics, self.color, pos, len),
            RenderStepKind::Underdouble =>
                snapshot_underdouble(snapshot, cell_metrics, self.color, pos, len),
            RenderStepKind::Underdot =>
                snapshot_underdot(snapshot, cell_metrics, self.color, pos, len),
            RenderStepKind::Underdash =>
                snapshot_underdash(snapshot, cell_metrics, self.color, pos, len),
        }
    }

    #[inline]
    pub fn extend(&mut self, kind: RenderStepKind, color: &'a color::Color) -> bool {
        if kind == self.kind && *color == *self.color {
            self.len += 1;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderStepKind {
    Background,
    Underline,
    Underdouble,
    Underdot,
    Underdash,
    Strikethrough,
}

pub fn snapshot_nvim(
    font_ctx: &Context,
    ui_model: &ui_model::UiModel,
    hl: &HighlightMap,
) -> Option<gsk::RenderNode> {
    let snapshot = gtk::Snapshot::new();
    let cell_metrics = font_ctx.cell_metrics();
    let (rows, columns) = (ui_model.rows, ui_model.columns);

    // Various operations for text formatting come at the end, so store them in a list until then.
    // We set the capacity to three times the size of the grid, since at most each cell can have
    // strikethrough + a type of underline + the second line in an underdouble. Most of the time
    // though, we optimistically expect this list to be much smaller than iterating through the UI
    // model.
    let mut text_fmt_steps = Vec::with_capacity(columns * rows * 3);

    // Note that we group each batch of nodes based on their type, since GTK+ does a better job of
    // optimizing contiguous series of similar drawing operations (source: Company)
    let model = ui_model.model();
    for (row, line) in model.iter().enumerate() {
        let mut pending_bg = None;
        let mut pending_strikethrough = None;
        let mut pending_underline = None;
        let mut pending_underdouble = None;

        for (col, cell) in line.line.iter().enumerate() {
            // Plan each step of the process of creating this snapshot as much as possible. We use
            // the term "plan" to describe the process of generating as few snapshot nodes as
            // possible in order to accomplish each step in creating a snapshot for the final image.
            // For example, if multiple adjacent background nodes have identical background colors -
            // our planning phase will generate a single snapshot node for any contiguous group of
            // cells. Where possible, we immediately generate nodes and add them to the snapshot.
            //
            // Currently, the only step of the rendering process we don't do this for is with
            // text nodes - where we rely on the itemization process to have already done this for
            // us. Additionally, all optimizations are limited to each row. We do not for instance,
            // combine the background nodes of multiple identical adjacent rows.
            plan_and_snapshot_cell_bg(
                &snapshot,
                &mut pending_bg,
                hl,
                cell,
                cell_metrics,
                (row, col),
            );
            plan_underline_strikethrough(
                &mut pending_strikethrough,
                &mut pending_underline,
                &mut pending_underdouble,
                &mut text_fmt_steps,
                hl,
                cell,
                (row, col),
            );
        }

        // Since background nodes come first, we can add them to the snapshot immediately
        if let Some(pending_bg) = pending_bg {
            pending_bg.to_snapshot(&snapshot, cell_metrics);
        }
    }

    for (row, line) in model.iter().enumerate() {
        for (col, cell) in line.line.iter().enumerate() {
            snapshot_cell(
                &snapshot,
                &line.item_line[col],
                hl,
                cell,
                (row, col),
                cell_metrics,
            );
        }
    }

    for step in text_fmt_steps.into_iter() {
        step.to_snapshot(&snapshot, cell_metrics);
    }

    snapshot.to_node()
}

pub fn snapshot_cursor<T: CursorRedrawCb + 'static>(
    snapshot: &gtk::Snapshot,
    cursor: &Cursor<T>,
    font_ctx: &Context,
    ui_model: &ui_model::UiModel,
    hl: &HighlightMap,
    transparency: TransparencySettings,
) {
    if !cursor.is_visible() {
        return;
    }

    let cell_metrics = font_ctx.cell_metrics();
    let CellMetrics {
        ascent, char_width, ..
    } = *cell_metrics;
    let (cursor_row, cursor_col) = ui_model.get_flushed_cursor();
    let (x, y) = cell_metrics.get_pixel_coords((cursor_row, cursor_col));

    let cursor_line = match ui_model.model().get(cursor_row) {
        Some(cursor_line) => cursor_line,
        None => return,
    };

    let next_cell = cursor_line.line.get(cursor_col + 1);
    let double_width = next_cell.is_some_and(|c| c.double_width);
    let fade_percentage = cursor.alpha();
    let cell = &cursor_line.line[cursor_col];

    let (clip_y, clip_width, clip_height) =
        cursor_rect(cursor.mode_info(), cell_metrics, y, double_width);
    let clip_rect = Rect::new(
        x as f32,
        clip_y as f32,
        clip_width as f32,
        clip_height as f32,
    );

    let bg_alpha = transparency.background_alpha;
    let filled_alpha = transparency.filled_alpha;
    let alpha = bg_alpha + ((filled_alpha - bg_alpha) * fade_percentage);
    let is_focused = cursor.is_focused();
    if is_focused {
        snapshot.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0), &clip_rect);
    }

    cursor.snapshot(
        snapshot,
        font_ctx,
        (x, y),
        cell,
        double_width,
        hl,
        fade_percentage,
        alpha,
    );

    // Skip re-rendering cell if it isn't needed
    if !is_focused {
        return;
    }

    let cell_start_col = cursor_line.cell_to_item(cursor_col);
    let fg = hl.actual_cell_fg(cell).fade(hl.bg(), fade_percentage);

    if cell_start_col >= 0 {
        snapshot.push_clip(&clip_rect);

        let cell_start_line_x = cell_start_col as f64 * char_width;
        for item in &*cursor_line.item_line[cell_start_col as usize] {
            if item.glyphs().is_some()
                && let Some(ref render_node) =
                    item.new_render_node(&fg, (cell_start_line_x as f32, (y + ascent) as f32))
            {
                snapshot.append_node(render_node);
            }
        }

        snapshot.pop();
    }

    if cell.hl.strikethrough {
        snapshot_strikethrough(snapshot, cell_metrics, &fg, (x, y), clip_width);
    }

    if cell.hl.underdashed {
        snapshot_underdash(
            snapshot,
            cell_metrics,
            &underline_color(cell, hl).fade(hl.bg(), fade_percentage),
            (x, y),
            clip_width,
        );
    } else if cell.hl.underdotted {
        snapshot_underdot(
            snapshot,
            cell_metrics,
            &underdotted_color(cell, hl).fade(hl.bg(), fade_percentage),
            (x, y),
            clip_width,
        );
    } else if cell.hl.underline {
        snapshot_underline(
            snapshot,
            cell_metrics,
            &underline_color(cell, hl).fade(hl.bg(), fade_percentage),
            (x, y),
            clip_width,
        );
    }

    if cell.hl.underdouble {
        snapshot_underdouble(
            snapshot,
            cell_metrics,
            &underline_color(cell, hl).fade(hl.bg(), fade_percentage),
            (x, y),
            clip_width,
        );
    }
}

fn snapshot_strikethrough(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    (x, y): (f64, f64),
    len: f64,
) {
    snapshot.append_color(
        &color.into(),
        &Rect::new(
            x as f32,
            (y + cell_metrics.strikethrough_position) as f32,
            len as f32,
            cell_metrics.strikethrough_thickness as f32,
        ),
    )
}

fn underline_rect(cell_metrics: &CellMetrics, (x, y): (f64, f64), len: f64) -> Rect {
    Rect::new(
        x as f32,
        (y + cell_metrics.underline_position) as f32,
        len as f32,
        cell_metrics.underline_thickness as f32,
    )
}

fn snapshot_underline(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    (x, y): (f64, f64),
    len: f64,
) {
    snapshot.append_color(&color.into(), &underline_rect(cell_metrics, (x, y), len))
}

fn snapshot_underdouble(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    pos: (f64, f64),
    len: f64,
) {
    /* We only need to handle the lower underline, the upper underline will be drawn by
     * snapshot_underline()
     */
    snapshot.append_color(
        &color.into(),
        &underline_rect(cell_metrics, pos, len)
            .offset_r(0.0, (cell_metrics.underline_thickness * 2.0) as f32),
    )
}

fn snapshot_underdot(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    (x, mut y): (f64, f64),
    len: f64,
) {
    let CellMetrics {
        underline_position,
        /* Ideally we always want to make sure that the underdot comes out as a distinct set of
         * repeating dots, always equally spaced, and as large as possible within the descent area
         * starting from the underline position to the bottom of the line.
         */
        underline_thickness: diameter,
        ..
    } = *cell_metrics;

    /* We also want to make sure that each dot starts on an X coordinate that's a multiple of the
     * width of the segment that we'll be repeating, in order to avoid the spacing between dots of
     * different colors from ever looking inconsistent. This can mean we'll sometimes only draw a
     * portion of a dot, but that generally looks nicer then the inconsistent alternative.
     */
    let start_x = x - (x % (diameter * 2.0));

    y = (y + underline_position).floor();

    let rect = Rect::new(x as f32, y as f32, len as f32, diameter as f32);
    snapshot.push_repeat(
        &rect,
        Some(&Rect::new(
            start_x as f32,
            y as f32,
            (diameter * 2.0) as f32,
            diameter as f32,
        )),
    );

    let dot = gsk::RoundedRect::from_rect(
        Rect::new(start_x as f32, y as f32, diameter as f32, diameter as f32),
        (diameter / 2.0) as f32,
    );
    snapshot.push_rounded_clip(&dot);
    snapshot.append_color(&color.into(), dot.bounds());
    snapshot.pop();

    snapshot.pop();
}

fn snapshot_underdash(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    (x, mut y): (f64, f64),
    len: f64,
) {
    let CellMetrics {
        underline_position,
        underline_thickness,
        char_width,
        ..
    } = *cell_metrics;

    /* Same trick as in snapshot_underdot(). Note that we need to make sure to round the dash width
     * though, because otherwise subpixel rendering and floating point imprecision will cause dashes
     * to become misaligned as they're repeated
     *
     * It would be nice not to have to round them though 🙄
     */
    let dash_width = (char_width / 3.0).round();
    let pattern_width = dash_width * 2.0;
    let start_x = x - (x % pattern_width);

    y = (y + underline_position).floor();

    let (x, start_x, y, len, underline_thickness) = (
        x as f32,
        start_x as f32,
        y as f32,
        len as f32,
        underline_thickness as f32,
    );

    snapshot.push_repeat(
        &Rect::new(x, y, len, underline_thickness),
        Some(&Rect::new(
            start_x,
            y,
            (dash_width * 2.0) as f32,
            underline_thickness,
        )),
    );
    snapshot.append_color(
        &color.into(),
        &Rect::new(start_x, y, dash_width as f32, underline_thickness),
    );
    snapshot.pop();
}

fn plan_and_snapshot_cell_bg<'a>(
    snapshot: &gtk::Snapshot,
    pending_bg: &mut Option<RenderStep<'a>>,
    hl: &'a HighlightMap,
    cell: &'a ui_model::Cell,
    cell_metrics: &CellMetrics,
    (row, col): (usize, usize),
) {
    if let Some(cell_bg) = hl.cell_bg(cell).filter(|bg| *bg != hl.bg()) {
        if let Some(cur_pending_bg) = pending_bg {
            if cur_pending_bg.extend(RenderStepKind::Background, cell_bg) {
                return;
            }
            cur_pending_bg.to_snapshot(snapshot, cell_metrics);
        }
        *pending_bg = Some(RenderStep::new(
            RenderStepKind::Background,
            cell_bg,
            (row, col),
        ));
    } else if let Some(pending_bg) = pending_bg.take() {
        pending_bg.to_snapshot(snapshot, cell_metrics);
    }
}

fn underline_color<'a>(cell: &'a ui_model::Cell, hl: &'a HighlightMap) -> &'a color::Color {
    hl.cell_sp(cell).unwrap_or_else(|| hl.actual_cell_fg(cell))
}

fn underdotted_color<'a>(cell: &'a ui_model::Cell, hl: &'a HighlightMap) -> &'a color::Color {
    hl.cell_sp(cell).unwrap_or(&color::COLOR_RED)
}

fn plan_underline_strikethrough<'a>(
    pending_strikethrough: &mut Option<usize>,
    pending_underline: &mut Option<usize>,
    pending_underdouble: &mut Option<usize>,
    pending_fmt_ops: &mut Vec<RenderStep<'a>>,
    hl: &'a HighlightMap,
    cell: &'a ui_model::Cell,
    pos: (usize, usize),
) {
    if cell.hl.strikethrough {
        let fg = hl.actual_cell_fg(cell);
        let mut extended = false;
        if let Some(idx) = *pending_strikethrough {
            extended = pending_fmt_ops[idx].extend(RenderStepKind::Strikethrough, fg);
        }

        if !extended {
            *pending_strikethrough = Some(pending_fmt_ops.len());
            pending_fmt_ops.push(RenderStep::new(RenderStepKind::Strikethrough, fg, pos));
        }
    } else {
        *pending_strikethrough = None;
    }

    let (kind, color) = if cell.hl.underdashed {
        (RenderStepKind::Underdash, underline_color(cell, hl))
    } else if cell.hl.underdotted {
        (RenderStepKind::Underdot, underdotted_color(cell, hl))
    } else if cell.hl.underline {
        (RenderStepKind::Underline, underline_color(cell, hl))
    } else {
        *pending_underline = None;
        *pending_underdouble = None;
        return;
    };

    let mut extended = false;
    if let Some(idx) = *pending_underline {
        extended = pending_fmt_ops[idx].extend(kind, color);
    }

    if !extended {
        *pending_underline = Some(pending_fmt_ops.len());
        pending_fmt_ops.push(RenderStep::new(kind, color, pos));
    }

    if cell.hl.underdouble {
        if let Some(idx) = *pending_underdouble
            && pending_fmt_ops[idx].extend(RenderStepKind::Underdouble, color)
        {
            return;
        }
        *pending_underdouble = Some(pending_fmt_ops.len());
        pending_fmt_ops.push(RenderStep::new(RenderStepKind::Underdouble, color, pos));
    }
}

#[inline]
fn snapshot_bg(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    (x, y): (f64, f64),
    len: f64,
) {
    snapshot.append_color(
        &color.into(),
        &Rect::new(
            x as f32,
            y as f32,
            len as f32,
            cell_metrics.line_height as f32,
        ),
    )
}

/// Generate render nodes for the current cell
fn snapshot_cell(
    snapshot: &gtk::Snapshot,
    items: &[ui_model::Item],
    hl: &HighlightMap,
    cell: &ui_model::Cell,
    pos: (usize, usize),
    cell_metrics: &CellMetrics,
) {
    let (x, y) = cell_metrics.get_pixel_coords(pos);
    for item in items {
        let fg = hl.actual_cell_fg(cell);

        if item.glyphs().is_some()
            && let Some(render_node) =
                item.render_node(fg, (x as f32, (y + cell_metrics.ascent) as f32))
        {
            snapshot.append_node(render_node);
        }
    }
}

pub fn shape_dirty(ctx: &context::Context, ui_model: &mut ui_model::UiModel, hl: &HighlightMap) {
    for line in ui_model.model_mut() {
        if !line.dirty_line {
            continue;
        }

        let styled_line = ui_model::StyledLine::from(line, hl, ctx.font_features());
        let items = ctx.itemize(&styled_line);
        line.merge(&styled_line, &items);

        for (col, cell) in line.line.iter_mut().enumerate() {
            if cell.dirty {
                for item in &mut *line.item_line[col] {
                    let mut glyphs = pango::GlyphString::new();
                    {
                        let analysis = item.analysis();
                        let offset = item.item.offset() as usize;
                        let length = item.item.length() as usize;
                        if let Some(line_str) = styled_line.line_str.get(offset..offset + length) {
                            pango::shape(line_str, analysis, &mut glyphs);
                        } else {
                            warn!("Wrong itemize split");
                        }
                    }

                    item.set_glyphs(glyphs);
                }
            }

            cell.dirty = false;
        }

        line.dirty_line = false;
    }
}
