use crate::geom::Point;
use crate::paint::Canvas;
use crate::theme::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonoIcon<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
}

impl<'a> MonoIcon<'a> {
    pub const fn new(width: u32, height: u32, data: &'a [u8]) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    pub const fn bytes_per_row(&self) -> u32 {
        self.width.div_ceil(8)
    }

    pub fn validate(&self) -> Result<(), MonoIconError> {
        if self.width == 0 || self.height == 0 {
            return Err(MonoIconError::ZeroDimensions);
        }
        let expected = self.bytes_per_row() as usize * self.height as usize;
        if self.data.len() != expected {
            return Err(MonoIconError::SizeMismatch {
                expected,
                actual: self.data.len(),
            });
        }
        Ok(())
    }

    #[inline]
    pub fn bit(&self, x: u32, y: u32) -> Option<bool> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let stride = self.bytes_per_row() as usize;
        let byte = self.data[y as usize * stride + x as usize / 8];
        Some(((byte >> (7 - (x % 8))) & 1) != 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonoIconError {
    ZeroDimensions,
    SizeMismatch { expected: usize, actual: usize },
}

pub fn draw_mono_icon(
    canvas: &mut Canvas<'_>,
    icon: &MonoIcon<'_>,
    position: Point,
    color: Color,
) -> Result<(), MonoIconError> {
    icon.validate()?;
    for y in 0..icon.height {
        for x in 0..icon.width {
            if icon.bit(x, y) == Some(true) {
                canvas.put_pixel(position.x + x as i32, position.y + y as i32, color);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon_3x2() -> MonoIcon<'static> {
        MonoIcon::new(3, 2, &[0b1010_0000, 0b0100_0000])
    }

    #[test]
    fn validates_dimensions() {
        assert_eq!(
            MonoIcon::new(0, 2, &[]).validate(),
            Err(MonoIconError::ZeroDimensions)
        );
        assert_eq!(
            MonoIcon::new(9, 1, &[0]).validate(),
            Err(MonoIconError::SizeMismatch {
                expected: 2,
                actual: 1
            })
        );
    }

    #[test]
    fn bit_lookup_is_row_major_msb_first() {
        let icon = icon_3x2();
        assert_eq!(icon.bit(0, 0), Some(true));
        assert_eq!(icon.bit(1, 0), Some(false));
        assert_eq!(icon.bit(2, 0), Some(true));
        assert_eq!(icon.bit(1, 1), Some(true));
        assert_eq!(icon.bit(4, 0), None);
    }

    #[test]
    fn draw_skips_off_pixels_and_tints_on_pixels() {
        let mut pixels = [0u32; 16];
        let mut canvas = Canvas::new(&mut pixels, 4, 4, 4);
        let icon = icon_3x2();
        let color = Color::rgb(0xFF, 0xA5, 0x00);

        draw_mono_icon(&mut canvas, &icon, Point::new(0, 1), color).unwrap();

        assert_eq!(canvas.pixels[4], color.0);
        assert_eq!(canvas.pixels[5], 0);
        assert_eq!(canvas.pixels[6], color.0);
        assert_eq!(canvas.pixels[9], color.0);
        assert_eq!(canvas.pixels[10], 0);
    }

    #[test]
    fn same_icon_draws_in_multiple_colors() {
        let icon = icon_3x2();

        let mut pixels_a = [0u32; 16];
        let mut canvas_a = Canvas::new(&mut pixels_a, 4, 4, 4);
        draw_mono_icon(
            &mut canvas_a,
            &icon,
            Point::new(0, 0),
            Color::rgb(0x12, 0x34, 0x56),
        )
        .unwrap();

        let mut pixels_b = [0u32; 16];
        let mut canvas_b = Canvas::new(&mut pixels_b, 4, 4, 4);
        draw_mono_icon(
            &mut canvas_b,
            &icon,
            Point::new(0, 0),
            Color::rgb(0xAA, 0xBB, 0xCC),
        )
        .unwrap();

        assert_ne!(canvas_a.pixels[0], canvas_b.pixels[0]);
        assert_eq!(canvas_a.pixels[1], 0);
        assert_eq!(canvas_b.pixels[1], 0);
    }
}
