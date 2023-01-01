use std::collections::HashSet;

use pango::{self, prelude::*};

use super::itemize::ItemizeIterator;
use crate::ui_model::StyledLine;

pub struct Context {
    font_metrics: FontMetrix,
    font_features: FontFeatures,
    line_space: i32,
}

impl Context {
    pub fn new(pango_context: pango::Context) -> Self {
        Context {
            line_space: 0,
            font_metrics: FontMetrix::new(pango_context, 0),
            font_features: FontFeatures::new(),
        }
    }

    pub fn update(&mut self, pango_context: pango::Context) {
        self.font_metrics = FontMetrix::new(pango_context, self.line_space);
    }

    pub fn update_font_features(&mut self, font_features: FontFeatures) {
        self.font_features = font_features;
    }

    pub fn update_line_space(&mut self, line_space: i32) {
        self.line_space = line_space;
        let pango_context = self.font_metrics.pango_context.clone();
        self.font_metrics = FontMetrix::new(pango_context, self.line_space);
    }

    pub fn itemize(&self, line: &StyledLine) -> Vec<pango::Item> {
        let attr_iter = line.attr_list.iterator();

        ItemizeIterator::new(&line.line_str)
            .flat_map(|res| {
                let pango_context = &self.font_metrics.pango_context;
                let offset = res.offset as i32;
                let len = res.len as i32;

                let first_res = pango::itemize(
                    pango_context,
                    &line.line_str,
                    offset,
                    len,
                    &line.attr_list,
                    Some(&attr_iter),
                );

                if !res.avoid_break || first_res.len() == 1 {
                    return first_res;
                }

                /* If we get multiple items, and it isn't from a multi-character ASCII string, then
                 * it's likely from an additional split pango had to perform because not all chars
                 * in the string were available in the current font. When this happens, in order to
                 * ensure combining characters are rendered correctly we need to try reitemizing the
                 * whole thing with the font containing the missing glyphs. Failing that, we
                 * fallback to the original (likely incorrect) itemization result.
                 */
                let our_font = self.font_description();
                let extra_fonts = first_res.iter().filter_map(|i| {
                    let font = i.analysis().font().describe();
                    if font != *our_font {
                        Some(font)
                    } else {
                        None
                    }
                });

                // We do res.len() - 2 so that in the likely event that most of the Cell rendered
                // with our_font, and the rest with another, we're able to skip allocating the
                // HashSet completely.
                let mut seen = HashSet::with_capacity(first_res.len() - 2);
                let mut new_res = None;
                for font_desc in extra_fonts {
                    if seen.contains(&font_desc) {
                        continue;
                    }

                    pango_context.set_font_description(Some(&font_desc));
                    let res = pango::itemize(
                        pango_context,
                        &line.line_str,
                        offset,
                        len,
                        &line.attr_list,
                        None,
                    );

                    let len = res.len();
                    if len == 1 || len < new_res.as_ref().unwrap_or(&first_res).len() {
                        new_res = Some(res);
                        if len == 1 {
                            break;
                        }
                    }
                    seen.insert(font_desc);
                }

                pango_context.set_font_description(Some(our_font));
                new_res.unwrap_or(first_res)
            })
            .collect()
    }

    pub fn create_layout(&self) -> pango::Layout {
        pango::Layout::new(&self.font_metrics.pango_context)
    }

    pub fn font_description(&self) -> &pango::FontDescription {
        &self.font_metrics.font_desc
    }

    pub fn cell_metrics(&self) -> &CellMetrics {
        &self.font_metrics.cell_metrics
    }

    pub fn font_features(&self) -> &FontFeatures {
        &self.font_features
    }

    pub fn font_families(&self) -> HashSet<glib::GString> {
        self.font_metrics
            .pango_context
            .list_families()
            .iter()
            .map(|f| f.name())
            .collect()
    }
}

struct FontMetrix {
    pango_context: pango::Context,
    cell_metrics: CellMetrics,
    font_desc: pango::FontDescription,
}

impl FontMetrix {
    pub fn new(pango_context: pango::Context, line_space: i32) -> Self {
        let font_metrics =
            pango_context.metrics(None, Some(&pango::Language::from_string("en_US")));
        let font_desc = pango_context.font_description().unwrap();

        FontMetrix {
            pango_context,
            cell_metrics: CellMetrics::new(&font_metrics, line_space),
            font_desc,
        }
    }
}

// TODO: See if we can convert most of these to f32
pub struct CellMetrics {
    pub line_height: f64,
    pub char_width: f64,
    pub ascent: f64,
    pub descent: f64,
    pub underline_position: f64,
    pub underline_thickness: f64,
    pub strikethrough_position: f64,
    pub strikethrough_thickness: f64,
    pub pango_ascent: i32,
    pub pango_descent: i32,
    pub pango_char_width: i32,
}

impl CellMetrics {
    fn new(font_metrics: &pango::FontMetrics, line_space: i32) -> Self {
        let ascent = (f64::from(font_metrics.ascent()) / f64::from(pango::SCALE)).ceil();
        let descent = (f64::from(font_metrics.descent()) / f64::from(pango::SCALE)).ceil();

        // distance above top of underline, will typically be negative
        let pango_underline_position = f64::from(font_metrics.underline_position());
        let underline_position = (pango_underline_position / f64::from(pango::SCALE))
            .abs()
            .ceil()
            .copysign(pango_underline_position);

        let underline_thickness =
            (f64::from(font_metrics.underline_thickness()) / f64::from(pango::SCALE)).ceil();

        let strikethrough_position =
            (f64::from(font_metrics.strikethrough_position()) / f64::from(pango::SCALE)).ceil();
        let strikethrough_thickness =
            (f64::from(font_metrics.strikethrough_thickness()) / f64::from(pango::SCALE)).ceil();

        CellMetrics {
            pango_ascent: font_metrics.ascent(),
            pango_descent: font_metrics.descent(),
            pango_char_width: font_metrics.approximate_char_width(),
            ascent,
            descent,
            line_height: ascent + descent + f64::from(line_space),
            char_width: f64::from(font_metrics.approximate_char_width()) / f64::from(pango::SCALE),
            underline_position: ascent - underline_position + underline_thickness / 2.0,
            underline_thickness,
            strikethrough_position: ascent - strikethrough_position + strikethrough_thickness / 2.0,
            strikethrough_thickness,
        }
    }

    #[cfg(test)]
    pub fn new_hw(line_height: f64, char_width: f64) -> Self {
        CellMetrics {
            pango_ascent: 0,
            pango_descent: 0,
            pango_char_width: 0,
            ascent: 0.0,
            descent: 0.0,
            line_height,
            char_width,
            underline_position: 0.0,
            underline_thickness: 0.0,
            strikethrough_position: 0.0,
            strikethrough_thickness: 0.0,
        }
    }

    // Translate the given grid coordinates into their actual pixel coordinates
    pub fn get_pixel_coords(&self, (row, col): (usize, usize)) -> (f64, f64) {
        (self.char_width * col as f64, self.line_height * row as f64)
    }

    /* Translate the given pixel coordinates to their positions on the grid, while allowing for
     * fractional values to be returned (nvim asks for this sometimes!)
     */
    pub fn get_fractional_grid_area(
        &self,
        (x, y, w, h): (f64, f64, f64, f64),
    ) -> (f64, f64, f64, f64) {
        (
            x / self.char_width,
            y / self.line_height,
            w / self.char_width,
            h / self.line_height,
        )
    }

    // Convert a count of cells to its respective length in pixels
    pub fn get_cell_len(&self, len: usize) -> f64 {
        self.char_width * len as f64
    }
}

pub struct FontFeatures {
    attr: Option<pango::Attribute>,
}

impl FontFeatures {
    pub fn new() -> Self {
        FontFeatures { attr: None }
    }

    pub fn from(font_features: String) -> Self {
        if font_features.trim().is_empty() {
            return Self::new();
        }

        FontFeatures {
            attr: Some(pango::AttrFontFeatures::new(&font_features).upcast()),
        }
    }

    pub fn insert_into(&self, attr_list: &pango::AttrList) {
        if let Some(ref attr) = self.attr {
            attr_list.insert(attr.clone());
        }
    }
}
