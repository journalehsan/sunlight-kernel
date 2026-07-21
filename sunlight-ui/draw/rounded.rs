use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Color;

fn clamp_radius(rect: Rect, radius: u32) -> i32 {
    radius.min(rect.w / 2).min(rect.h / 2) as i32
}

fn contains_rounded_pixel(rect: Rect, radius: i32, x: i32, y: i32) -> bool {
    if rect.w == 0 || rect.h == 0 {
        return false;
    }
    if x < rect.x || y < rect.y || x >= rect.right() || y >= rect.bottom() {
        return false;
    }
    if radius <= 0 {
        return true;
    }

    let local_x = x - rect.x;
    let local_y = y - rect.y;
    let w = rect.w as i32;
    let h = rect.h as i32;

    let dx = if local_x < radius {
        radius - 1 - local_x
    } else if local_x >= w - radius {
        local_x - (w - radius)
    } else {
        0
    };
    let dy = if local_y < radius {
        radius - 1 - local_y
    } else if local_y >= h - radius {
        local_y - (h - radius)
    } else {
        0
    };

    if dx == 0 || dy == 0 {
        return true;
    }

    dx * dx + dy * dy < radius * radius
}

/// Like `contains_rounded_pixel` but only rounds the top-left and top-right corners.
fn contains_top_rounded_pixel(rect: Rect, radius: i32, x: i32, y: i32) -> bool {
    if rect.w == 0 || rect.h == 0 {
        return false;
    }
    if x < rect.x || y < rect.y || x >= rect.right() || y >= rect.bottom() {
        return false;
    }
    if radius <= 0 {
        return true;
    }

    let local_x = x - rect.x;
    let local_y = y - rect.y;
    let w = rect.w as i32;

    // Only apply rounding in the top `radius` rows
    if local_y >= radius {
        return true;
    }

    let dx = if local_x < radius {
        radius - 1 - local_x
    } else if local_x >= w - radius {
        local_x - (w - radius)
    } else {
        return true;
    };
    let dy = radius - 1 - local_y;

    dx * dx + dy * dy < radius * radius
}

fn clipped_rect(canvas: &Canvas<'_>, rect: Rect) -> Option<(i32, i32, i32, i32)> {
    let x0 = rect.x.max(0).min(canvas.width as i32);
    let y0 = rect.y.max(0).min(canvas.height as i32);
    let x1 = rect.right().max(0).min(canvas.width as i32);
    let y1 = rect.bottom().max(0).min(canvas.height as i32);
    if x0 >= x1 || y0 >= y1 {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

impl<'fb> Canvas<'fb> {
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: u32, color: Color) {
        let Some((x0, y0, x1, y1)) = clipped_rect(self, rect) else {
            return;
        };
        let radius = clamp_radius(rect, radius);

        for y in y0..y1 {
            let row_start = y as usize * self.stride as usize;
            for x in x0..x1 {
                if contains_rounded_pixel(rect, radius, x, y) {
                    self.pixels[row_start + x as usize] = color.0;
                }
            }
        }
    }

    /// Alpha-composite a rounded rectangle over the existing pixels.
    pub fn blend_rounded_rect(&mut self, rect: Rect, radius: u32, color: Color) {
        let Some((x0, y0, x1, y1)) = clipped_rect(self, rect) else {
            return;
        };
        let radius = clamp_radius(rect, radius);

        for y in y0..y1 {
            let row_start = y as usize * self.stride as usize;
            for x in x0..x1 {
                if contains_rounded_pixel(rect, radius, x, y) {
                    let idx = row_start + x as usize;
                    self.pixels[idx] = color.blend_over(Color(self.pixels[idx])).0;
                }
            }
        }
    }

    pub fn stroke_rounded_rect(&mut self, rect: Rect, radius: u32, thickness: u32, color: Color) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }

        let Some((x0, y0, x1, y1)) = clipped_rect(self, rect) else {
            return;
        };
        let outer_radius = clamp_radius(rect, radius);
        let thickness = thickness.max(1) as i32;
        let inner = rect.inset(thickness);
        let inner_radius = clamp_radius(inner, radius.saturating_sub(thickness as u32));
        let has_inner = inner.w > 0 && inner.h > 0;

        for y in y0..y1 {
            let row_start = y as usize * self.stride as usize;
            for x in x0..x1 {
                if !contains_rounded_pixel(rect, outer_radius, x, y) {
                    continue;
                }
                if has_inner && contains_rounded_pixel(inner, inner_radius, x, y) {
                    continue;
                }
                self.pixels[row_start + x as usize] = color.0;
            }
        }
    }

    /// Fill a rect with radius applied only to the top-left and top-right corners.
    pub fn fill_top_rounded_rect(&mut self, rect: Rect, radius: u32, color: Color) {
        let Some((x0, y0, x1, y1)) = clipped_rect(self, rect) else {
            return;
        };
        let radius = clamp_radius(rect, radius);

        for y in y0..y1 {
            let row_start = y as usize * self.stride as usize;
            for x in x0..x1 {
                if contains_top_rounded_pixel(rect, radius, x, y) {
                    self.pixels[row_start + x as usize] = color.0;
                }
            }
        }
    }

    pub fn fill_rounded_rect_with_border(
        &mut self,
        rect: Rect,
        radius: u32,
        fill: Color,
        border: Color,
        thickness: u32,
    ) {
        self.fill_rounded_rect(rect, radius, fill);
        self.stroke_rounded_rect(rect, radius, thickness, border);
    }
}

#[cfg(test)]
mod tests {
    use crate::paint::Canvas;
    use crate::theme::Color;

    #[test]
    fn rounded_fill_clips_to_canvas() {
        let mut pixels = [0u32; 16];
        let mut canvas = Canvas::new(&mut pixels, 4, 4, 4);
        canvas.fill_rounded_rect(crate::Rect::new(-2, -2, 5, 5), 2, Color::rgb(1, 2, 3));
        assert_eq!(pixels[15], 0);
    }

    #[test]
    fn rounded_stroke_leaves_center_unfilled() {
        let mut pixels = [0u32; 25];
        let mut canvas = Canvas::new(&mut pixels, 5, 5, 5);
        canvas.stroke_rounded_rect(crate::Rect::new(0, 0, 5, 5), 2, 1, Color::rgb(9, 9, 9));
        assert_eq!(pixels[12], 0);
    }
}
