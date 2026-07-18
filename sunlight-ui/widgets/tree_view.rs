//! Generic tree view widget with reusable flattening, selection, and expansion state.

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};

use crate::font::{UiSymbol, VecText};
use crate::geom::{Point, Rect};
use crate::image::TgaImage;
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

const ROW_PAD_X: i32 = 4;
const ROW_PAD_Y: i32 = 2;
const DISCLOSURE_W: i32 = 12;
const ICON_GAP: i32 = 4;
const DEFAULT_ROW_H: u32 = 18;
const DEFAULT_INDENT_W: u32 = 14;

pub trait TreeModel {
    type Id: Copy + Eq + Ord;

    fn roots(&self) -> &[Self::Id];
    fn parent(&self, id: Self::Id) -> Option<Self::Id>;
    fn children(&self, id: Self::Id) -> &[Self::Id];
    fn item(&self, id: Self::Id) -> TreeItem;

    fn has_children(&self, id: Self::Id) -> bool {
        !self.children(id).is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeItem {
    pub label: String,
    pub icon: Option<UiSymbol>,
    pub image_icon: Option<TgaImage>,
    pub status_image_icon: Option<TgaImage>,
    pub secondary_text: Option<String>,
    pub disabled: bool,
}

impl TreeItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            image_icon: None,
            status_image_icon: None,
            secondary_text: None,
            disabled: false,
        }
    }

    pub fn with_icon(mut self, icon: UiSymbol) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_image_icon(mut self, icon: TgaImage) -> Self {
        self.image_icon = Some(icon);
        self
    }

    pub fn with_status_image_icon(mut self, icon: TgaImage) -> Self {
        self.status_image_icon = Some(icon);
        self
    }

    pub fn with_secondary_text(mut self, text: impl Into<String>) -> Self {
        self.secondary_text = Some(text.into());
        self
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeViewRow<Id> {
    pub id: Id,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
    pub selected: bool,
    pub disabled: bool,
    pub label: String,
    pub icon: Option<UiSymbol>,
    pub image_icon: Option<TgaImage>,
    pub status_image_icon: Option<TgaImage>,
    pub secondary_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeViewAction<Id> {
    SelectionChanged(Option<Id>),
    ExpansionChanged { id: Id, expanded: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeHitTarget {
    Row,
    Disclosure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeViewHit<Id> {
    pub row_index: usize,
    pub id: Id,
    pub target: TreeHitTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeViewState<Id> {
    expanded_ids: BTreeSet<Id>,
    selected_id: Option<Id>,
    scroll_offset: usize,
}

impl<Id> Default for TreeViewState<Id>
where
    Id: Copy + Eq + Ord,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<Id> TreeViewState<Id>
where
    Id: Copy + Eq + Ord,
{
    pub fn new() -> Self {
        Self {
            expanded_ids: BTreeSet::new(),
            selected_id: None,
            scroll_offset: 0,
        }
    }

    pub fn clear(&mut self) {
        self.expanded_ids.clear();
        self.selected_id = None;
        self.scroll_offset = 0;
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn set_scroll_offset(&mut self, scroll_offset: usize) {
        self.scroll_offset = scroll_offset;
    }

    pub fn selected_id(&self) -> Option<Id> {
        self.selected_id
    }

    pub fn is_expanded(&self, id: Id) -> bool {
        self.expanded_ids.contains(&id)
    }

    pub fn set_selected(&mut self, selected_id: Option<Id>) -> Option<TreeViewAction<Id>> {
        if self.selected_id == selected_id {
            return None;
        }
        self.selected_id = selected_id;
        Some(TreeViewAction::SelectionChanged(selected_id))
    }

    pub fn expand(&mut self, id: Id) -> Option<TreeViewAction<Id>> {
        if self.expanded_ids.insert(id) {
            Some(TreeViewAction::ExpansionChanged { id, expanded: true })
        } else {
            None
        }
    }

    pub fn collapse(&mut self, id: Id) -> Option<TreeViewAction<Id>> {
        if self.expanded_ids.remove(&id) {
            Some(TreeViewAction::ExpansionChanged {
                id,
                expanded: false,
            })
        } else {
            None
        }
    }

    pub fn toggle<M>(&mut self, model: &M, id: Id) -> Option<TreeViewAction<Id>>
    where
        M: TreeModel<Id = Id>,
    {
        if !model.has_children(id) {
            return None;
        }
        if self.is_expanded(id) {
            self.collapse(id)
        } else {
            self.expand(id)
        }
    }

    pub fn handle_hit<M>(&mut self, model: &M, hit: TreeViewHit<Id>) -> Option<TreeViewAction<Id>>
    where
        M: TreeModel<Id = Id>,
    {
        match hit.target {
            TreeHitTarget::Disclosure => self.toggle(model, hit.id),
            TreeHitTarget::Row => self.set_selected(Some(hit.id)),
        }
    }

    pub fn move_selection(
        &mut self,
        rows: &[TreeViewRow<Id>],
        delta: i32,
    ) -> Option<TreeViewAction<Id>> {
        if rows.is_empty() || delta == 0 {
            return None;
        }

        let current_index = self
            .selected_id
            .and_then(|selected_id| rows.iter().position(|row| row.id == selected_id));

        let next_index = match (current_index, delta.is_negative()) {
            (Some(index), true) => index.saturating_sub(1),
            (Some(index), false) => (index + 1).min(rows.len().saturating_sub(1)),
            (None, true) => 0,
            (None, false) => 0,
        };

        self.set_selected(Some(rows[next_index].id))
    }

    pub fn collapse_or_select_parent<M>(
        &mut self,
        model: &M,
        rows: &[TreeViewRow<Id>],
    ) -> Option<TreeViewAction<Id>>
    where
        M: TreeModel<Id = Id>,
    {
        let selected_id = self.selected_id?;
        let row = rows.iter().find(|row| row.id == selected_id)?;
        if row.has_children && row.expanded {
            self.collapse(selected_id)
        } else {
            self.set_selected(model.parent(selected_id))
        }
    }

    pub fn expand_or_select_first_child<M>(&mut self, model: &M) -> Option<TreeViewAction<Id>>
    where
        M: TreeModel<Id = Id>,
    {
        let selected_id = self.selected_id?;
        let children = model.children(selected_id);
        if children.is_empty() {
            return None;
        }
        if self.is_expanded(selected_id) {
            self.set_selected(Some(children[0]))
        } else {
            self.expand(selected_id)
        }
    }

    pub fn toggle_selected<M>(&mut self, model: &M) -> Option<TreeViewAction<Id>>
    where
        M: TreeModel<Id = Id>,
    {
        let selected_id = self.selected_id?;
        self.toggle(model, selected_id)
    }

    pub fn clamp_scroll(&mut self, row_count: usize, visible_rows: usize) {
        self.scroll_offset = self
            .scroll_offset
            .min(row_count.saturating_sub(visible_rows.max(1)));
    }

    pub fn ensure_selected_visible(&mut self, rows: &[TreeViewRow<Id>], visible_rows: usize) {
        let visible_rows = visible_rows.max(1);
        self.clamp_scroll(rows.len(), visible_rows);

        let Some(selected_id) = self.selected_id else {
            return;
        };
        let Some(selected_index) = rows.iter().position(|row| row.id == selected_id) else {
            return;
        };

        if selected_index < self.scroll_offset {
            self.scroll_offset = selected_index;
            return;
        }

        let visible_end = self.scroll_offset + visible_rows;
        if selected_index >= visible_end {
            self.scroll_offset = selected_index + 1 - visible_rows;
        }
    }

    pub fn rebuild_rows<M>(&mut self, model: &M) -> Vec<TreeViewRow<Id>>
    where
        M: TreeModel<Id = Id>,
    {
        let valid_ids = collect_reachable_ids(model);
        self.expanded_ids.retain(|id| valid_ids.contains(id));
        if self
            .selected_id
            .is_some_and(|selected_id| !valid_ids.contains(&selected_id))
        {
            self.selected_id = None;
        }

        let mut rows = Vec::with_capacity(valid_ids.len());
        let mut stack = Vec::with_capacity(valid_ids.len());
        for &root_id in model.roots().iter().rev() {
            stack.push((root_id, 0usize));
        }

        while let Some((id, depth)) = stack.pop() {
            let children = model.children(id);
            let item = model.item(id);
            let expanded = !children.is_empty() && self.expanded_ids.contains(&id);

            rows.push(TreeViewRow {
                id,
                depth,
                has_children: !children.is_empty(),
                expanded,
                selected: self.selected_id == Some(id),
                disabled: item.disabled,
                label: item.label,
                icon: item.icon,
                image_icon: item.image_icon,
                status_image_icon: item.status_image_icon,
                secondary_text: item.secondary_text,
            });

            if expanded {
                for &child_id in children.iter().rev() {
                    stack.push((child_id, depth + 1));
                }
            }
        }

        rows
    }
}

pub struct TreeView<'a, Id> {
    pub rect: Rect,
    pub rows: &'a [TreeViewRow<Id>],
    pub scroll_offset: usize,
    pub focused: bool,
    pub hovered: Option<Id>,
    pub row_h: u32,
    pub indent_w: u32,
    font: Option<&'a dyn VecText>,
}

impl<'a, Id> TreeView<'a, Id>
where
    Id: Copy + Eq,
{
    pub fn new(rect: Rect, rows: &'a [TreeViewRow<Id>]) -> Self {
        Self {
            rect,
            rows,
            scroll_offset: 0,
            focused: false,
            hovered: None,
            row_h: DEFAULT_ROW_H,
            indent_w: DEFAULT_INDENT_W,
            font: None,
        }
    }

    pub fn with_scroll_offset(mut self, scroll_offset: usize) -> Self {
        self.scroll_offset = scroll_offset;
        self
    }

    pub fn with_focus(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn with_hovered(mut self, hovered: Option<Id>) -> Self {
        self.hovered = hovered;
        self
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        let min_row_h = font.line_height() + 4;
        if self.row_h < min_row_h {
            self.row_h = min_row_h;
        }
        self
    }

    pub fn visible_row_count(&self) -> usize {
        let inner_h = self.rect.h.saturating_sub((ROW_PAD_Y.max(0) as u32) * 2);
        ((inner_h / self.row_h).max(1)) as usize
    }

    pub fn max_scroll(&self) -> usize {
        self.rows.len().saturating_sub(self.visible_row_count())
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel);

        let scroll = self.scroll_offset.min(self.max_scroll());
        let visible_rows = self.visible_row_count();
        for (local_index, row) in self.rows.iter().skip(scroll).take(visible_rows).enumerate() {
            let Some((row_rect, marker_x, label_x)) = self.row_layout(local_index, row) else {
                continue;
            };

            if row.selected {
                canvas.fill_rect(row_rect, theme.accent.darken(180));
            } else if self.hovered == Some(row.id) {
                canvas.fill_rect(row_rect, theme.panel_alt);
            }

            let text_color = row_text_color(row, theme);
            if row.has_children {
                draw_disclosure(
                    canvas,
                    marker_x,
                    row_rect.y + (row_rect.h as i32 - 7) / 2,
                    row.expanded,
                    text_color,
                );
            }

            let mut content_x = label_x;
            if let Some(icon) = row.image_icon {
                let icon_size = (row_rect.h.saturating_sub(4)).min(16);
                let icon_y = row_rect.y + (row_rect.h as i32 - icon_size as i32) / 2;
                canvas.draw_tga_icon_tinted(
                    &icon,
                    Rect::new(content_x, icon_y, icon_size, icon_size),
                    text_color,
                );
                content_x += icon_size as i32 + ICON_GAP;
            } else if let Some(icon) = row.icon {
                let glyph_y = row_rect.y + (row_rect.h as i32 - 9) / 2;
                canvas.draw_ui_symbol(content_x, glyph_y, icon, text_color);
                content_x += Canvas::measure_ui_symbol(icon) as i32 + ICON_GAP;
            }

            let secondary_width = row
                .secondary_text
                .as_ref()
                .map_or(0, |text| measured_width(self.font, text) as i32 + 8);
            let status_width = if row.status_image_icon.is_some() {
                16
            } else {
                0
            };
            let label_max_width =
                (self.rect.right() - ROW_PAD_X - content_x - secondary_width - status_width).max(0)
                    as u32;
            let clipped_label = clip_text_to_width(self.font, &row.label, label_max_width);
            draw_text_vcenter(
                canvas,
                self.font,
                clipped_label.as_str(),
                content_x,
                row_rect.y,
                row_rect.h,
                text_color,
            );

            if let Some(secondary) = row.secondary_text.as_ref() {
                let secondary_max_width = (self.rect.right() - ROW_PAD_X - content_x).max(0) as u32;
                let clipped_secondary =
                    clip_text_to_width(self.font, secondary, secondary_max_width.min(120));
                let secondary_w = measured_width(self.font, clipped_secondary.as_str()) as i32;
                let secondary_x =
                    (self.rect.right() - ROW_PAD_X - status_width - secondary_w).max(content_x);
                draw_text_vcenter(
                    canvas,
                    self.font,
                    clipped_secondary.as_str(),
                    secondary_x,
                    row_rect.y,
                    row_rect.h,
                    if row.selected {
                        theme.accent.lighten(24)
                    } else {
                        theme.text_dim
                    },
                );
            }
            if let Some(status_icon) = row.status_image_icon {
                let icon_x = self.rect.right() - ROW_PAD_X - 14;
                let icon_y = row_rect.y + (row_rect.h as i32 - 12) / 2;
                canvas.draw_tga_icon_tinted(
                    &status_icon,
                    Rect::new(icon_x, icon_y, 12, 12),
                    if row.selected {
                        theme.accent
                    } else {
                        theme.warn
                    },
                );
            }
        }

        if self.max_scroll() > 0 {
            if scroll > 0 {
                canvas.fill_rect(
                    Rect::new(self.rect.right() - 8, self.rect.y + 6, 3, 5),
                    theme.text_dim,
                );
            }
            if scroll + visible_rows < self.rows.len() {
                canvas.fill_rect(
                    Rect::new(self.rect.right() - 8, self.rect.bottom() - 11, 3, 5),
                    theme.text_dim,
                );
            }
        }

        canvas.draw_rect(
            self.rect,
            if self.focused {
                theme.accent
            } else {
                theme.border
            },
        );
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Option<TreeViewHit<Id>> {
        if !self.rect.contains(Point::new(x, y)) {
            return None;
        }

        let local_y = y - self.rect.y - ROW_PAD_Y;
        if local_y < 0 {
            return None;
        }

        let local_row = (local_y as u32 / self.row_h) as usize;
        let row_index = self.scroll_offset.min(self.max_scroll()) + local_row;
        let row = self.rows.get(row_index)?;
        let marker_x = self.rect.x + ROW_PAD_X + (row.depth as i32 * self.indent_w as i32);
        let row_y = self.rect.y + ROW_PAD_Y + (local_row as u32 * self.row_h) as i32;
        let marker_rect = Rect::new(marker_x, row_y, DISCLOSURE_W as u32, self.row_h);

        let target = if row.has_children && marker_rect.contains(Point::new(x, y)) {
            TreeHitTarget::Disclosure
        } else {
            TreeHitTarget::Row
        };

        Some(TreeViewHit {
            row_index,
            id: row.id,
            target,
        })
    }

    fn row_layout(&self, local_index: usize, row: &TreeViewRow<Id>) -> Option<(Rect, i32, i32)> {
        let row_y = self.rect.y + ROW_PAD_Y + (local_index as u32 * self.row_h) as i32;
        if row_y >= self.rect.bottom() {
            return None;
        }
        let row_rect = Rect::new(self.rect.x, row_y, self.rect.w, self.row_h);
        let marker_x = self.rect.x + ROW_PAD_X + (row.depth as i32 * self.indent_w as i32);
        let label_x = marker_x + DISCLOSURE_W;
        Some((row_rect, marker_x, label_x))
    }
}

fn collect_reachable_ids<M>(model: &M) -> BTreeSet<M::Id>
where
    M: TreeModel,
{
    let mut valid_ids = BTreeSet::new();
    let mut stack = Vec::new();
    for &root_id in model.roots().iter().rev() {
        stack.push(root_id);
    }

    while let Some(id) = stack.pop() {
        if !valid_ids.insert(id) {
            continue;
        }
        for &child_id in model.children(id).iter().rev() {
            stack.push(child_id);
        }
    }

    valid_ids
}

fn row_text_color<Id>(row: &TreeViewRow<Id>, theme: &Theme) -> Color {
    if row.disabled {
        theme.text_dim
    } else if row.selected {
        theme.accent
    } else {
        theme.text
    }
}

fn draw_text_vcenter(
    canvas: &mut Canvas,
    font: Option<&dyn VecText>,
    text: &str,
    x: i32,
    y: i32,
    height: u32,
    color: Color,
) {
    if let Some(font) = font {
        font.draw_vcenter(canvas, text, x, y, height, color);
    } else {
        let ty = y + (height as i32 - 7) / 2;
        canvas.draw_text(x, ty, text, color);
    }
}

fn measured_width(font: Option<&dyn VecText>, text: &str) -> u32 {
    font.map_or_else(|| Canvas::measure_text(text), |font| font.measure_w(text))
}

fn clip_text_to_width(font: Option<&dyn VecText>, value: &str, max_width: u32) -> String {
    if max_width == 0 {
        return String::new();
    }
    if measured_width(font, value) <= max_width {
        return value.to_string();
    }

    let ellipsis = "...";
    let ellipsis_w = measured_width(font, ellipsis);
    if ellipsis_w >= max_width {
        return ellipsis.to_string();
    }

    let mut out = String::new();
    for ch in value.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        candidate.push_str(ellipsis);
        if measured_width(font, candidate.as_str()) > max_width {
            break;
        }
        out.push(ch);
    }
    out.push_str(ellipsis);
    out
}

fn draw_disclosure(canvas: &mut Canvas, x: i32, y: i32, expanded: bool, color: Color) {
    if expanded {
        for (row_index, &(start, width)) in DOWN_TRIANGLE_ROWS.iter().enumerate() {
            canvas.hline(x + start, y + row_index as i32, width, color);
        }
    } else {
        for (row_index, &(start, width)) in RIGHT_TRIANGLE_ROWS.iter().enumerate() {
            canvas.hline(x + start, y + row_index as i32, width, color);
        }
    }
}

const RIGHT_TRIANGLE_ROWS: [(i32, u32); 7] =
    [(0, 1), (0, 2), (0, 3), (0, 4), (0, 3), (0, 2), (0, 1)];

const DOWN_TRIANGLE_ROWS: [(i32, u32); 7] =
    [(0, 7), (1, 5), (1, 5), (2, 3), (2, 3), (3, 1), (3, 1)];

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestNode {
        parent: Option<usize>,
        children: Vec<usize>,
        label: &'static str,
    }

    struct TestModel {
        roots: Vec<usize>,
        nodes: Vec<Option<TestNode>>,
    }

    impl TestModel {
        fn basic() -> Self {
            Self {
                roots: vec![0],
                nodes: vec![
                    Some(TestNode {
                        parent: None,
                        children: vec![1, 2],
                        label: "root",
                    }),
                    Some(TestNode {
                        parent: Some(0),
                        children: vec![3, 4],
                        label: "alpha",
                    }),
                    Some(TestNode {
                        parent: Some(0),
                        children: vec![],
                        label: "beta",
                    }),
                    Some(TestNode {
                        parent: Some(1),
                        children: vec![],
                        label: "alpha-1",
                    }),
                    Some(TestNode {
                        parent: Some(1),
                        children: vec![],
                        label: "alpha-2",
                    }),
                ],
            }
        }

        fn deep(depth: usize) -> Self {
            let mut nodes = Vec::with_capacity(depth);
            for index in 0..depth {
                nodes.push(Some(TestNode {
                    parent: index.checked_sub(1),
                    children: if index + 1 < depth {
                        vec![index + 1]
                    } else {
                        Vec::new()
                    },
                    label: "node",
                }));
            }
            Self {
                roots: vec![0],
                nodes,
            }
        }
    }

    impl TreeModel for TestModel {
        type Id = usize;

        fn roots(&self) -> &[Self::Id] {
            &self.roots
        }

        fn parent(&self, id: Self::Id) -> Option<Self::Id> {
            self.nodes
                .get(id)
                .and_then(Option::as_ref)
                .and_then(|node| node.parent)
        }

        fn children(&self, id: Self::Id) -> &[Self::Id] {
            self.nodes
                .get(id)
                .and_then(Option::as_ref)
                .map_or(&[], |node| node.children.as_slice())
        }

        fn item(&self, id: Self::Id) -> TreeItem {
            let label = self
                .nodes
                .get(id)
                .and_then(Option::as_ref)
                .map_or("(missing)", |node| node.label);
            TreeItem::new(label)
        }
    }

    #[test]
    fn root_row_generation_is_stable() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        let rows = state.rebuild_rows(&model);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "root");
        assert_eq!(rows[0].depth, 0);
    }

    #[test]
    fn initially_expanded_root_shows_first_level_children() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(0);
        let rows = state.rebuild_rows(&model);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["root", "alpha", "beta"]);
    }

    #[test]
    fn expand_branch_adds_descendants_in_order() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(0);
        state.expand(1);
        let rows = state.rebuild_rows(&model);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["root", "alpha", "alpha-1", "alpha-2", "beta"]);
    }

    #[test]
    fn collapse_branch_hides_descendants_only() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(0);
        state.expand(1);
        state.collapse(1);
        let rows = state.rebuild_rows(&model);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["root", "alpha", "beta"]);
    }

    #[test]
    fn indentation_depth_matches_visible_hierarchy() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(0);
        state.expand(1);
        let rows = state.rebuild_rows(&model);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].depth, 2);
        assert_eq!(rows[3].depth, 2);
        assert_eq!(rows[4].depth, 1);
    }

    #[test]
    fn selection_changes_and_marks_visible_row() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(0);
        state.expand(1);
        let _rows = state.rebuild_rows(&model);
        state.set_selected(Some(3));
        let rows = state.rebuild_rows(&model);
        assert_eq!(state.selected_id(), Some(3));
        assert!(rows.iter().find(|row| row.id == 3).unwrap().selected);
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn selection_does_not_change_expansion_state() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(0);
        state.expand(1);
        state.set_selected(Some(4));
        let _ = state.rebuild_rows(&model);
        assert!(state.is_expanded(0));
        assert!(state.is_expanded(1));
    }

    #[test]
    fn invalid_selected_id_is_cleaned_on_rebuild() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.set_selected(Some(99));
        let _ = state.rebuild_rows(&model);
        assert_eq!(state.selected_id(), None);
    }

    #[test]
    fn invalid_expanded_id_is_cleaned_on_rebuild() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(99);
        let _ = state.rebuild_rows(&model);
        assert!(!state.is_expanded(99));
    }

    #[test]
    fn deep_tree_rebuild_avoids_recursive_walks() {
        let depth = 4_096;
        let model = TestModel::deep(depth);
        let mut state = TreeViewState::new();
        for index in 0..depth {
            state.expand(index);
        }
        let rows = state.rebuild_rows(&model);
        assert_eq!(rows.len(), depth);
        assert_eq!(rows.last().unwrap().depth, depth - 1);
    }

    #[test]
    fn repeated_rebuilds_are_stable() {
        let model = TestModel::basic();
        let mut state = TreeViewState::new();
        state.expand(0);
        state.expand(1);
        state.set_selected(Some(4));

        let first = state.rebuild_rows(&model);
        let second = state.rebuild_rows(&model);

        assert_eq!(first, second);
    }
}
