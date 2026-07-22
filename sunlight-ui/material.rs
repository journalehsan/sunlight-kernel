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
        match role {
            SurfaceRole::ApplicationWindow => Self::solid(theme.bg).with_radius(0),
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
            SurfaceRole::PopupOrMenu => Self::tinted(theme.panel, 232) // ~91%
                .with_noise(3)
                .with_border(theme.border)
                .with_radius(12)
                .clamp(),
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
        let tint = if active {
            Color::rgb(0x1E, 0x1E, 0x26)
        } else {
            Color::rgb(0x2B, 0x2B, 0x36)
        };
        Self::glass(tint, if active { 245 } else { 235 })
            .with_noise(if active { 4 } else { 3 })
            .with_radius(0)
            .clamp()
    }

    /// Control-panel card / disclosure group surface.
    pub fn card(theme: &Theme) -> Self {
        Self::tinted(theme.panel, 250)
            .with_noise(2)
            .with_border(theme.border)
            .with_radius(8)
            .clamp()
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
    fn text_ambient_shadow_only_for_low_contrast_glass() {
        let solid = Material::solid(Color::rgb(0, 0, 0));
        assert!(!solid.wants_text_ambient_shadow());
        let glass_high = Material::glass(Color::rgb(0, 0, 0), 250);
        assert!(!glass_high.wants_text_ambient_shadow());
        let glass_low = Material::glass(Color::rgb(0, 0, 0), 180);
        assert!(glass_low.wants_text_ambient_shadow());
    }
}
