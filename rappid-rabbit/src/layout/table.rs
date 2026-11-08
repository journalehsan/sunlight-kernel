//! Bounded automatic table layout in the existing geometry arena.
use super::*;

const MAX_COLUMNS: usize = 256;

struct Cell {
    id: LayoutId,
    row: usize,
    column: usize,
    columns: usize,
    rows: usize,
}

#[derive(Default)]
struct Grid {
    rows: Vec<LayoutId>,
    cells: Vec<Cell>,
    columns: usize,
}

impl LayoutEngine<'_> {
    fn table_grid(&self, table: LayoutId) -> Grid {
        let mut grid = Grid::default();
        let mut stack: Vec<_> = self.tree.boxes[table.0 as usize]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            let item = &self.tree.boxes[id.0 as usize];
            match item.style.display {
                Display::TableRow => grid.rows.push(id),
                Display::TableRowGroup => stack.extend(item.children.iter().rev().copied()),
                _ => {}
            }
        }
        let mut occupied_until = vec![0; MAX_COLUMNS];
        for (row, &id) in grid.rows.iter().enumerate() {
            let mut column = 0;
            for &id in &self.tree.boxes[id.0 as usize].children {
                let item = &self.tree.boxes[id.0 as usize];
                if item.style.display != Display::TableCell {
                    continue;
                }
                let attrs = self
                    .context
                    .styles
                    .document()
                    .attributes(item.dom_node)
                    .unwrap_or(&[]);
                let columns = attribute_value(attrs, "colspan")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, MAX_COLUMNS);
                let rows = attribute_value(attrs, "rowspan")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1);
                let rows = if rows == 0 {
                    // HTML rowspan=0 extends to the end of its row group.
                    let group = self.tree.boxes[grid.rows[row].0 as usize].parent;
                    grid.rows[row..]
                        .iter()
                        .take_while(|r| self.tree.boxes[r.0 as usize].parent == group)
                        .count()
                } else {
                    rows.min(grid.rows.len() - row)
                };
                while column < MAX_COLUMNS {
                    let end = (column + columns).min(MAX_COLUMNS);
                    if let Some(blocked) = occupied_until[column..end]
                        .iter()
                        .rposition(|&until| until > row)
                    {
                        column += blocked + 1;
                    } else {
                        break;
                    }
                }
                if column == MAX_COLUMNS {
                    break;
                }
                let columns = columns.min(MAX_COLUMNS - column);
                for slot in &mut occupied_until[column..column + columns] {
                    *slot = row + rows;
                }
                grid.cells.push(Cell {
                    id,
                    row,
                    column,
                    columns,
                    rows,
                });
                column += columns;
                grid.columns = grid.columns.max(column);
            }
        }
        grid
    }

    fn table_spacing(&self, table: LayoutId) -> (f32, f32) {
        let item = &self.tree.boxes[table.0 as usize];
        let computed = self.context.styles.computed(item.dom_node);
        if keyword(computed, Property::BorderCollapse) == "collapse" {
            return (0.0, 0.0);
        }
        let raw = computed
            .and_then(|s| s.value(&Property::BorderSpacing))
            .map(PropertyValue::display)
            .unwrap_or_default();
        let mut values = raw.split_ascii_whitespace().map(|v| {
            let v = crate::css::parse_inline_style(&format!("width:{v}"));
            v.first()
                .and_then(|d| match d.value {
                    PropertyValue::LengthPx(px) => Some(px.max(0) as f32),
                    _ => None,
                })
                .unwrap_or(0.0)
        });
        let x = values.next().unwrap_or(0.0);
        (x, values.next().unwrap_or(x))
    }

    fn table_columns(&self, grid: &Grid, spacing: f32, width: Option<f32>) -> (Vec<f32>, Vec<f32>) {
        let mut minimum = vec![0.0_f32; grid.columns];
        let mut preferred = minimum.clone();
        // Single-column constraints precede spanning constraints.
        let mut cells: Vec<_> = grid.cells.iter().collect();
        cells.sort_by_key(|cell| cell.columns);
        for cell in cells {
            let item = &self.tree.boxes[cell.id.0 as usize];
            let padding = resolve_edges(
                &self.context,
                item.style.padding,
                &item.style,
                width.unwrap_or(0.0),
            );
            let outer = padding.horizontal() + item.style.border.horizontal();
            let specified = match item.style.width {
                Dimension::Px(px) => {
                    px + if item.style.box_sizing == BoxSizing::ContentBox {
                        outer
                    } else {
                        0.0
                    }
                }
                Dimension::Percent(pct) => width.map(|w| w * pct).unwrap_or(0.0),
                _ => 0.0,
            };
            let min = max(
                item.intrinsic.min_content_width + outer,
                if matches!(item.style.width, Dimension::Px(_)) {
                    specified
                } else {
                    0.0
                },
            );
            let max = max(
                min,
                max(item.intrinsic.max_content_width + outer, specified),
            );
            let range = cell.column..cell.column + cell.columns;
            let gaps = spacing * (cell.columns - 1) as f32;
            let missing = (min - minimum[range.clone()].iter().sum::<f32>() - gaps).max(0.0)
                / cell.columns as f32;
            for column in range.clone() {
                minimum[column] += missing;
                preferred[column] = preferred[column].max(minimum[column]);
            }
            let missing = (max - preferred[range.clone()].iter().sum::<f32>() - gaps).max(0.0)
                / cell.columns as f32;
            for column in range {
                preferred[column] += missing;
            }
        }
        (minimum, preferred)
    }

    pub(super) fn measure_table(&self, table: LayoutId) -> IntrinsicSize {
        let grid = self.table_grid(table);
        let (x, y) = self.table_spacing(table);
        let (min, max) = self.table_columns(&grid, x, None);
        // Percentage columns constrain the preferred width of an auto table.
        // For example, two 25% side cells leave half the table for the
        // intrinsic search control in the center cell.
        let mut percentages = vec![0.0_f32; grid.columns];
        for cell in &grid.cells {
            if let Dimension::Percent(value) = self.tree.boxes[cell.id.0 as usize].style.width {
                for p in &mut percentages[cell.column..cell.column + cell.columns] {
                    *p = p.max(value / cell.columns as f32);
                }
            }
        }
        let percent_sum: f32 = percentages.iter().sum();
        let mut preferred: f32 = max.iter().sum();
        if percent_sum > 0.0 && percent_sum <= 1.0 {
            for (&width, &percent) in max.iter().zip(&percentages) {
                if percent > 0.0 {
                    preferred = preferred.max(width / percent);
                }
            }
            if percent_sum < 1.0 {
                let auto: f32 = max
                    .iter()
                    .zip(&percentages)
                    .filter(|(_, p)| **p == 0.0)
                    .map(|(w, _)| *w)
                    .sum();
                preferred = preferred.max(auto / (1.0 - percent_sum));
            }
        }
        let gaps = if grid.columns > 0 {
            x * (grid.columns + 1) as f32
        } else {
            0.0
        };
        IntrinsicSize {
            min_content_width: if keyword(
                self.context
                    .styles
                    .computed(self.tree.boxes[table.0 as usize].dom_node),
                Property::TableLayout,
            ) == "fixed"
            {
                0.0
            } else {
                min.iter().sum::<f32>() + gaps
            },
            max_content_width: preferred + gaps,
            natural_height: grid
                .rows
                .iter()
                .map(|r| self.tree.boxes[r.0 as usize].intrinsic.natural_height)
                .sum::<f32>()
                + y * (grid.rows.len() + 1) as f32,
        }
    }

    pub(super) fn arrange_table_children(
        &mut self,
        table: LayoutId,
        width: f32,
        height: f32,
    ) -> f32 {
        let grid = self.table_grid(table);
        if grid.columns == 0 {
            return self
                .arrange_block_children(table, width, height, &[], true)
                .0;
        }
        let (sx, sy) = self.table_spacing(table);
        let available = (width - sx * (grid.columns + 1) as f32).max(0.0);
        let (minimum, preferred) = self.table_columns(&grid, sx, Some(available));
        let min_sum = minimum.iter().sum::<f32>();
        let pref_sum = preferred.iter().sum::<f32>();
        let ratio = if pref_sum > min_sum {
            ((available - min_sum) / (pref_sum - min_sum)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let extra = (available - pref_sum).max(0.0) / grid.columns as f32;
        let mut widths: Vec<_> = minimum
            .iter()
            .zip(preferred)
            .map(|(min, pref)| min + (pref - min) * ratio + extra)
            .collect();
        if keyword(
            self.context
                .styles
                .computed(self.tree.boxes[table.0 as usize].dom_node),
            Property::TableLayout,
        ) == "fixed"
        {
            let mut specified_columns = vec![None; grid.columns];
            for cell in grid.cells.iter().filter(|cell| cell.row == 0) {
                let style = &self.tree.boxes[cell.id.0 as usize].style;
                let specified = match style.width {
                    Dimension::Px(px) => Some(px),
                    Dimension::Percent(pct) => Some(available * pct),
                    _ => None,
                };
                if let Some(width) = specified {
                    for value in &mut specified_columns[cell.column..cell.column + cell.columns] {
                        *value = Some(width / cell.columns as f32);
                    }
                }
            }
            let used: f32 = specified_columns.iter().flatten().sum();
            let unspecified = specified_columns
                .iter()
                .filter(|v| v.is_none())
                .count()
                .max(1);
            for (width, specified) in widths.iter_mut().zip(specified_columns) {
                *width = specified.unwrap_or((available - used).max(0.0) / unspecified as f32);
            }
        }
        let origin = self.tree.boxes[table.0 as usize].content_box;
        let mut xs = vec![origin.x + sx];
        for width in &widths {
            xs.push(xs.last().unwrap() + width + sx);
        }
        let mut heights = vec![0.0_f32; grid.rows.len()];
        for cell in &grid.cells {
            let index = cell.id.0 as usize;
            let style = self.tree.boxes[index].style.clone();
            let width = widths[cell.column..cell.column + cell.columns]
                .iter()
                .sum::<f32>()
                + sx * (cell.columns - 1) as f32;
            self.tree.boxes[index].style.width = Dimension::Px(width);
            self.tree.boxes[index].style.box_sizing = BoxSizing::BorderBox;
            self.tree.boxes[index].style.margin = Edges::default();
            self.arrange(cell.id, width, height);
            self.tree.boxes[index].style = style;
            if cell.rows == 1 {
                heights[cell.row] = heights[cell.row].max(self.tree.boxes[index].border_box.height);
            }
        }
        for cell in &grid.cells {
            let range = cell.row..cell.row + cell.rows;
            let used = heights[range.clone()].iter().sum::<f32>() + sy * (cell.rows - 1) as f32;
            let extra = (self.tree.boxes[cell.id.0 as usize].border_box.height - used).max(0.0)
                / cell.rows as f32;
            for row in range {
                heights[row] += extra;
            }
        }
        let mut ys = vec![origin.y + sy];
        for h in &heights {
            ys.push(ys.last().unwrap() + h + sy);
        }
        // Row and group boxes remain in the arena for backgrounds and inspection.
        for (row, &id) in grid.rows.iter().enumerate() {
            self.set_table_box(
                id,
                Rect::new(origin.x + sx, ys[row], width - 2.0 * sx, heights[row]),
            );
        }
        let groups = self.tree.boxes[table.0 as usize].children.clone();
        for group in groups {
            if self.tree.boxes[group.0 as usize].style.display == Display::TableRowGroup {
                let rows: Vec<_> = grid
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| self.tree.boxes[r.0 as usize].parent == Some(group))
                    .map(|(i, _)| i)
                    .collect();
                if let (Some(&first), Some(&last)) = (rows.first(), rows.last()) {
                    self.set_table_box(
                        group,
                        Rect::new(
                            origin.x + sx,
                            ys[first],
                            width - 2.0 * sx,
                            ys[last] + heights[last] - ys[first],
                        ),
                    );
                }
            }
        }
        for cell in &grid.cells {
            let index = cell.id.0 as usize;
            let bounds = self.tree.boxes[index].border_box;
            translate_subtree(
                &mut self.tree.boxes,
                cell.id,
                xs[cell.column] - bounds.x,
                ys[cell.row] - bounds.y,
            );
            let used = heights[cell.row..cell.row + cell.rows].iter().sum::<f32>()
                + sy * (cell.rows - 1) as f32;
            let spare = (used - bounds.height).max(0.0);
            let computed = self
                .context
                .styles
                .computed(self.tree.boxes[index].dom_node);
            let offset = match keyword(computed, Property::VerticalAlign).as_str() {
                "bottom" => spare,
                "middle" => spare * 0.5,
                _ => 0.0,
            };
            for child in self.tree.boxes[index].children.clone() {
                translate_subtree(&mut self.tree.boxes, child, 0.0, offset);
            }
            let padding = self.tree.boxes[index].used_padding;
            let border = self.tree.boxes[index].style.border;
            self.set_box_heights(
                index,
                (used - padding.vertical() - border.vertical()).max(0.0),
                padding,
                border,
            );
        }
        ys.last().copied().unwrap_or(origin.y) - origin.y
    }

    fn set_table_box(&mut self, id: LayoutId, rect: Rect) {
        let item = &mut self.tree.boxes[id.0 as usize];
        item.content_box = rect;
        item.padding_box = rect;
        item.border_box = rect;
        item.margin_box = rect;
    }
}
