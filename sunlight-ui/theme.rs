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
    pub const fn r(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }
    #[inline]
    pub const fn g(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }
    #[inline]
    pub const fn b(self) -> u8 {
        (self.0 & 0xFF) as u8
    }
    #[inline]
    pub const fn a(self) -> u8 {
        ((self.0 >> 24) & 0xFF) as u8
    }

    /// Absolute channel distance used for hue-family checks (no HSL in hot paths).
    #[inline]
    pub const fn channel_distance(self, other: Color) -> u32 {
        let dr = (self.r() as i32 - other.r() as i32).unsigned_abs();
        let dg = (self.g() as i32 - other.g() as i32).unsigned_abs();
        let db = (self.b() as i32 - other.b() as i32).unsigned_abs();
        dr + dg + db
    }

    /// True when both colors share a near-neutral warm/cool balance (same family).
    ///
    /// Compares relative R/G/B bias rather than absolute luminance so a denser
    /// titlebar can still match a lighter root charcoal.
    #[inline]
    pub const fn same_hue_family(self, other: Color) -> bool {
        // Drop alpha; compare chromatic bias of each channel vs mean.
        let mean_s = (self.r() as i32 + self.g() as i32 + self.b() as i32) / 3;
        let mean_o = (other.r() as i32 + other.g() as i32 + other.b() as i32) / 3;
        let br = (self.r() as i32 - mean_s) - (other.r() as i32 - mean_o);
        let bg = (self.g() as i32 - mean_s) - (other.g() as i32 - mean_o);
        let bb = (self.b() as i32 - mean_s) - (other.b() as i32 - mean_o);
        br.unsigned_abs() + bg.unsigned_abs() + bb.unsigned_abs() <= 12
    }

    /// Blend `self` over `dst` using `self.alpha`.
    /// Returns straight-alpha ARGB.
    #[inline]
    pub fn blend_over(self, dst: Color) -> Color {
        let src_a = self.a() as u64;
        if src_a == 255 {
            return self;
        }
        if src_a == 0 {
            return dst;
        }
        let dst_a = dst.a() as u64;
        let inv_src_a = 255 - src_a;

        // Keep the stored result in straight-alpha form. The common shortcut
        // `src * a + dst * (1-a)` is only valid for an opaque destination; on
        // a transparent canvas it stores premultiplied RGB and the compositor
        // attenuates the pixel a second time.
        let out_a_numerator = src_a * 255 + dst_a * inv_src_a;
        let out_a = (out_a_numerator + 127) / 255;
        let blend_channel = |src: u8, dst_channel: u8| -> u8 {
            let numerator = src as u64 * src_a * 255 + dst_channel as u64 * dst_a * inv_src_a;
            ((numerator + out_a_numerator / 2) / out_a_numerator) as u8
        };
        Color::rgba(
            blend_channel(self.r(), dst.r()),
            blend_channel(self.g(), dst.g()),
            blend_channel(self.b(), dst.b()),
            out_a as u8,
        )
    }

    /// Lighten by mixing with white by `amount` 0..=255.
    pub const fn lighten(self, amount: u8) -> Color {
        let mix = amount as u32;
        let inv = 255 - mix;
        // r*inv/255 + mix is at most 255 for u8 inputs; avoid non-const min().
        let r = self.r() as u32 * inv / 255 + mix;
        let g = self.g() as u32 * inv / 255 + mix;
        let b = self.b() as u32 * inv / 255 + mix;
        Color::rgb(r as u8, g as u8, b as u8)
    }

    /// Darken by mixing with black by `amount` 0..=255.
    pub const fn darken(self, amount: u8) -> Color {
        let keep = 255 - amount as u32;
        let r = (self.r() as u32 * keep / 255) as u8;
        let g = (self.g() as u32 * keep / 255) as u8;
        let b = (self.b() as u32 * keep / 255) as u8;
        Color::rgb(r, g, b)
    }

    /// Mix `self` toward `other` by `amount` 0..=255 (resolved at theme build).
    pub const fn mix(self, other: Color, amount: u8) -> Color {
        let t = amount as u32;
        let inv = 255 - t;
        let r = (self.r() as u32 * inv + other.r() as u32 * t) / 255;
        let g = (self.g() as u32 * inv + other.g() as u32 * t) / 255;
        let b = (self.b() as u32 * inv + other.b() as u32 * t) / 255;
        let a = (self.a() as u32 * inv + other.a() as u32 * t) / 255;
        Color::rgba(r as u8, g as u8, b as u8, a as u8)
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
    /// Readable secondary text for dense UI areas.
    pub text_muted: Color,
    /// Text intended to sit on accent-colored backgrounds.
    pub text_on_accent: Color,
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
    /// Readable error text on dark panels.
    pub danger_text: Color,
    /// Subtle border / separator
    pub border: Color,
    /// Window chrome roles (titlebar, selection, control backplates).
    ///
    /// Derived once when the theme is constructed — never recomputed per frame.
    pub chrome: ChromeRoles,
}

/// Semantic colors for native window chrome and shared surfaces.
///
/// Titlebar and window root share one warm-neutral charcoal family; active
/// state is density/contrast, not a separate hue. Applications should consume
/// these roles instead of copying raw ARGB literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromeRoles {
    /// Canonical window root / WindowGlass tint (matches Start menu charcoal).
    pub window_bg: Color,
    /// Active titlebar tint — same hue, slightly denser.
    pub titlebar_active: Color,
    /// Inactive titlebar tint — same hue, near root density.
    pub titlebar_inactive: Color,
    /// Subtle titlebar/body divider (active window).
    pub titlebar_divider_active: Color,
    /// Subtle titlebar/body divider (inactive window).
    pub titlebar_divider_inactive: Color,
    /// Active window title text.
    pub title_active: Color,
    /// Inactive window title text (reduced, still readable).
    pub title_inactive: Color,
    /// Active Horizon control glyphs.
    pub control_glyph_active: Color,
    /// Inactive Horizon control glyphs.
    pub control_glyph_inactive: Color,
    /// Neutral control hover backplate.
    pub control_hover: Color,
    /// Neutral control pressed backplate.
    pub control_pressed: Color,
    /// Grouped card / summary surface tint.
    pub card_bg: Color,
    /// Dense input / table surface tint.
    pub input_bg: Color,
    /// Subtle border / separator (alias of theme.border family).
    pub subtle_border: Color,
    /// Restrained list selection (focused window).
    pub selection: Color,
    /// Quieter selection when the window is inactive.
    pub selection_inactive: Color,
    /// Disabled foreground for text and glyphs.
    pub disabled_fg: Color,
}

impl ChromeRoles {
    /// Build chrome roles from base theme surfaces. Const — safe at theme init.
    pub const fn from_surfaces(
        panel: Color,
        panel_alt: Color,
        text: Color,
        icon_fg: Color,
        icon_disabled: Color,
        accent: Color,
        border: Color,
    ) -> Self {
        // Active titlebar: ~4% denser (darker) charcoal, same RGB bias as panel.
        let titlebar_active = panel.darken(14);
        // Inactive titlebar sits at root density so it does not read as a slab.
        let titlebar_inactive = panel;
        // Dividers: near-invisible hairlines (modern glass, not a slab seam).
        let titlebar_divider_active = panel.lighten(18);
        let titlebar_divider_inactive = panel.lighten(8);
        // Selection: quiet warm tint over panel — never a saturated orange slab.
        // Large surfaces (sidebar) rely on a thin accent bar for identity.
        let selection = panel.mix(accent, 32).darken(6);
        let selection_inactive = panel.mix(accent, 18).darken(4);
        Self {
            window_bg: panel,
            titlebar_active,
            titlebar_inactive,
            titlebar_divider_active,
            titlebar_divider_inactive,
            title_active: text,
            // Readable but quieter than active title (~60% perceived contrast).
            title_inactive: Color::rgb(0xA0, 0xA0, 0xA8),
            control_glyph_active: icon_fg,
            // Keep inactive glyphs visible; avoid near-disappearing grey.
            control_glyph_inactive: icon_disabled.lighten(48),
            // Warm-neutral glass highlights (no blue/purple cast).
            control_hover: Color::rgba(0x38, 0x38, 0x3C, 200),
            control_pressed: Color::rgba(0x26, 0x26, 0x2A, 230),
            card_bg: panel_alt,
            input_bg: panel_alt.lighten(6),
            subtle_border: border,
            selection,
            selection_inactive,
            disabled_fg: icon_disabled,
        }
    }
}

impl Theme {
    /// SunlightOS default dark theme with orange accent.
    pub const fn sunlight_dark() -> Self {
        let panel = Color::rgb(0x1C, 0x1C, 0x1F);
        let panel_alt = Color::rgb(0x22, 0x22, 0x26);
        let text = Color::rgb(0xF0, 0xF0, 0xF0);
        let icon_foreground = Color::rgb(0xF0, 0xF0, 0xF0);
        let icon_disabled = Color::rgb(0x5A, 0x5A, 0x66);
        let accent = Color::rgb(0xFF, 0xA5, 0x00); // SunlightOS orange
        let border = Color::rgb(0x35, 0x35, 0x40);
        Self {
            bg: Color::rgb(0x12, 0x12, 0x14),
            panel,
            panel_alt,
            text,
            text_dim: Color::rgb(0x88, 0x88, 0x99),
            text_muted: Color::rgb(0xC8, 0xC8, 0xD2),
            text_on_accent: Color::rgb(0x12, 0x12, 0x14),
            icon_foreground,
            icon_muted: Color::rgb(0xA0, 0xA0, 0xAF),
            icon_disabled,
            accent,
            accent_hover: Color::rgb(0xFF, 0xBF, 0x40),
            ok: Color::rgb(0x4C, 0xAF, 0x50),
            warn: Color::rgb(0xFF, 0xC1, 0x07),
            danger: Color::rgb(0xF4, 0x43, 0x36),
            danger_text: Color::rgb(0xFF, 0x8A, 0x80),
            border,
            chrome: ChromeRoles::from_surfaces(
                panel,
                panel_alt,
                text,
                icon_foreground,
                icon_disabled,
                accent,
                border,
            ),
        }
    }

    /// Light theme variant.
    pub const fn sunlight_light() -> Self {
        let panel = Color::rgb(0xFF, 0xFF, 0xFF);
        let panel_alt = Color::rgb(0xEB, 0xEB, 0xEF);
        let text = Color::rgb(0x11, 0x11, 0x11);
        let icon_foreground = Color::rgb(0x11, 0x11, 0x11);
        let icon_disabled = Color::rgb(0xA8, 0xA8, 0xB5);
        let accent = Color::rgb(0xE6, 0x8A, 0x00);
        let border = Color::rgb(0xCC, 0xCC, 0xD6);
        Self {
            bg: Color::rgb(0xF2, 0xF2, 0xF5),
            panel,
            panel_alt,
            text,
            text_dim: Color::rgb(0x66, 0x66, 0x77),
            text_muted: Color::rgb(0x3F, 0x3F, 0x4A),
            text_on_accent: Color::rgb(0x11, 0x11, 0x11),
            icon_foreground,
            icon_muted: Color::rgb(0x5C, 0x5C, 0x6B),
            icon_disabled,
            accent,
            accent_hover: Color::rgb(0xFF, 0xA5, 0x00),
            ok: Color::rgb(0x2E, 0x7D, 0x32),
            warn: Color::rgb(0xF5, 0x7F, 0x17),
            danger: Color::rgb(0xC6, 0x28, 0x28),
            danger_text: Color::rgb(0xB7, 0x1C, 0x1C),
            border,
            chrome: ChromeRoles::from_surfaces(
                panel,
                panel_alt,
                text,
                icon_foreground,
                icon_disabled,
                accent,
                border,
            ),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::sunlight_dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_chrome_shares_panel_hue_family() {
        let theme = Theme::sunlight_dark();
        let c = theme.chrome;
        assert!(c.window_bg.same_hue_family(c.titlebar_active));
        assert!(c.window_bg.same_hue_family(c.titlebar_inactive));
        assert_eq!(c.window_bg, theme.panel);
        assert_eq!(c.titlebar_inactive, theme.panel);
        // Active is denser (darker or equal), not a different cold slate.
        assert!(c.titlebar_active.r() <= c.window_bg.r());
        assert!(c.titlebar_active.g() <= c.window_bg.g());
        assert!(c.titlebar_active.b() <= c.window_bg.b());
        // No strong blue/purple bias vs R/G on charcoal surfaces.
        assert!(c.titlebar_active.b().saturating_sub(c.titlebar_active.r()) <= 4);
        assert!(
            c.titlebar_inactive
                .b()
                .saturating_sub(c.titlebar_inactive.r())
                <= 4
        );
    }

    #[test]
    fn selection_is_restrained_not_saturated_accent() {
        let theme = Theme::sunlight_dark();
        let sel = theme.chrome.selection;
        // Far from pure accent; stays near panel luminance (sidebar-safe).
        assert!(sel.channel_distance(theme.accent) > 140);
        assert!(sel.channel_distance(theme.panel) < 56);
        assert_ne!(sel, theme.chrome.selection_inactive);
    }

    #[test]
    fn inactive_title_remains_readable() {
        let theme = Theme::sunlight_dark();
        let t = theme.chrome.title_inactive;
        // Luma proxy: not near-black, not full white.
        let luma = (t.r() as u32 + t.g() as u32 + t.b() as u32) / 3;
        assert!(luma >= 0x70);
        assert!(luma <= 0xC0);
        assert_ne!(theme.chrome.title_active, theme.chrome.title_inactive);
    }
}
