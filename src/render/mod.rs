mod context;
mod itemize;
mod model_clip_iterator;

pub use self::context::{CellMetrics, Context, FontFeatures};

use crate::{
    color,
    cursor::{cursor_rect, Cursor},
    highlight::HighlightMap,
    ui_model,
};
use pango;
use gtk::graphene::Rect;

pub fn snapshot_nvim(
    font_ctx: &Context,
    ui_model: &mut ui_model::UiModel,
    hl: &HighlightMap,
    bg_alpha: f64,
) -> gsk::RenderNode {
    let snapshot = gtk::Snapshot::new();
    let cell_metrics = font_ctx.cell_metrics();
    let &CellMetrics { char_width, line_height, .. } = cell_metrics;
    let model = ui_model.model_mut();

    // We group each batch of nodes based on their type, since GTK+ does a better job of
    // optimizing contiguous series of similar drawing operations (source: Company)
    let mut line_x;
    let mut line_y = 0.0;
    for line in model.iter() {
        line_x = 0.0;
        for (col, cell) in line.line.iter().enumerate() {
            let pos = (line_x, line_y);

            snapshot_cell_bg(&snapshot, line, hl, cell, col, pos, cell_metrics, bg_alpha);
            line_x += char_width as f32;
        }
        line_y += line_height as f32;
    }

    line_y = 0.0;
    for line in model.iter_mut() {
        line_x = 0.0;
        for (col, cell) in line.line.iter().enumerate() {
            let pos = (line_x, line_y);
            let items = &mut *line.item_line[col];

            snapshot_cell(&snapshot, items, hl, cell, pos, cell_metrics);
            line_x += char_width as f32;
        }
        line_y += line_height as f32;
    }

    line_y = 0.0;
    for line in model.iter() {
        line_x = 0.0;
        for cell in line.line.iter() {
            let pos = (line_x, line_y);

            snapshot_underline_strikethrough(&snapshot, hl, cell, pos, cell_metrics, 0.0);
            line_x += char_width as f32;
        }
        line_y += line_height as f32;
    }

    snapshot.to_node().expect("Render node creation shouldn't fail")
}

pub fn snapshot_cursor<C: Cursor>(
    snapshot: &gtk::Snapshot,
    cursor: &C,
    font_ctx: &Context,
    ui_model: &ui_model::UiModel,
    hl: &HighlightMap,
) {
    if !cursor.is_visible() {
        return;
    }

    let cell_metrics = font_ctx.cell_metrics();
    let CellMetrics {
        ascent,
        char_width,
        line_height,
        ..
    } = *cell_metrics;
    let (cursor_row, cursor_col) = ui_model.get_cursor();

    let x = cursor_col as f64 * char_width;
    let y = cursor_row as f64 * line_height;

    let cursor_line = match ui_model.model().get(cursor_row) {
        Some(cursor_line) => cursor_line,
        None => return,
    };

    let next_cell = cursor_line.line.get(cursor_col + 1);
    let double_width = next_cell.map_or(false, |c| c.double_width);
    let alpha = cursor.alpha();
    let cell = &cursor_line.line[cursor_col];

    // Skip re-rendering cell if it isn't needed
    if !cursor.snapshot(snapshot, font_ctx, (x, y), cell, double_width, &hl, alpha) {
        return;
    }

    let (clip_y, clip_width, clip_height) =
        cursor_rect(cursor.mode_info(), cell_metrics, y, double_width);
    let clip_rect = Rect::new(
        x as f32,
        clip_y as f32,
        clip_width as f32,
        clip_height as f32,
    );

    let cell_start_col = cursor_line.cell_to_item(cursor_col);
    if cell_start_col >= 0 {
        snapshot.push_clip(&clip_rect);

        let fg = hl.actual_cell_fg(cell).fade(hl.bg(), alpha);
        let fg = fg.as_ref().into();

        let cell_start_line_x = cell_start_col as f64 * char_width;
        for item in &*cursor_line.item_line[cell_start_col as usize] {
            if item.glyphs().is_some() {
                snapshot.append_node(item.new_render_node(
                    fg, (cell_start_line_x as f32, (y + ascent) as f32)
                ));
            }
        }

        let mut pos = (x as f32, y as f32);
        snapshot_underline_strikethrough(snapshot, hl, cell, pos, cell_metrics, alpha);
        if let Some(next_cell) = next_cell {
            if double_width {
                pos.0 += char_width as f32;
                snapshot_underline_strikethrough(
                    snapshot, hl, next_cell, pos, cell_metrics, alpha
                );
            }
        }

        snapshot.pop();
    }
}

/* TODO: Come up with a struct to keep track of cells whose underlines can be combined into one
 * operation
 */
fn snapshot_underline_strikethrough(
    snapshot: &gtk::Snapshot,
    hl: &HighlightMap,
    cell: &ui_model::Cell,
    (x, mut y): (f32, f32),
    cell_metrics: &CellMetrics,
    inverse_level: f64,
) {
    let &CellMetrics {
        mut char_width,
        strikethrough_position,
        strikethrough_thickness,
        underline_position,
        underline_thickness,
        ..
    } = cell_metrics;
    char_width = char_width.ceil();
    let bg = hl.bg();

    if cell.hl.strikethrough {
        let fg = hl.actual_cell_fg(cell).fade(bg, inverse_level);

        snapshot.append_color(
            &fg.as_ref().into(),
            &Rect::new(
                x,
                y + strikethrough_position as f32,
                char_width as f32,
                strikethrough_thickness as f32,
            )
        );
    }

    y += underline_position as f32;
    let rect = Rect::new(x, y, char_width as f32, underline_thickness as f32);
    if cell.hl.undercurl {
        let sp = hl
            .cell_sp(cell)
            .unwrap_or(&color::COLOR_RED)
            .fade(bg, inverse_level)
            .as_ref()
            .into();

        let width = (char_width / 6.0).min(underline_thickness);
        let seg_rect = Rect::new(
            x - (width / 2.0) as f32,
            y,
            width as f32,
            underline_thickness as f32
        );
        let mut dot = gsk::RoundedRect::from_rect(seg_rect, (underline_thickness / 2.0) as f32);

        snapshot.push_repeat(&rect, None);
        snapshot.push_rounded_clip(&dot);
        snapshot.append_color(&sp, dot.bounds());
        snapshot.pop();

        // TODO: figure out if we can get rid of this, we really just need some way to express
        // that we want to repeat an area of (2x, y)
        dot.offset(dot.bounds().width(), 0.0);
        snapshot.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0), dot.bounds());
        snapshot.pop();
    } else if cell.hl.underline {
        let sp = hl
            .cell_sp(cell)
            .unwrap_or_else(|| hl.actual_cell_fg(cell))
            .fade(bg, inverse_level);

        snapshot.append_color(&sp.as_ref().into(), &rect);
    }
}

fn snapshot_cell_bg(
    snapshot: &gtk::Snapshot,
    line: &ui_model::Line,
    hl: &HighlightMap,
    cell: &ui_model::Cell,
    col: usize,
    (line_x, line_y): (f32, f32),
    cell_metrics: &CellMetrics,
    bg_alpha: f64,
) {
    let &CellMetrics {
        char_width,
        line_height,
        ..
    } = cell_metrics;

    if let Some(bg) = hl.cell_bg(cell) {
        if !line.is_binded_to_item(col) {
            if bg != hl.bg() {
                snapshot.append_color(
                    &bg.to_rgbo(bg_alpha),
                    &Rect::new(line_x, line_y, char_width.ceil() as f32, line_height as f32)
                );
            }
        } else {
            snapshot.append_color(
                &bg.to_rgbo(bg_alpha),
                &Rect::new(
                    line_x,
                    line_y,
                    (char_width * line.item_len_from_idx(col) as f64) as f32,
                    line_height as f32
                )
            );
        }
    }
}

/// Generate render nodes for the current cell
fn snapshot_cell(
    snapshot: &gtk::Snapshot,
    items: &mut [ui_model::Item],
    hl: &HighlightMap,
    cell: &ui_model::Cell,
    (x, y): (f32, f32),
    cell_metrics: &CellMetrics,
) {
    for item in items {
        let fg = hl.actual_cell_fg(cell);

        if item.glyphs().is_some() {
            snapshot.append_node(item.render_node(
                &fg, (x, y + cell_metrics.ascent as f32)
            ));
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
                            pango::shape(&line_str, analysis, &mut glyphs);
                        } else {
                            warn!("Wrong itemize split");
                        }
                    }

                    item.set_glyphs(ctx, glyphs);
                }
            }

            cell.dirty = false;
        }

        line.dirty_line = false;
    }
}
