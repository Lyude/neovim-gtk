use unicode_segmentation::*;

#[derive(Debug, PartialEq, Eq)]
pub struct ItemizeResult {
    pub offset: usize,
    pub len: usize,
    pub avoid_break: bool,
}

impl ItemizeResult {
    pub fn new(offset: usize, len: usize, avoid_break: bool) -> Self {
        Self {
            offset,
            len,
            avoid_break,
        }
    }
}

pub struct ItemizeIterator<'a> {
    grapheme_iter: GraphemeIndices<'a>,
    line: &'a str,
    prev_grapheme: Option<(usize, &'a str)>,
}

impl<'a> ItemizeIterator<'a> {
    pub fn new(line: &'a str) -> Self {
        ItemizeIterator {
            grapheme_iter: line.grapheme_indices(true),
            line,
            prev_grapheme: None,
        }
    }
}

/**
 * Iterates through a line of text while itemizing it into the largest possible clusters of
 * non-whitespace characters that can be drawn at once without risking column misalignment from
 * ambiguous width characters. This means for ASCII where the size of non-whitespace is essentially
 * guaranteed to be consistent, items will ideally be per-word to speed up rendering. For Unicode,
 * items will be per-grapheme to ensure correct monospaced display.
 */
impl Iterator for ItemizeIterator<'_> {
    type Item = ItemizeResult;

    fn next(&mut self) -> Option<Self::Item> {
        let mut start_index = None;
        let mut avoid_break = false;

        let end_index = loop {
            let grapheme_indice = self
                .prev_grapheme
                .take()
                .or_else(|| self.grapheme_iter.next());
            if let Some((index, grapheme)) = grapheme_indice {
                // Figure out if this grapheme is whitespace and/or ASCII in one iteration
                let mut is_whitespace = true;
                let mut is_ascii = true;
                for c in grapheme.chars() {
                    if is_whitespace {
                        if c.is_whitespace() {
                            continue;
                        }
                        is_whitespace = false;
                    }
                    if !c.is_ascii() {
                        is_ascii = false;
                        break;
                    }
                }

                if start_index.is_none() && !is_whitespace {
                    start_index = Some(index);
                    if !is_ascii {
                        avoid_break = true;
                        break index + grapheme.len();
                    }
                }
                if start_index.is_some() && (is_whitespace || !is_ascii) {
                    self.prev_grapheme = grapheme_indice;
                    break index;
                }
            } else {
                break self.line.len();
            }
        };

        start_index.map(|start_index| {
            ItemizeResult::new(start_index, end_index - start_index, avoid_break)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iterator() {
        let mut iter = ItemizeIterator::new("Test  line 啊啊 ते ");

        assert_eq!(Some(ItemizeResult::new(0, 4, false)), iter.next());
        assert_eq!(Some(ItemizeResult::new(6, 4, false)), iter.next());
        assert_eq!(Some(ItemizeResult::new(11, 3, true)), iter.next());
        assert_eq!(Some(ItemizeResult::new(14, 3, true)), iter.next());
        assert_eq!(Some(ItemizeResult::new(18, 6, true)), iter.next());
        assert_eq!(None, iter.next());
    }
}
