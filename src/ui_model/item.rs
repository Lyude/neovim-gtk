use std::cell::*;

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
    glyphs: RefCell<Option<pango::GlyphString>>,
    render_node: RefCell<Option<gsk::TextNode>>,
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
            glyphs: RefCell::new(None),
            render_node: RefCell::new(None),
            ink_overflow: None,
        }
    }

    pub fn glyphs(&self) -> Ref<Option<pango::GlyphString>> {
        self.glyphs.borrow()
    }

    pub fn set_glyphs(&mut self, ctx: &render::Context, glyphs: pango::GlyphString) {
        let mut glyphs = glyphs;
        let (ink_rect, _) = glyphs.extents(&self.font);
        self.ink_overflow = InkOverflow::from(ctx, &ink_rect, self.cells_count as i32);
        *self.glyphs.borrow_mut() = Some(glyphs);
        *self.render_node.borrow_mut() = None;
    }

    pub fn render_node(&self, color: &color::Color, (x, y): (f32, f32)) -> gsk::TextNode {
        let mut render_node = self.render_node.borrow_mut();
        if render_node.is_none() {
            *render_node = gsk::TextNode::new(
                &self.font,
                self.glyphs.borrow_mut().as_mut().unwrap(),
                &color.into(),
                &graphene::Point::new(x, y)
            );
        }

        render_node.as_ref().expect("Failed to create render node").clone()
    }

    pub fn new_render_node(&self, color: &color::Color, (x, y): (f32, f32)) -> gsk::TextNode {
        gsk::TextNode::new(
            &self.font,
            &mut self.glyphs().as_ref().unwrap().clone(),
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
