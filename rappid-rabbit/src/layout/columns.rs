//! Balanced columns for consecutive block children, without splitting a block.
use super::*;

impl LayoutEngine<'_> {
    pub(super) fn column_count(&self, id: LayoutId) -> usize {
        keyword(
            self.context
                .styles
                .computed(self.tree.boxes[id.0 as usize].dom_node),
            Property::ColumnCount,
        )
        .parse::<usize>()
        .unwrap_or(1)
        .clamp(1, 16)
    }

    pub(super) fn arrange_columns(&mut self, id: LayoutId, width: f32, height: f32) -> f32 {
        let item = &self.tree.boxes[id.0 as usize];
        // Inline fragmentation and nested column balancing remain normal flow.
        if item.children.iter().any(|child| {
            let b = &self.tree.boxes[child.0 as usize];
            !b.is_block_container()
                && b.style.display != Display::InlineBlock
                && !matches!(b.kind, BoxType::TextRun(t) if t.trim().is_empty())
        }) {
            return self.arrange_block_children(id, width, height, &[], true).0;
        }
        let children: Vec<_> = item
            .children
            .iter()
            .copied()
            .filter(|&c| {
                !self.out_of_flow(c)
                    && (self.tree.boxes[c.0 as usize].is_block_container()
                        || self.tree.boxes[c.0 as usize].style.display == Display::InlineBlock)
            })
            .collect();
        if children.is_empty() {
            return 0.0;
        }
        let count = self.column_count(id).min(children.len());
        let gap = if item.style.column_gap > 0.0 {
            item.style.column_gap
        } else {
            item.style.font_size
        };
        let column_width = ((width - gap * (count - 1) as f32) / count as f32).max(0.0);
        let origin = item.content_box;
        let heights: Vec<_> = children
            .iter()
            .map(|&c| self.arrange(c, column_width, height))
            .collect();
        let target = heights.iter().sum::<f32>() / count as f32;
        let mut column = 0;
        let mut y = 0.0_f32;
        let mut bottom = 0.0_f32;
        for (i, (&child, &h)) in children.iter().zip(&heights).enumerate() {
            if column + 1 < count
                && y > 0.0
                && (y + h - target).abs() > (y - target).abs()
                && children.len() - i >= count - column - 1
            {
                column += 1;
                y = 0.0;
            }
            let bounds = self.tree.boxes[child.0 as usize].margin_box;
            translate_subtree(
                &mut self.tree.boxes,
                child,
                origin.x + column as f32 * (column_width + gap) - bounds.x,
                origin.y + y - bounds.y,
            );
            y += h;
            bottom = bottom.max(y);
        }
        bottom
    }
}
