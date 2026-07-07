//! SunlightOS UI Theme
//!
//! All widget colors are derived from a single `Theme` instance.
//! To retheme the entire UI, replace the active theme — no widget
//! code needs to change.

/// 32-bit ARGB color (matches framebuffer pixel layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const TRANSPARENT: Self = Self(0x00000000);

    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    #[inline]
    pub fn r(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }
    #[inline]
    pub fn g(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }
    #[inline]
    pub fn b(self) -> u8 {
        (self.0 & 0xFF) as u8
    }
    #[inline]
    pub fn a(self) -> u8 {
        ((self.0 >> 24) & 0xFF) as u8
    }

    /// Blend `self` over `dst` using `self.alpha`.
    #[inline]
    pub fn blend_over(self, dst: Color) -> Color {
        let a = self.a() as u32;
        if a == 255 {
            return self;
        }
        if a == 0 {
            return dst;
        }
        let ia = 255 - a;
        let r = (self.r() as u32 * a + dst.r() as u32 * ia) / 255;
        let g = (self.g() as u32 * a + dst.g() as u32 * ia) / 255;
        let b = (self.b() as u32 * a + dst.b() as u32 * ia) / 255;
        Color::rgb(r as u8, g as u8, b as u8)
    }

    /// Lighten by mixing with white by `amount` 0..=255.
    pub fn lighten(self, amount: u8) -> Color {
        let mix = amount as u32;
        let r = (self.r() as u32 * (255 - mix) / 255 + mix).min(255);
        let g = (self.g() as u32 * (255 - mix) / 255 + mix).min(255);
        let b = (self.b() as u32 * (255 - mix) / 255 + mix).min(255);
        Color::rgb(r as u8, g as u8, b as u8)
    }

    /// Darken by mixing with black by `amount` 0..=255.
    pub fn darken(self, amount: u8) -> Color {
        let keep = 255 - amount as u32;
        let r = (self.r() as u32 * keep / 255) as u8;
        let g = (self.g() as u32 * keep / 255) as u8;
        let b = (self.b() as u32 * keep / 255) as u8;
        Color::rgb(r, g, b)
    }
}

/// Full UI theme. Every widget reads colors exclusively from here.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Main window / desktop background
    pub bg: Color,
    /// Primary panel / widget surface
    pub panel: Color,
    /// Alternate panel (alternating rows, secondary surfaces)
    pub panel_alt: Color,
    /// Primary text
    pub text: Color,
    /// Dimmed / secondary text
    pub text_dim: Color,
    /// Default monochrome icon foreground
    pub icon_foreground: Color,
    /// Muted icon state for secondary actions
    pub icon_muted: Color,
    /// Disabled icon state
    pub icon_disabled: Color,
    /// Accent (buttons, tabs, highlights) — SunlightOS orange by default
    pub accent: Color,
    /// Accent hover state
    pub accent_hover: Color,
    /// Success / OK indicator
    pub ok: Color,
    /// Warning indicator
    pub warn: Color,
    /// Danger / error indicator
    pub danger: Color,
    /// Subtle border / separator
    pub border: Color,
}

impl Theme {
    /// SunlightOS default dark theme with orange accent.
    pub const fn sunlight_dark() -> Self {
        Self {
            bg: Color::rgb(0x12, 0x12, 0x14),
            panel: Color::rgb(0x1C, 0x1C, 0x1F),
            panel_alt: Color::rgb(0x22, 0x22, 0x26),
            text: Color::rgb(0xF0, 0xF0, 0xF0),
            text_dim: Color::rgb(0x88, 0x88, 0x99),
            icon_foreground: Color::rgb(0xF0, 0xF0, 0xF0),
            icon_muted: Color::rgb(0xA0, 0xA0, 0xAF),
            icon_disabled: Color::rgb(0x5A, 0x5A, 0x66),
            accent: Color::rgb(0xFF, 0xA5, 0x00), // SunlightOS orange
            accent_hover: Color::rgb(0xFF, 0xBF, 0x40),
            ok: Color::rgb(0x4C, 0xAF, 0x50),
            warn: Color::rgb(0xFF, 0xC1, 0x07),
            danger: Color::rgb(0xF4, 0x43, 0x36),
            border: Color::rgb(0x35, 0x35, 0x40),
        }
    }

    /// Light theme variant.
    pub const fn sunlight_light() -> Self {
        Self {
            bg: Color::rgb(0xF2, 0xF2, 0xF5),
            panel: Color::rgb(0xFF, 0xFF, 0xFF),
            panel_alt: Color::rgb(0xEB, 0xEB, 0xEF),
            text: Color::rgb(0x11, 0x11, 0x11),
            text_dim: Color::rgb(0x66, 0x66, 0x77),
            icon_foreground: Color::rgb(0x11, 0x11, 0x11),
            icon_muted: Color::rgb(0x5C, 0x5C, 0x6B),
            icon_disabled: Color::rgb(0xA8, 0xA8, 0xB5),
            accent: Color::rgb(0xE6, 0x8A, 0x00),
            accent_hover: Color::rgb(0xFF, 0xA5, 0x00),
            ok: Color::rgb(0x2E, 0x7D, 0x32),
            warn: Color::rgb(0xF5, 0x7F, 0x17),
            danger: Color::rgb(0xC6, 0x28, 0x28),
            border: Color::rgb(0xCC, 0xCC, 0xD6),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::sunlight_dark()
    }
}
