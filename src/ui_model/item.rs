use crate::{
    render,
    color,
};

use gsk::{
    self,
    graphene,
};
use pango;

#[derive(Clone)]
pub struct Item {
    pub item: pango::Item,
    pub cells_count: usize,
    glyphs: Option<pango::GlyphString>,
    render_node: Option<gsk::TextNode>,
    pub ink_overflow: Option<InkOverflow>,
    font: pango::Font,
}

impl Item {
    pub fn new(item: pango::Item, cells_count: usize) -> Self {
        debug_assert!(cells_count > 0);

        Item {
            font: item.analysis().font(),
            item,
            cells_count,
            glyphs: None,
            render_node: None,
            ink_overflow: None, // TODO: get rid of this
        }
    }

    pub fn glyphs(&self) -> Option<&pango::GlyphString> {
        self.glyphs.as_ref()
    }

    pub fn set_glyphs(&mut self, ctx: &render::Context, glyphs: pango::GlyphString) {
        let mut glyphs = glyphs;
        let (ink_rect, _) = glyphs.extents(&self.font);
        self.ink_overflow = InkOverflow::from(ctx, &ink_rect, self.cells_count as i32);
        self.glyphs = Some(glyphs);
        self.render_node = None;
    }

    pub fn render_node(&mut self, color: &color::Color, (x, y): (f32, f32)) -> &gsk::TextNode {
        if self.render_node.is_none() {
            self.render_node = gsk::TextNode::new(
                &self.font,
                self.glyphs.as_mut().unwrap(),
                &color.into(),
                &graphene::Point::new(x, y)
            );
        }

        self.render_node.as_ref().expect("Failed to create render node")
    }

    pub fn new_render_node(&self, color: &color::Color, (x, y): (f32, f32)) -> gsk::TextNode {
        gsk::TextNode::new(
            &self.font,
            &mut self.glyphs.as_ref().unwrap().clone(),
            &color.into(),
            &graphene::Point::new(x, y)
        ).expect("Failed to create render node")
    }

    pub fn font(&self) -> &pango::Font {
        &self.font
    }

    pub fn analysis(&self) -> &pango::Analysis {
        self.item.analysis()
    }
}

// TODO: Because we don't handle calculating damage ourselves anymore (it magically "just works"
// with render nodes), we probably will want to remove the overflow logic entirely
#[derive(Clone)]
pub struct InkOverflow {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bot: f64,
}

impl InkOverflow {
    pub fn from(
        ctx: &render::Context,
        ink_rect: &pango::Rectangle,
        cells_count: i32,
    ) -> Option<Self> {
        let cell_metrix = ctx.cell_metrics();

        let ink_descent = ink_rect.y() + ink_rect.height();
        let ink_ascent = ink_rect.y().abs();

        let mut top = ink_ascent - cell_metrix.pango_ascent;
        if top < 0 {
            top = 0;
        }

        let mut bot = ink_descent - cell_metrix.pango_descent;
        if bot < 0 {
            bot = 0;
        }

        let left = if ink_rect.x() < 0 { ink_rect.x().abs() } else { 0 };

        let mut right = ink_rect.width() - cells_count * cell_metrix.pango_char_width;
        if right < 0 {
            right = 0;
        }

        if left == 0 && right == 0 && top == 0 && bot == 0 {
            None
        } else {
            Some(InkOverflow {
                left: left as f64 / pango::SCALE as f64,
                right: right as f64 / pango::SCALE as f64,
                top: top as f64 / pango::SCALE as f64,
                bot: bot as f64 / pango::SCALE as f64,
            })
        }
    }
}
