//! Pure zoom model shared by the document viewer UI and worker.
//!
//! Zoom is expressed as a pixels-per-point `scale` (100% == 1 pixel per
//! typographic point). Fit modes are resolved dynamically from the page's
//! point dimensions and the available document viewport, and every scale is
//! sanitized so it can never overflow the renderer's dimension or pixel
//! budgets.

/// Smallest scale the viewer will request.
pub const MIN_SCALE: f32 = 0.1;
/// Largest scale the viewer will request (matches the service contract).
pub const MAX_SCALE: f32 = 4.0;
/// Slider bounds, in percent.
pub const MIN_ZOOM_PERCENT: u32 = 70;
pub const MAX_ZOOM_PERCENT: u32 = 200;
/// Keep in step with `sunlight_docserv`'s 16-megapixel backend budget.
pub const MAX_PAGE_PIXELS: u64 = 16_000_000;
/// The renderer truncates scaled dimensions to `u16`.
pub const MAX_PAGE_DIM: u32 = u16::MAX as u32;

/// A requested zoom, independent of the current viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomMode {
    /// Fit the whole page inside the viewport.
    FitPage,
    /// Fit the page width to the viewport.
    FitWidth,
    /// A fixed pixels-per-point scale.
    Fixed(f32),
}

impl ZoomMode {
    /// Resolve this mode to a concrete, sanitized scale.
    ///
    /// `page_pt` is the page size in points; `viewport` is the drawable
    /// document area in pixels.
    pub fn resolve(self, page_pt: (f32, f32), viewport: (u32, u32)) -> f32 {
        let (page_w, page_h) = page_pt;
        let (view_w, view_h) = viewport;
        let raw = match self {
            ZoomMode::Fixed(scale) => scale,
            ZoomMode::FitWidth => {
                if page_w > 0.0 {
                    view_w as f32 / page_w
                } else {
                    1.0
                }
            }
            ZoomMode::FitPage => {
                let by_width = if page_w > 0.0 {
                    view_w as f32 / page_w
                } else {
                    1.0
                };
                let by_height = if page_h > 0.0 {
                    view_h as f32 / page_h
                } else {
                    1.0
                };
                by_width.min(by_height)
            }
        };
        sanitize_scale(page_pt, raw)
    }
}

/// Clamp a candidate scale into the renderer's safe envelope.
///
/// Rejects non-finite or non-positive scales, bounds them to
/// [`MIN_SCALE`]/[`MAX_SCALE`], and additionally reduces them so the rendered
/// bitmap cannot exceed the pixel area or `u16` dimension limits.
pub fn sanitize_scale(page_pt: (f32, f32), scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 1.0;
    }
    let mut limited = scale.clamp(MIN_SCALE, MAX_SCALE);
    let (page_w, page_h) = page_pt;
    if page_w.is_finite() && page_h.is_finite() && page_w > 0.0 && page_h > 0.0 {
        let area = page_w as f64 * page_h as f64;
        if area > 0.0 {
            let by_area = (MAX_PAGE_PIXELS as f64 / area).sqrt();
            let by_width = MAX_PAGE_DIM as f64 / page_w as f64;
            let by_height = MAX_PAGE_DIM as f64 / page_h as f64;
            let ceiling = by_area.min(by_width).min(by_height);
            if ceiling.is_finite() {
                limited = limited.min(ceiling as f32);
            }
        }
    }
    limited.clamp(MIN_SCALE, MAX_SCALE)
}

/// Round a scale to a whole zoom percentage for display and sliders.
pub fn zoom_percent(scale: f32) -> u32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 0;
    }
    (scale * 100.0).round() as u32
}

/// Convert a whole percentage into a scale.
pub fn scale_from_percent(percent: u32) -> f32 {
    percent as f32 / 100.0
}

/// A named toolbar/slider zoom choice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomPreset {
    FitPage,
    FitWidth,
    ActualSize,
    Percent(u32),
}

impl ZoomPreset {
    /// Toolbar dropdown entries, in display order.
    pub const ALL: [ZoomPreset; 10] = [
        ZoomPreset::FitPage,
        ZoomPreset::FitWidth,
        ZoomPreset::ActualSize,
        ZoomPreset::Percent(70),
        ZoomPreset::Percent(85),
        ZoomPreset::Percent(100),
        ZoomPreset::Percent(125),
        ZoomPreset::Percent(150),
        ZoomPreset::Percent(175),
        ZoomPreset::Percent(200),
    ];

    /// Human-readable label for the dropdown.
    pub fn label(self) -> &'static str {
        match self {
            ZoomPreset::FitPage => "Fit Page",
            ZoomPreset::FitWidth => "Fit Width",
            ZoomPreset::ActualSize => "Actual Size",
            ZoomPreset::Percent(70) => "70%",
            ZoomPreset::Percent(85) => "85%",
            ZoomPreset::Percent(100) => "100%",
            ZoomPreset::Percent(125) => "125%",
            ZoomPreset::Percent(150) => "150%",
            ZoomPreset::Percent(175) => "175%",
            ZoomPreset::Percent(200) => "200%",
            ZoomPreset::Percent(_) => "Custom",
        }
    }

    /// The zoom mode this preset selects.
    pub fn mode(self) -> ZoomMode {
        match self {
            ZoomPreset::FitPage => ZoomMode::FitPage,
            ZoomPreset::FitWidth => ZoomMode::FitWidth,
            ZoomPreset::ActualSize => ZoomMode::Fixed(1.0),
            ZoomPreset::Percent(percent) => ZoomMode::Fixed(scale_from_percent(percent)),
        }
    }

    /// Whether this preset describes the supplied resolved mode.
    pub fn matches(self, mode: ZoomMode) -> bool {
        match (self, mode) {
            (ZoomPreset::FitPage, ZoomMode::FitPage) => true,
            (ZoomPreset::FitWidth, ZoomMode::FitWidth) => true,
            (ZoomPreset::ActualSize, ZoomMode::Fixed(scale)) => (scale - 1.0).abs() < 1e-4,
            (ZoomPreset::Percent(percent), ZoomMode::Fixed(scale)) => {
                (scale - scale_from_percent(percent)).abs() < 1e-4
            }
            _ => false,
        }
    }
}

/// The label shown on the zoom dropdown button for a resolved mode.
pub fn mode_label(mode: ZoomMode, percent: u32) -> &'static str {
    match mode {
        ZoomMode::FitPage => "Fit Page",
        ZoomMode::FitWidth => "Fit Width",
        ZoomMode::Fixed(_) => match percent {
            70 => "70%",
            85 => "85%",
            100 => "100%",
            125 => "125%",
            150 => "150%",
            175 => "175%",
            200 => "200%",
            _ => "Custom",
        },
    }
}

/// One discrete zoom step for the +/− controls, in percentage points.
pub const ZOOM_STEP_PERCENT: i32 = 25;

/// Step a percentage within the slider bounds.
pub fn step_percent(percent: u32, steps: i32) -> u32 {
    let stepped = percent as i64 + ZOOM_STEP_PERCENT as i64 * steps as i64;
    stepped.clamp(MIN_ZOOM_PERCENT as i64, MAX_ZOOM_PERCENT as i64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: (f32, f32) = (595.276, 841.890);

    #[test]
    fn fit_page_and_fit_width_use_viewport() {
        let viewport = (700, 560);
        let fit_width = ZoomMode::FitWidth.resolve(A4, viewport);
        assert!((fit_width - 700.0 / 595.276).abs() < 1e-3);
        let fit_page = ZoomMode::FitPage.resolve(A4, viewport);
        assert!((fit_page - (560.0 / 841.890)).abs() < 1e-3);
        assert!(fit_page <= fit_width);
    }

    #[test]
    fn fixed_scale_is_sanitized() {
        assert_eq!(ZoomMode::Fixed(1.0).resolve(A4, (700, 560)), 1.0);
        assert_eq!(ZoomMode::Fixed(f32::NAN).resolve(A4, (700, 560)), 1.0);
        assert_eq!(ZoomMode::Fixed(-2.0).resolve(A4, (700, 560)), 1.0);
        assert_eq!(ZoomMode::Fixed(100.0).resolve(A4, (700, 560)), MAX_SCALE);
    }

    #[test]
    fn sanitize_reduces_oversized_scale_to_pixel_budget() {
        let huge = (5000.0f32, 5000.0f32);
        let limited = sanitize_scale(huge, 4.0);
        assert!(limited < 4.0);
        assert!((huge.0 * limited) * (huge.1 * limited) <= MAX_PAGE_PIXELS as f32 + 1.0);
    }

    #[test]
    fn percent_helpers_round_trip() {
        assert_eq!(zoom_percent(1.0), 100);
        assert_eq!(zoom_percent(0.85), 85);
        assert_eq!(scale_from_percent(150), 1.5);
        assert_eq!(step_percent(100, 1), 125);
        assert_eq!(step_percent(200, 1), 200);
        assert_eq!(step_percent(70, -1), 70);
    }

    #[test]
    fn presets_match_modes() {
        assert!(ZoomPreset::ActualSize.matches(ZoomMode::Fixed(1.0)));
        assert!(ZoomPreset::Percent(100).matches(ZoomMode::Fixed(1.0)));
        assert!(ZoomPreset::FitPage.matches(ZoomMode::FitPage));
        assert!(!ZoomPreset::FitWidth.matches(ZoomMode::FitPage));
        assert!(!ZoomPreset::Percent(125).matches(ZoomMode::FitPage));
    }
}
