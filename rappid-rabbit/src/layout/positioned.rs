//! Positioned boxes are arranged after normal flow, in DOM preorder.
use super::*;

impl LayoutEngine<'_> {
    pub(super) fn out_of_flow(&self, id: LayoutId) -> bool {
        matches!(
            keyword(
                self.context
                    .styles
                    .computed(self.tree.boxes[id.0 as usize].dom_node),
                Property::Position
            )
            .as_str(),
            "absolute" | "fixed"
        ) && !matches!(
            self.context
                .styles
                .document()
                .get(self.tree.boxes[id.0 as usize].dom_node),
            Some(Node::Text { .. })
        )
    }

    pub(super) fn arrange_positioned(&mut self) {
        for index in 0..self.tree.boxes.len() {
            let id = LayoutId(index as u32);
            if !self.out_of_flow(id) {
                continue;
            }
            let node = self.tree.boxes[index].dom_node;
            let computed = self.context.styles.computed(node);
            let mut containing = Rect::new(
                0.0,
                0.0,
                self.context.viewport_width,
                self.context.viewport_height,
            );
            if keyword(computed, Property::Position) != "fixed" {
                let mut parent = self.tree.boxes[index].parent;
                while let Some(id) = parent {
                    let item = &self.tree.boxes[id.0 as usize];
                    if keyword(
                        self.context.styles.computed(item.dom_node),
                        Property::Position,
                    ) != "static"
                    {
                        containing = item.padding_box;
                        break;
                    }
                    parent = item.parent;
                }
            }
            let offset = |property, size| {
                css_length(
                    computed,
                    property,
                    self.context.viewport_width,
                    self.context.viewport_height,
                    size,
                )
            };
            let left = offset(Property::Left, containing.width);
            let right = offset(Property::Right, containing.width);
            let top = offset(Property::Top, containing.height);
            let bottom = offset(Property::Bottom, containing.height);
            let style = self.tree.boxes[index].style.clone();
            let padding = resolve_edges(&self.context, style.padding, &style, containing.width);
            let outer = padding.horizontal() + style.border.horizontal();
            if style.width == Dimension::Auto {
                let width = if let (Some(left), Some(right)) = (left, right) {
                    (containing.width - left - right - outer).max(0.0)
                } else {
                    let intrinsic = self.tree.boxes[index].intrinsic;
                    intrinsic
                        .min_content_width
                        .max(intrinsic.max_content_width.min(containing.width - outer))
                };
                self.tree.boxes[index].style.width = Dimension::Px(
                    width
                        + if style.box_sizing == BoxSizing::BorderBox {
                            outer
                        } else {
                            0.0
                        },
                );
            }
            self.arrange(id, containing.width, containing.height);
            self.tree.boxes[index].style = style;
            let bounds = self.tree.boxes[index].border_box;
            let x = left.unwrap_or_else(|| {
                right
                    .map(|r| containing.width - r - bounds.width)
                    .unwrap_or(0.0)
            });
            let y = top.unwrap_or_else(|| {
                bottom
                    .map(|b| containing.height - b - bounds.height)
                    .unwrap_or(0.0)
            });
            translate_subtree(
                &mut self.tree.boxes,
                id,
                containing.x + x - bounds.x,
                containing.y + y - bounds.y,
            );
        }
    }
}
