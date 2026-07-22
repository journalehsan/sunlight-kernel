//! Reusable UI materials for Sunlight chrome and widgets.
//!
//! Materials describe *intent* (solid fill, tinted fill, lightweight glass), not
//! compositor implementation details. Consumers request a material and draw it
//! through [`Canvas::fill_material`](crate::paint::Canvas::fill_material)
//! without knowing about framebuffer composition.
//!
//! Glass is alpha + warm/dark tint + a tiny static noise tile. It is **not**
//! background blur.

use crate::theme::{Color, Theme};

/// Conceptual surface roles used to pick conservative material defaults.
///
/// These are toolkit-level roles. Compositors may map protocol window types
/// onto them, or use them only for client-drawn chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SurfaceRole {
    ApplicationWindow = 0,
    Panel = 1,
    Dock = 2,
    PopupOrMenu = 3,
    Tooltip = 4,
    SystemOverlay = 5,
}

/// Material style family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MaterialKind {
    /// Fully opaque solid fill.
    Solid = 0,
    /// Tinted fill with opacity; no noise.
    Tinted = 1,
    /// Lightweight glass: tint + opacity + optional static noise + border.
    Glass = 2,
}

/// Foreground contrast expected by a material preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadabilityRole {
    Primary,
    Muted,
}

/// Shared Sunlight window/effect geometry in device-independent pixels.
///
/// The compositor scales these values once for the active display.  Keeping
/// the inner radius and effect expansion together prevents independently tuned
/// masks from producing detached or pinched corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecorationGeometry {
    pub window_corner_radius: u32,
    pub structural_rim: u32,
    pub ambient_shadow_falloff: u32,
    pub ambient_shadow_offset_y: u32,
    pub solar_focus_falloff: u32,
}

impl DecorationGeometry {
    /// Shared Sunlight decoration metrics (device-independent pixels).
    ///
    /// Ambient shadow is intentionally wider than the original 12px strip so the
    /// falloff reads as a soft KWin-style contact gradient (dark near the window,
    /// lighter as it spreads). Structural rim stays a 1px Mac-style hairline.
    pub const SUNLIGHT: Self = Self {
        window_corner_radius: 10,
        structural_rim: 1,
        ambient_shadow_falloff: 28,
        ambient_shadow_offset_y: 6,
        solar_focus_falloff: 10,
    };

    pub const fn outer_shadow_corner_radius(self) -> u32 {
        self.window_corner_radius
            .saturating_add(self.ambient_shadow_falloff)
    }

    pub const fn outer_focus_corner_radius(self) -> u32 {
        self.window_corner_radius
            .saturating_add(self.solar_focus_falloff)
    }
}

/// Canonical material family for native Sunlight surfaces.
///
/// This is deliberately separate from [`Theme`] so adding material policy does
/// not expand every existing theme literal.  It contains no cache or allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialPalette {
    pub overlay_glass: Material,
    pub window_glass: Material,
    pub titlebar_active: Material,
    pub titlebar_inactive: Material,
    pub card_glass: Material,
    pub solid_content: Material,
    pub tinted_content: Material,
}

impl MaterialPalette {
    pub const fn new(theme: &Theme) -> Self {
        let chrome = theme.chrome;
        let radius = DecorationGeometry::SUNLIGHT.window_corner_radius;
        Self {
            // Canonical Start-menu values: preserve its effective pre-extraction
            // appearance (including the fact that its old Tinted preset disabled
            // the requested noise).
            overlay_glass: Material::glass(chrome.window_bg, 232)
                .with_noise(0)
                .with_border(chrome.subtle_border)
                .with_radius(12),
            // Window root: Start-menu charcoal glass — shared hue with titlebar.
            window_glass: Material::glass(chrome.window_bg, 232)
                .with_noise(4)
                .with_radius(radius),
            // Active titlebar: same hue family, ~4% denser (higher opacity + darker tint).
            titlebar_active: Material::glass(chrome.titlebar_active, 240)
                .with_noise(3)
                .with_radius(radius),
            // Inactive titlebar: same material family near root density.
            titlebar_inactive: Material::glass(chrome.titlebar_inactive, 233)
                .with_noise(3)
                .with_radius(radius),
            card_glass: Material::glass(chrome.card_bg, 247)
                .with_noise(2)
                .with_border(chrome.subtle_border)
                .with_radius(8),
            solid_content: Material::solid(theme.bg),
            tinted_content: Material::tinted(chrome.input_bg, 250)
                .with_border(chrome.subtle_border)
                .with_radius(6),
        }
    }

    /// Accessibility fallback: preserve hierarchy and borders while removing
    /// transparency and noise from every material.
    pub const fn opaque(self) -> Self {
        Self {
            overlay_glass: self.overlay_glass.opaque_fallback(),
            window_glass: self.window_glass.opaque_fallback(),
            titlebar_active: self.titlebar_active.opaque_fallback(),
            titlebar_inactive: self.titlebar_inactive.opaque_fallback(),
            card_glass: self.card_glass.opaque_fallback(),
            solid_content: self.solid_content.opaque_fallback(),
            tinted_content: self.tinted_content.opaque_fallback(),
        }
    }
}

/// General-purpose material description.
///
/// Opacity and noise are stored as 0..=255 and clamped by constructors and
/// [`Material::clamp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Material {
    pub kind: MaterialKind,
    /// Base color (RGB). Alpha channel is ignored; use [`opacity`].
    pub tint: Color,
    /// Overall coverage 0..=255 (255 = opaque).
    pub opacity: u8,
    /// Noise amplitude 0..=255. Meaningful for glass only; ~1–3% = 3..=8.
    pub noise_strength: u8,
    /// Optional 1-px border color drawn after the fill.
    pub border: Option<Color>,
    /// Corner radius for rounded fills. 0 = sharp rectangle.
    pub radius: u32,
}

impl Material {
    pub const MAX_NOISE: u8 = 64;

    /// Opaque solid fill.
    pub const fn solid(tint: Color) -> Self {
        Self {
            kind: MaterialKind::Solid,
            tint,
            opacity: 255,
            noise_strength: 0,
            border: None,
            radius: 0,
        }
    }

    /// Tinted fill with explicit opacity.
    pub const fn tinted(tint: Color, opacity: u8) -> Self {
        Self {
            kind: MaterialKind::Tinted,
            tint,
            opacity,
            noise_strength: 0,
            border: None,
            radius: 0,
        }
    }

    /// Glass fill with default subtle noise (~2%).
    pub const fn glass(tint: Color, opacity: u8) -> Self {
        Self {
            kind: MaterialKind::Glass,
            tint,
            opacity,
            noise_strength: 5,
            border: None,
            radius: 0,
        }
    }

    pub const fn with_noise(mut self, strength: u8) -> Self {
        self.noise_strength = strength;
        self
    }

    pub const fn with_border(mut self, border: Color) -> Self {
        self.border = Some(border);
        self
    }

    pub const fn with_radius(mut self, radius: u32) -> Self {
        self.radius = radius;
        self
    }

    pub const fn without_border(mut self) -> Self {
        self.border = None;
        self
    }

    pub const fn opaque_fallback(mut self) -> Self {
        self.opacity = 255;
        self.noise_strength = 0;
        self
    }

    /// Clamp opacity and noise to safe ranges.
    pub const fn clamp(mut self) -> Self {
        // opacity already u8; keep 0..=255 as-is
        if self.noise_strength > Self::MAX_NOISE {
            self.noise_strength = Self::MAX_NOISE;
        }
        match self.kind {
            MaterialKind::Solid => {
                self.opacity = 255;
                self.noise_strength = 0;
            }
            MaterialKind::Tinted => {
                self.noise_strength = 0;
            }
            MaterialKind::Glass => {}
        }
        self
    }

    /// Conservative role defaults for shell chrome.
    ///
    /// Application content is **not** glass — opaque by policy.
    pub fn for_role(role: SurfaceRole, theme: &Theme) -> Self {
        let materials = MaterialPalette::new(theme);
        match role {
            SurfaceRole::ApplicationWindow => materials.solid_content,
            SurfaceRole::Panel => Self::tinted(theme.panel, 240) // ~94%
                .with_noise(2)
                .with_border(theme.border)
                .with_radius(10)
                .clamp(),
            SurfaceRole::Dock => Self::tinted(theme.panel, 242) // ~95%
                .with_noise(2)
                .with_border(theme.border)
                .with_radius(10)
                .clamp(),
            SurfaceRole::PopupOrMenu => materials.overlay_glass,
            SurfaceRole::Tooltip => Self::tinted(theme.panel_alt, 245)
                .with_noise(0)
                .with_border(theme.border.lighten(20))
                .with_radius(6)
                .clamp(),
            SurfaceRole::SystemOverlay => Self::glass(theme.panel, 210)
                .with_noise(4)
                .with_border(theme.border)
                .with_radius(8)
                .clamp(),
        }
    }

    /// Titlebar / decoration backplate glass (restrained).
    pub fn titlebar_glass(_theme: &Theme, active: bool) -> Self {
        let materials = MaterialPalette::new(_theme);
        if active {
            materials.titlebar_active
        } else {
            materials.titlebar_inactive
        }
    }

    /// Control-panel card / disclosure group surface.
    pub fn card(theme: &Theme) -> Self {
        MaterialPalette::new(theme).card_glass
    }

    pub fn readability_role(self) -> ReadabilityRole {
        if self.opacity >= 220 {
            ReadabilityRole::Primary
        } else {
            ReadabilityRole::Muted
        }
    }

    /// Readable foreground for text drawn on this material.
    pub fn readable_fg(self, theme: &Theme) -> Color {
        // Prefer theme text; dim slightly on inactive-style glass.
        match self.kind {
            MaterialKind::Solid | MaterialKind::Tinted => {
                if self.opacity >= 200 {
                    theme.text
                } else {
                    theme.text_muted
                }
            }
            MaterialKind::Glass => {
                if self.opacity >= 220 {
                    theme.text
                } else {
                    theme.text_muted
                }
            }
        }
    }

    /// Whether a subtle 1-px ambient text shadow is appropriate.
    pub fn wants_text_ambient_shadow(self) -> bool {
        matches!(self.kind, MaterialKind::Glass) && self.opacity < 240
    }

    /// Premultiplied-friendly straight-alpha color for blending over a
    /// destination (matches existing [`Color::blend_over`] arithmetic).
    pub fn sample_color(self, x: i32, y: i32) -> Color {
        let m = self.clamp();
        let mut r = m.tint.r() as i16;
        let mut g = m.tint.g() as i16;
        let mut b = m.tint.b() as i16;
        if matches!(m.kind, MaterialKind::Glass) && m.noise_strength > 0 {
            let n = noise_sample(x, y) as i16; // 0..=255
            // Map noise around 0 with amplitude ~ noise_strength.
            let delta = ((n - 128) * m.noise_strength as i16) / 255;
            r = (r + delta).clamp(0, 255);
            g = (g + delta).clamp(0, 255);
            b = (b + delta).clamp(0, 255);
        }
        let a = match m.kind {
            MaterialKind::Solid => 255,
            MaterialKind::Tinted | MaterialKind::Glass => m.opacity,
        };
        Color::rgba(r as u8, g as u8, b as u8, a)
    }
}

// ── Static deterministic noise tile ──────────────────────────────────────────

/// 16×16 tile generated once with a fixed LCG seed. Spatially stable; never
/// reallocated per frame or per window.
const NOISE_TILE_N: usize = 16;
const NOISE_TILE: [u8; NOISE_TILE_N * NOISE_TILE_N] = generate_noise_tile();

const fn generate_noise_tile() -> [u8; NOISE_TILE_N * NOISE_TILE_N] {
    let mut out = [0u8; NOISE_TILE_N * NOISE_TILE_N];
    // Numerical Recipes LCG constants; fixed seed → deterministic.
    let mut state: u32 = 0xA5A5_5A5A;
    let mut i = 0;
    while i < out.len() {
        state = state
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        out[i] = (state >> 16) as u8;
        i += 1;
    }
    out
}

/// Sample the static noise tile at absolute pixel coordinates.
#[inline]
pub fn noise_sample(x: i32, y: i32) -> u8 {
    let ux = (x.rem_euclid(NOISE_TILE_N as i32)) as usize;
    let uy = (y.rem_euclid(NOISE_TILE_N as i32)) as usize;
    NOISE_TILE[uy * NOISE_TILE_N + ux]
}

/// Size of the reusable noise tile (bytes). Exposed for memory accounting tests.
pub const fn noise_tile_bytes() -> usize {
    NOISE_TILE_N * NOISE_TILE_N
}

/// Blend arithmetic used by materials (straight alpha over XRGB/ARGB dst).
/// Kept free-standing so unit tests can assert formula stability.
#[inline]
pub fn blend_straight_over(src: Color, dst: Color) -> Color {
    src.blend_over(dst)
}

/// Clamp helper for external validation (opacity 0..=255, noise 0..=MAX_NOISE).
pub fn validate_material_params(opacity: u8, noise: u8) -> (u8, u8) {
    (opacity, noise.min(Material::MAX_NOISE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_and_noise_are_clamped() {
        let m = Material::glass(Color::rgb(10, 20, 30), 200)
            .with_noise(255)
            .clamp();
        assert_eq!(m.noise_strength, Material::MAX_NOISE);
        assert_eq!(m.opacity, 200);

        let solid = Material::solid(Color::rgb(1, 2, 3))
            .with_noise(40)
            .clamp();
        assert_eq!(solid.opacity, 255);
        assert_eq!(solid.noise_strength, 0);

        let (o, n) = validate_material_params(10, 200);
        assert_eq!(o, 10);
        assert_eq!(n, Material::MAX_NOISE);
    }

    #[test]
    fn noise_is_deterministic_and_spatially_stable() {
        assert_eq!(noise_sample(0, 0), noise_sample(0, 0));
        assert_eq!(noise_sample(3, 7), noise_sample(3 + 16, 7 + 16));
        assert_eq!(noise_sample(-1, -1), noise_sample(15, 15));
        // Not a flat constant — tile has variation.
        let a = noise_sample(0, 0);
        let b = noise_sample(1, 0);
        let c = noise_sample(0, 1);
        assert!(a != b || b != c || a != c);
        assert_eq!(noise_tile_bytes(), 256);
    }

    #[test]
    fn straight_alpha_blend_matches_color_blend_over() {
        let src = Color::rgba(255, 0, 0, 128);
        let dst = Color::rgb(0, 0, 255);
        assert_eq!(blend_straight_over(src, dst), src.blend_over(dst));
        assert_eq!(blend_straight_over(Color::TRANSPARENT, dst), dst);
        assert_eq!(
            blend_straight_over(Color::rgb(1, 2, 3), dst).0 & 0x00FF_FFFF,
            0x0001_0203
        );
    }

    #[test]
    fn straight_alpha_over_transparent_keeps_unpremultiplied_rgb() {
        let src = Color::rgba(240, 80, 20, 128);
        let out = blend_straight_over(src, Color::TRANSPARENT);
        assert_eq!(out, src);
    }

    #[test]
    fn straight_alpha_layers_preserve_alpha_and_normalized_color() {
        let lower = Color::rgba(0, 0, 255, 128);
        let upper = Color::rgba(255, 0, 0, 128);
        let out = upper.blend_over(lower);
        assert_eq!(out.a(), 192);
        assert_eq!((out.r(), out.g(), out.b()), (170, 0, 85));
    }

    #[test]
    fn solid_sample_is_opaque_and_ignores_noise() {
        let m = Material::solid(Color::rgb(0x12, 0x34, 0x56)).with_noise(40);
        let c = m.sample_color(4, 4);
        assert_eq!(c.a(), 255);
        assert_eq!(c.r(), 0x12);
        assert_eq!(c.g(), 0x34);
        assert_eq!(c.b(), 0x56);
    }

    #[test]
    fn glass_sample_applies_opacity() {
        let m = Material::glass(Color::rgb(0x20, 0x20, 0x28), 200).with_noise(0);
        let c = m.sample_color(0, 0);
        assert_eq!(c.a(), 200);
    }

    #[test]
    fn role_presets_keep_application_content_opaque() {
        let theme = Theme::sunlight_dark();
        let app = Material::for_role(SurfaceRole::ApplicationWindow, &theme);
        assert_eq!(app.kind, MaterialKind::Solid);
        assert_eq!(app.opacity, 255);

        let panel = Material::for_role(SurfaceRole::Panel, &theme).clamp();
        assert!(panel.opacity >= 230); // ~90%+
        assert!(panel.noise_strength <= 4);
    }

    #[test]
    fn canonical_start_values_are_preserved_and_shared() {
        let theme = Theme::sunlight_dark();
        let palette = MaterialPalette::new(&theme);
        let start = palette.overlay_glass;
        assert_eq!(start.tint, Color::rgb(0x1C, 0x1C, 0x1F));
        assert_eq!(start.opacity, 232);
        assert_eq!(start.noise_strength, 0);
        assert_eq!(start.border, Some(Color::rgb(0x35, 0x35, 0x40)));
        assert_eq!(start.radius, 12);
        assert_eq!(
            Material::for_role(SurfaceRole::PopupOrMenu, &theme),
            start
        );
        // Start menu and window root share the canonical charcoal tint.
        assert_eq!(palette.window_glass.tint, start.tint);
        assert_eq!(palette.window_glass.opacity, start.opacity);
    }

    #[test]
    fn titlebar_and_root_share_hue_family() {
        let theme = Theme::sunlight_dark();
        let palette = MaterialPalette::new(&theme);
        assert!(palette
            .window_glass
            .tint
            .same_hue_family(palette.titlebar_active.tint));
        assert!(palette
            .window_glass
            .tint
            .same_hue_family(palette.titlebar_inactive.tint));
        // Active is denser (higher opacity) than window root.
        assert!(palette.titlebar_active.opacity >= palette.window_glass.opacity);
        // Inactive sits near root density — not a separate cold slab.
        assert!(
            palette
                .titlebar_inactive
                .opacity
                .abs_diff(palette.window_glass.opacity)
                <= 4
        );
        assert_eq!(palette.titlebar_active.tint, theme.chrome.titlebar_active);
        assert_eq!(
            palette.titlebar_inactive.tint,
            theme.chrome.titlebar_inactive
        );
    }

    #[test]
    fn window_hierarchy_keeps_cards_denser_and_content_opaque() {
        let palette = MaterialPalette::new(&Theme::sunlight_dark());
        assert!(palette.card_glass.opacity > palette.window_glass.opacity);
        assert_eq!(palette.solid_content.opacity, 255);
        assert_eq!(palette.solid_content.kind, MaterialKind::Solid);
        assert!(palette.tinted_content.opacity >= palette.card_glass.opacity);
    }

    #[test]
    fn opaque_accessibility_fallback_disables_alpha_and_noise() {
        let palette = MaterialPalette::new(&Theme::sunlight_dark()).opaque();
        for material in [
            palette.overlay_glass,
            palette.window_glass,
            palette.titlebar_active,
            palette.titlebar_inactive,
            palette.card_glass,
            palette.solid_content,
            palette.tinted_content,
        ] {
            assert_eq!(material.opacity, 255);
            assert_eq!(material.noise_strength, 0);
        }
    }

    #[test]
    fn outer_effect_geometry_expands_the_window_corner() {
        let geometry = DecorationGeometry::SUNLIGHT;
        assert_eq!(
            geometry.outer_shadow_corner_radius(),
            geometry.window_corner_radius + geometry.ambient_shadow_falloff
        );
        assert_eq!(
            geometry.outer_focus_corner_radius(),
            geometry.window_corner_radius + geometry.solar_focus_falloff
        );
        // KWin-style ambient is intentionally wider than a tight 12px strip.
        assert!(geometry.ambient_shadow_falloff >= 24);
        assert!(geometry.ambient_shadow_offset_y >= 4);
        // Mac-style hairline, not a thick frame.
        assert_eq!(geometry.structural_rim, 1);
    }

    #[test]
    fn text_ambient_shadow_only_for_low_contrast_glass() {
        let solid = Material::solid(Color::rgb(0, 0, 0));
        assert!(!solid.wants_text_ambient_shadow());
        let glass_high = Material::glass(Color::rgb(0, 0, 0), 250);
        assert!(!glass_high.wants_text_ambient_shadow());
        let glass_low = Material::glass(Color::rgb(0, 0, 0), 180);
        assert!(glass_low.wants_text_ambient_shadow());
    }
}
