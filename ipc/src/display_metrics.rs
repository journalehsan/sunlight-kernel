//! Central display geometry types and layout helpers.
//!
//! `sunlight-display` owns the live metrics; clients query them via
//! `SgpMsg::GET_SCREEN_INFO`. These helpers keep shell, compositor, and input
//! code aligned on the same bounds and stride math.

/// Fixed-point scale where `SCALE_FP_ONE` = 1.0×. HiDPI scaling is not
/// applied yet; the field exists for future settings integration.
pub const SCALE_FP_ONE: u32 = 65536;

/// Safe compositor allocation when no backend reports a usable size.
pub const SAFE_FALLBACK_W: u32 = 1280;
pub const SAFE_FALLBACK_H: u32 = 800;

/// Upper sanity bound on any reported dimension.
pub const MAX_DIM: u32 = 16384;

/// Minimum sane dimension for client-side clamps.
pub const MIN_DIM: u32 = 320;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PixelFormat {
    Xrgb8888 = 0,
    Unknown = 255,
}

impl PixelFormat {
    pub const fn from_discriminant(v: u8) -> Self {
        match v {
            0 => Self::Xrgb8888,
            _ => Self::Unknown,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Xrgb8888 => "xrgb8888",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScreenBackend {
    LimineFramebuffer = 0,
    VirtioGpu = 1,
    Fallback = 2,
}

impl ScreenBackend {
    pub const fn from_discriminant(v: u8) -> Self {
        match v {
            0 => Self::LimineFramebuffer,
            1 => Self::VirtioGpu,
            2 => Self::Fallback,
            _ => Self::Fallback,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LimineFramebuffer => "limine-framebuffer",
            Self::VirtioGpu => "virtio-gpu",
            Self::Fallback => "fallback",
        }
    }
}

/// Authoritative screen geometry for layout, input clamping, and framebuffer math.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMetrics {
    pub width_px: u32,
    pub height_px: u32,
    pub stride_bytes: u32,
    pub scale_fp: u32,
    pub refresh_hz: Option<u32>,
    pub pixel_format: PixelFormat,
    pub backend: ScreenBackend,
}

/// Full-screen target rectangle for wallpaper/desktop fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Validate a backend-reported `(width, height)`.
pub fn validate_size(w: u32, h: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
        None
    } else {
        Some((w, h))
    }
}

impl DisplayMetrics {
    pub const fn safe_fallback() -> Self {
        Self {
            width_px: SAFE_FALLBACK_W,
            height_px: SAFE_FALLBACK_H,
            stride_bytes: SAFE_FALLBACK_W * 4,
            scale_fp: SCALE_FP_ONE,
            refresh_hz: None,
            pixel_format: PixelFormat::Xrgb8888,
            backend: ScreenBackend::Fallback,
        }
    }

    pub const fn new(
        width_px: u32,
        height_px: u32,
        stride_bytes: u32,
        pixel_format: PixelFormat,
        backend: ScreenBackend,
    ) -> Self {
        Self {
            width_px,
            height_px,
            stride_bytes,
            scale_fp: SCALE_FP_ONE,
            refresh_hz: None,
            pixel_format,
            backend,
        }
    }

    pub const fn stride_words(&self) -> usize {
        (self.stride_bytes / 4) as usize
    }

    pub fn wallpaper_target_rect(self) -> ScreenRect {
        ScreenRect {
            x: 0,
            y: 0,
            w: self.width_px,
            h: self.height_px,
        }
    }

    pub fn pixel_offset(self, x: u32, y: u32) -> usize {
        y as usize * self.stride_words() + x as usize
    }

    pub fn clamp_point(self, x: u32, y: u32) -> (u32, u32) {
        (
            x.min(self.width_px.saturating_sub(1)),
            y.min(self.height_px.saturating_sub(1)),
        )
    }

    pub fn clamp_i32_point(self, x: i32, y: i32) -> (i32, i32) {
        let max_x = (self.width_px.saturating_sub(1)) as i32;
        let max_y = (self.height_px.saturating_sub(1)) as i32;
        (x.clamp(0, max_x), y.clamp(0, max_y))
    }

    /// Keep a window chrome rectangle fully visible, honoring a minimum Y (e.g. top panel).
    pub fn fit_window_origin(
        self,
        chrome_w: u32,
        chrome_h: u32,
        min_y: u32,
        mut x: u32,
        mut y: u32,
    ) -> (u32, u32) {
        if chrome_w >= self.width_px {
            x = 0;
        } else {
            x = x.min(self.width_px.saturating_sub(chrome_w));
        }

        let max_y = self.height_px.saturating_sub(chrome_h).max(min_y);
        y = y.clamp(min_y, max_y);
        (x, y)
    }

    /// Default cascaded placement for a normal app window.
    pub fn initial_window_origin(
        self,
        win_id: u64,
        client_w: u32,
        client_h: u32,
        chrome_w: u32,
        chrome_h: u32,
        min_y: u32,
    ) -> (u32, u32) {
        let cascade = ((win_id.saturating_sub(1)) % 8) as u32 * 28;
        let x = self
            .width_px
            .saturating_sub(client_w)
            .saturating_div(2)
            .saturating_add(cascade);
        let y = ((self.height_px / 4)
            .saturating_sub(client_h / 2)
            .saturating_add(cascade))
        .max(min_y);
        self.fit_window_origin(chrome_w, chrome_h, min_y, x, y)
    }

    /// Pack `GET_SCREEN_INFO` reply words (backward compatible: word 0 unchanged).
    pub fn pack_reply_words(self) -> [u64; 4] {
        let wh = (self.width_px as u64) | ((self.height_px as u64) << 32);
        let stride_fmt = (self.stride_bytes as u64) | ((self.pixel_format as u64) << 32);
        let meta = (self.scale_fp as u64) | ((self.backend as u64) << 32);
        let refresh = self.refresh_hz.unwrap_or(0) as u64;
        [wh, stride_fmt, meta, refresh]
    }

    /// Decode a `GET_SCREEN_INFO` reply. Older servers may only populate word 0.
    pub fn from_reply(words: &[u64]) -> Self {
        let packed = words.first().copied().unwrap_or(0);
        let width_px = (packed & 0xFFFF_FFFF) as u32;
        let height_px = (packed >> 32) as u32;

        let stride_fmt = words.get(1).copied().unwrap_or(0);
        let stride_bytes = if stride_fmt != 0 {
            (stride_fmt & 0xFFFF_FFFF) as u32
        } else {
            width_px.saturating_mul(4)
        };
        let pixel_format = PixelFormat::from_discriminant((stride_fmt >> 32) as u8);

        let meta = words.get(2).copied().unwrap_or(0);
        let scale_fp = if meta != 0 {
            (meta & 0xFFFF_FFFF) as u32
        } else {
            SCALE_FP_ONE
        };
        let backend = ScreenBackend::from_discriminant((meta >> 32) as u8);

        let refresh_word = words.get(3).copied().unwrap_or(0);
        let refresh_hz = if refresh_word != 0 {
            Some(refresh_word as u32)
        } else {
            None
        };

        if validate_size(width_px, height_px).is_some() {
            Self {
                width_px,
                height_px,
                stride_bytes,
                scale_fp,
                refresh_hz,
                pixel_format,
                backend,
            }
        } else {
            Self::safe_fallback()
        }
    }

    /// Clamp to minimum sane dimensions for client bootstrapping.
    pub fn clamped_for_clients(self) -> Self {
        Self {
            width_px: self.width_px.max(MIN_DIM),
            height_px: self.height_px.max(240),
            ..self
        }
    }
}

/// Chrome border width used by the compositor when fitting window placement.
pub const BORDER_W: u32 = 1;
