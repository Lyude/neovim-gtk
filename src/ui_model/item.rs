use std::cell::*;

use crate::color;

use gsk::graphene;

#[derive(Clone)]
pub struct Item {
    pub item: pango::Item,
    pub cells_count: usize,
    glyphs: RefCell<Option<pango::GlyphString>>,
    /**
     * The cached render node for this cell. Note we also need to cache when an item fails to
     * generate a valid render node, as pointed out by this testcase:
     * https://github.com/Lyude/neovim-gtk/issues/8#issuecomment-1353840913
     * Hence the double Option<…>
     */
    render_node: RefCell<Option<Option<gsk::TextNode>>>,
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
        }
    }

    pub fn glyphs(&self) -> Ref<'_, Option<pango::GlyphString>> {
        self.glyphs.borrow()
    }

    pub fn set_glyphs(&mut self, glyphs: pango::GlyphString) {
        *self.glyphs.borrow_mut() = Some(glyphs);
        *self.render_node.borrow_mut() = None;
    }

    pub fn render_node(&self, color: &color::Color, (x, y): (f32, f32)) -> Option<gsk::TextNode> {
        let mut render_node = self.render_node.borrow_mut();
        if let Some(ref render_node) = *render_node {
            render_node.clone()
        } else {
            let new_render_node = gsk::TextNode::new(
                &self.font,
                self.glyphs.borrow_mut().as_mut().unwrap(),
                &color.into(),
                &graphene::Point::new(x, y),
            );
            *render_node = Some(new_render_node.clone());
            new_render_node
        }
    }

    pub fn new_render_node(
        &self,
        color: &color::Color,
        (x, y): (f32, f32),
    ) -> Option<gsk::TextNode> {
        gsk::TextNode::new(
            &self.font,
            &self.glyphs().as_ref().unwrap().clone(),
            &color.into(),
            &graphene::Point::new(x, y),
        )
    }

    pub fn font(&self) -> &pango::Font {
        &self.font
    }

    pub fn analysis(&self) -> &pango::Analysis {
        self.item.analysis()
    }
}
