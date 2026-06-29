pub mod ui_symbols;

pub use ui_symbols::{UiGlyph, UiSymbol};

use crate::paint::Canvas;
use crate::theme::Color;

/// Anti-aliased vector text renderer.
///
/// Widgets in `sunlight-ui` accept an optional `&dyn VecText` to render labels
/// at GUI quality. When `None`, the built-in 5×7 bitmap font is used as
/// fallback (early-boot screens, TTY, panic handlers).
///
/// `sun-font` provides the concrete implementation (`VecFont`). App code passes
/// a reference to avoid a circular dependency between `sunlight-ui` and `sun-font`.
pub trait VecText {
    /// Draw `text` at `(x, y)` — `y` is the top of the em box. Returns new x.
    fn draw(&self, canvas: &mut Canvas, text: &str, x: i32, y: i32, color: Color) -> i32;
    /// Draw `text` vertically centred within a `height`-px band starting at `y`.
    fn draw_vcenter(
        &self,
        canvas: &mut Canvas,
        text: &str,
        x: i32,
        y: i32,
        height: u32,
        color: Color,
    ) -> i32;
    /// Pixel width of `text` rendered in this style.
    fn measure_w(&self, text: &str) -> u32;
    /// Em-box height (line height) in pixels.
    fn line_height(&self) -> u32;
}
