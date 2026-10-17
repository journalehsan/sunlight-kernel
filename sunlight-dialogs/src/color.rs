//! Color math shared by native dialogs. HEX is RGB/RGBA, never packed ARGB.
use alloc::{format, string::String};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Accept exactly six digits (optionally prefixed with #), or eight when
    /// alpha is enabled. Incomplete edits never produce a color.
    pub fn parse_hex(text: &str, allow_alpha: bool) -> Option<Self> {
        let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
        if !(digits.len() == 6 || (allow_alpha && digits.len() == 8))
            || !digits.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        let (rgb, a) = if digits.len() == 8 {
            (value >> 8, value as u8)
        } else {
            (value, 255)
        };
        Some(Self {
            r: (rgb >> 16) as u8,
            g: (rgb >> 8) as u8,
            b: rgb as u8,
            a,
        })
    }

    pub fn to_hex(self, alpha: bool) -> String {
        if alpha {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        } else {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        }
    }

    pub const fn argb(self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    pub fn hsv(self) -> Hsv {
        let r = self.r as f32 / 255.;
        let g = self.g as f32 / 255.;
        let b = self.b as f32 / 255.;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0. {
            0.
        } else if max == r {
            60. * ((g - b) / d)
        } else if max == g {
            60. * ((b - r) / d + 2.)
        } else {
            60. * ((r - g) / d + 4.)
        };
        Hsv {
            h: wrap_hue(h),
            s: if max == 0. { 0. } else { d / max },
            v: max,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

pub fn wrap_hue(h: f32) -> f32 {
    if !h.is_finite() {
        return 0.;
    }
    let h = h % 360.;
    if h < 0. {
        h + 360.
    } else {
        h
    }
}

impl Hsv {
    pub fn rgba(self, a: u8) -> Rgba {
        let h = wrap_hue(self.h) / 60.;
        let s = unit(self.s);
        let v = unit(self.v);
        let c = v * s;
        let x = c * (1. - ((h % 2.) - 1.).abs());
        let (r, g, b) = match h as u8 {
            0 => (c, x, 0.),
            1 => (x, c, 0.),
            2 => (0., c, x),
            3 => (0., x, c),
            4 => (x, 0., c),
            _ => (c, 0., x),
        };
        let m = v - c;
        Rgba {
            r: byte(r + m),
            g: byte(g + m),
            b: byte(b + m),
            a,
        }
    }
}

fn unit(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0., 1.)
    } else {
        0.
    }
}

pub fn byte(v: f32) -> u8 {
    (unit(v) * 255. + 0.5) as u8
}

/// Inclusive pixel endpoints; safe for empty/single-pixel tracks and captured
/// pointer coordinates far outside a control.
pub fn axis_fraction(position: i32, origin: i32, length: u32) -> f32 {
    if length <= 1 {
        return 0.;
    }
    ((position as i64 - origin as i64) as f32 / (length - 1) as f32).clamp(0., 1.)
}

pub const PRESETS: [Rgba; 10] = [
    Rgba::rgb(0, 0, 0),
    Rgba::rgb(255, 255, 255),
    Rgba::rgb(255, 0, 0),
    Rgba::rgb(0, 192, 0),
    Rgba::rgb(0, 0, 255),
    Rgba::rgb(255, 255, 0),
    Rgba::rgb(255, 165, 0),
    Rgba::rgb(128, 0, 128),
    Rgba::rgb(0, 255, 255),
    Rgba::rgb(128, 128, 128),
];

/// Session-local, bounded MRU. The host records only confirmed selections.
#[derive(Clone, Default)]
pub struct RecentColors(heapless::Vec<Rgba, 8>);

impl RecentColors {
    pub fn colors(&self) -> &[Rgba] {
        &self.0
    }

    pub fn record(&mut self, color: Rgba) {
        if let Some(i) = self.0.iter().position(|c| *c == color) {
            self.0.remove(i);
        }
        if self.0.is_full() {
            self.0.pop();
        }
        let _ = self.0.insert(0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_boundaries_and_alpha_order() {
        for text in ["#000000", "#FFFFFF", "#FFAA00", "#010203"] {
            let c = Rgba::parse_hex(text, false).unwrap();
            assert_eq!(c.to_hex(false), text);
            assert_eq!(c.a, 255);
        }
        let c = Rgba::parse_hex(" #12ab3400 ", true).unwrap();
        assert_eq!(c.to_hex(true), "#12AB3400");
        assert_eq!(c.argb(), 0x0012ab34);
        assert_eq!(
            Rgba::parse_hex("ffaa00", false).unwrap().to_hex(false),
            "#FFAA00"
        );
        assert_eq!(
            Rgba::parse_hex("#000000", true).unwrap().to_hex(true),
            "#000000FF"
        );
        for bad in [
            "", "#", "#123", "#12345", "#1234567", "#GG1234", "##123456", "0x123456", "#12é34",
        ] {
            assert!(Rgba::parse_hex(bad, true).is_none(), "{bad}");
        }
        assert!(Rgba::parse_hex("#11223344", false).is_none());
    }

    #[test]
    fn primaries_grays_and_roundtrips() {
        for (c, h) in [
            (Rgba::rgb(255, 0, 0), 0.),
            (Rgba::rgb(0, 255, 0), 120.),
            (Rgba::rgb(0, 0, 255), 240.),
        ] {
            assert_eq!(c.hsv(), Hsv { h, s: 1., v: 1. });
            assert_eq!(c.hsv().rgba(255), c);
        }
        for gray in 0..=255 {
            let c = Rgba::rgb(gray, gray, gray);
            assert_eq!(c.hsv().s, 0.);
            assert_eq!(c.hsv().rgba(255), c);
        }
        for r in (0..=255).step_by(17) {
            for g in (0..=255).step_by(17) {
                for b in (0..=255).step_by(17) {
                    let c = Rgba::rgb(r, g, b);
                    assert_eq!(c.hsv().rgba(255), c);
                }
            }
        }
    }

    #[test]
    fn wrapping_clamping_and_opacity() {
        for h in [-720., -360., 0., 360., 720.] {
            assert_eq!(
                Hsv { h, s: 1., v: 1. }.rgba(128),
                Rgba {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 128
                }
            );
        }
        assert_eq!(wrap_hue(-60.), 300.);
        assert_eq!(byte(0.), 0);
        assert_eq!(byte(0.5), 128);
        assert_eq!(byte(1.), 255);
        assert_eq!(
            Hsv {
                h: 0.,
                s: 3.,
                v: -1.
            }
            .rgba(0),
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 0
            }
        );
    }

    #[test]
    fn pixel_mapping_and_recent_capacity() {
        assert_eq!(axis_fraction(10, 10, 101), 0.);
        assert_eq!(axis_fraction(60, 10, 101), 0.5);
        assert_eq!(axis_fraction(110, 10, 101), 1.);
        assert_eq!(axis_fraction(i32::MIN, 10, 101), 0.);
        assert_eq!(axis_fraction(i32::MAX, 10, 101), 1.);
        assert_eq!(axis_fraction(42, 0, 0), 0.);
        assert_eq!(axis_fraction(42, 0, 1), 0.);
        let mut recent = RecentColors::default();
        for c in PRESETS {
            recent.record(c);
        }
        assert_eq!(recent.colors().len(), 8);
        recent.record(PRESETS[9]);
        assert_eq!(recent.colors().len(), 8);
        assert_eq!(recent.colors()[0], PRESETS[9]);
    }
}
