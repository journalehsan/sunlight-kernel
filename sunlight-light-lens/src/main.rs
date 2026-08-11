#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

use core::cmp::Ordering;

use sun_font::{
    draw_text as sf_draw, draw_text_centered as sf_centered, draw_text_right as sf_right,
    draw_text_vcenter as sf_vcenter, line_height as sf_lh, measure_text as sf_measure, FontRole,
    TextStyle,
};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_libc::{self as libc, crt0, DirEntry, FT_FILE};
use sunlight_ui::image::{decode_simg, RgbaImage, TgaImage};
use sunlight_ui::widgets::StatusBar;
use sunlight_ui::{
    request_close, App, AxisSizing, Canvas, Color, Column, Event, LayoutBox, LayoutInvalidation,
    Point, Rect, Row, Size, Sizing, Theme, UiSymbol, Window, WindowConfig, WindowDecoration,
    WindowEvent, WindowMaterial,
};

const WIN_W: u32 = 1220;
const WIN_H: u32 = 760;
const HEADER_H: u32 = 36;
const TOOLBAR_H: u32 = 52;
const STATUS_H: u32 = StatusBar::HEIGHT;
const OUTER_PAD: i32 = 10;
const GAP: i32 = 10;
const LEFT_W: u32 = 188;
const RIGHT_W: u32 = 280;
/// Reserved left column (px) for info-panel row labels.
const INFO_LABEL_W: i32 = 84;
const MAX_PATH: usize = 256;
const MAX_SIBLINGS: usize = 128;
const MAX_DIR_ENTRIES: usize = 160;
const MSG_LEN: usize = 96;
const VALUE_LEN: usize = 96;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

const KEY_ESC: u8 = 0x01;
const KEY_Q: u8 = 0x10;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;

// Material Icons font rasterised at build time (minitype pipeline) for smaller
// footprint and monochrome-friendly action/app icons.
static APP_ICON_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_app.tga"));
static MISSING_ICON_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_missing.tga"));

static mut IMAGE_BUF: [u8; MAX_IMAGE_BYTES] = [0u8; MAX_IMAGE_BYTES];
static mut IMAGE_LEN: usize = 0;

// Global allocator comes from sunlight-libc (`global-alloc` feature) so SIMG v2
// can allocate a single decoded pixel buffer.

#[derive(Clone, Copy, PartialEq, Eq)]
struct PathBuf {
    buf: [u8; MAX_PATH],
    len: usize,
}

impl PathBuf {
    const fn empty() -> Self {
        Self {
            buf: [0; MAX_PATH],
            len: 0,
        }
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() >= MAX_PATH || bytes.contains(&0) {
            return None;
        }
        let mut out = Self::empty();
        out.buf[..bytes.len()].copy_from_slice(bytes);
        out.len = bytes.len();
        Some(out)
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn file_name_bytes(&self) -> &[u8] {
        let bytes = self.as_bytes();
        let start = bytes
            .iter()
            .rposition(|&b| b == b'/')
            .map(|idx| idx + 1)
            .unwrap_or(0);
        &bytes[start..]
    }

    fn file_name_str(&self) -> &str {
        core::str::from_utf8(self.file_name_bytes()).unwrap_or("")
    }

    fn parent(&self) -> Option<Self> {
        if self.len <= 1 {
            return None;
        }
        let bytes = self.as_bytes();
        let mut end = self.len;
        while end > 1 && bytes[end - 1] == b'/' {
            end -= 1;
        }
        let mut pos = end;
        while pos > 0 && bytes[pos - 1] != b'/' {
            pos -= 1;
        }
        if pos == 0 {
            return None;
        }
        if pos == 1 {
            return Self::from_bytes(b"/");
        }
        Self::from_bytes(&bytes[..pos - 1])
    }

    fn join(&self, name: &[u8]) -> Option<Self> {
        if name.is_empty() || name.contains(&b'/') {
            return None;
        }
        let sep = if self.len == 1 && self.buf[0] == b'/' {
            0
        } else {
            1
        };
        let total = self.len + sep + name.len();
        if total >= MAX_PATH {
            return None;
        }
        let mut out = Self::empty();
        out.buf[..self.len].copy_from_slice(self.as_bytes());
        let mut cursor = self.len;
        if sep != 0 {
            out.buf[cursor] = b'/';
            cursor += 1;
        }
        out.buf[cursor..cursor + name.len()].copy_from_slice(name);
        out.len = cursor + name.len();
        Some(out)
    }
}

#[derive(Clone, Copy)]
struct TextBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> TextBuf<N> {
    const fn empty() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn set_str(&mut self, text: &str) {
        self.set_bytes(text.as_bytes());
    }

    fn set_bytes(&mut self, text: &[u8]) {
        self.len = text.len().min(N);
        self.buf[..self.len].copy_from_slice(&text[..self.len]);
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    /// Truncate the buffer in place so it renders within `max_w` pixels when
    /// drawn with `role`, appending ".." when content is cut. The MiniType
    /// font is proportional, so width is measured per glyph rather than
    /// derived from a fixed character count.
    fn truncate_to_width(&mut self, max_w: i32, role: FontRole) -> &str {
        if sf_measure(self.as_str(), role).w as i32 <= max_w {
            return self.as_str();
        }
        let dots = "..";
        let budget = max_w.saturating_sub(sf_measure(dots, role).w as i32).max(0) as u32;
        // Walk glyphs left-to-right, accumulating their real advance widths,
        // and keep the longest prefix that still fits within `budget`. The
        // borrow of `full` is scoped to this loop so `self` can be mutated
        // afterwards.
        let cut = {
            let full = self.as_str();
            let mut acc: u32 = 0;
            let mut cut = 0usize;
            for (byte_idx, ch) in full.char_indices() {
                let next = byte_idx + ch.len_utf8();
                let glyph_w = sf_measure(&full[byte_idx..next], role).w;
                if acc + glyph_w > budget {
                    break;
                }
                acc += glyph_w;
                cut = next;
            }
            cut
        };
        self.len = cut;
        let room = self.buf.len() - self.len;
        let add = dots.len().min(room);
        self.buf[self.len..self.len + add].copy_from_slice(&dots.as_bytes()[..add]);
        self.len += add;
        self.as_str()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Empty,
    Ready,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormatKind {
    /// Historical `.simg` that is still TGA type-2 on disk.
    SimgLegacy,
    /// Versioned SIMG v2 container (magic `SIMG`).
    SimgV2,
    Tga,
    Unknown,
}

#[derive(Clone, Copy)]
struct ImageInfo {
    size_bytes: u64,
    width: u32,
    height: u32,
    bpp: u8,
    format: FormatKind,
}

impl ImageInfo {
    const fn empty() -> Self {
        Self {
            size_bytes: 0,
            width: 0,
            height: 0,
            bpp: 0,
            format: FormatKind::Unknown,
        }
    }
}

struct LightLensApp {
    current_path: PathBuf,
    sibling_paths: [PathBuf; MAX_SIBLINGS],
    sibling_count: usize,
    current_index: usize,
    has_current_index: bool,
    /// Decoded photo pixels (straight ARGB u32). Prefer this over a TGA view so
    /// we can force-opaque blit the same way the File Manager preview fixed the
    /// gray SIMG v2 hole on alpha-capable surfaces.
    image: Option<RgbaImage>,
    info: ImageInfo,
    load_state: LoadState,
    message: TextBuf<MSG_LEN>,
    app_icon: Option<TgaImage>,
    missing_icon: Option<TgaImage>,
    /// Authoritative drawable bounds supplied by the application host.
    client_bounds: Rect,
    layout_invalidation: LayoutInvalidation,
    layout: LightLensLayout,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LightLensLayout {
    root: Rect,
    header: Rect,
    toolbar: Rect,
    body: Rect,
    left: Rect,
    /// Layout-owned image-viewer region. Preview chrome and image fitting stay
    /// inside this rectangle and never influence the surrounding layout.
    viewport: Rect,
    right: Rect,
    status: Rect,
    back: Rect,
    next: Rect,
    tools: [Rect; 6],
}

impl LightLensApp {
    fn new(initial_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            current_path: PathBuf::empty(),
            sibling_paths: [PathBuf::empty(); MAX_SIBLINGS],
            sibling_count: 0,
            current_index: 0,
            has_current_index: false,
            image: None,
            info: ImageInfo::empty(),
            load_state: LoadState::Empty,
            message: TextBuf::empty(),
            app_icon: TgaImage::parse(APP_ICON_TGA).ok(),
            missing_icon: TgaImage::parse(MISSING_ICON_TGA).ok(),
            client_bounds: Rect::new(0, 0, WIN_W, WIN_H),
            layout_invalidation: LayoutInvalidation::new(),
            layout: LightLensLayout::default(),
        };
        if let Some(path) = initial_path {
            app.open_path(path);
        } else {
            app.message.set_str("Open an image from Sunlight Files.");
        }
        app
    }

    fn compute_layout(root: Rect) -> LightLensLayout {
        let fill = Sizing::new(AxisSizing::Fill, AxisSizing::Fill);
        let fixed_height = |height| {
            LayoutBox::new(Rect::new(0, 0, 0, height))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(height)))
        };
        let mut root_children = [
            fixed_height(HEADER_H),
            fixed_height(TOOLBAR_H),
            LayoutBox::new(Rect::new(0, 0, 0, 0)).with_sizing(fill),
            fixed_height(STATUS_H),
        ];
        let _ = Column::new(root).arrange(&mut root_children);
        let header = root_children[0].bounds();
        let toolbar = root_children[1].bounds();
        let body = root_children[2].bounds();
        let status = root_children[3].bounds();

        let body_inner = body.inset(OUTER_PAD);
        let mut body_children = [
            LayoutBox::new(Rect::new(0, 0, LEFT_W, 0))
                .with_sizing(Sizing::new(AxisSizing::Fixed(LEFT_W), AxisSizing::Fill)),
            LayoutBox::new(Rect::new(0, 0, 0, 0)).with_sizing(fill),
            LayoutBox::new(Rect::new(0, 0, RIGHT_W, 0))
                .with_sizing(Sizing::new(AxisSizing::Fixed(RIGHT_W), AxisSizing::Fill)),
        ];
        let _ = Row::new(body_inner)
            .with_gap(GAP.max(0) as u32)
            .arrange(&mut body_children);

        // Preserve the existing left/right clusters. Only the middle spacer
        // consumes changing toolbar width.
        let tool_widths = [54u32, 54, 62, 62, 48, 56];
        let button_height = toolbar.h.saturating_sub(16);
        let toolbar_inner = Rect::new(
            toolbar.x + 10,
            toolbar.y + 8,
            toolbar.w.saturating_sub(20),
            button_height,
        );
        let fixed = |width| {
            LayoutBox::new(Rect::new(0, 0, width, button_height)).with_sizing(Sizing::new(
                AxisSizing::Fixed(width),
                AxisSizing::Fixed(button_height),
            ))
        };
        let mut toolbar_children = [
            fixed(84),
            fixed(84),
            LayoutBox::new(Rect::new(0, 0, 0, button_height)).with_sizing(Sizing::new(
                AxisSizing::Fill,
                AxisSizing::Fixed(button_height),
            )),
            fixed(tool_widths[0]),
            fixed(tool_widths[1]),
            fixed(tool_widths[2]),
            fixed(tool_widths[3]),
            fixed(tool_widths[4]),
            fixed(tool_widths[5]),
        ];
        let _ = Row::new(toolbar_inner)
            .with_gap(6)
            .arrange(&mut toolbar_children);

        LightLensLayout {
            root,
            header,
            toolbar,
            body,
            left: body_children[0].bounds(),
            viewport: body_children[1].bounds(),
            right: body_children[2].bounds(),
            status,
            back: toolbar_children[0].bounds(),
            next: toolbar_children[1].bounds(),
            tools: [
                toolbar_children[3].bounds(),
                toolbar_children[4].bounds(),
                toolbar_children[5].bounds(),
                toolbar_children[6].bounds(),
                toolbar_children[7].bounds(),
                toolbar_children[8].bounds(),
            ],
        }
    }

    fn ensure_layout(&mut self) -> bool {
        if !self.layout_invalidation.update(self.client_bounds) {
            return false;
        }
        self.layout = Self::compute_layout(self.client_bounds);
        true
    }

    fn set_client_bounds(&mut self, width: u32, height: u32) -> bool {
        let bounds = Rect::new(0, 0, width, height);
        if bounds == self.client_bounds {
            return false;
        }
        self.client_bounds = bounds;
        self.layout_invalidation.invalidate();
        self.ensure_layout()
    }

    /// Draw a titled panel using the MiniType vector font, matching the look
    /// of `Panel::with_title` but without the bitmap title that widget draws.
    /// Returns the content rect (everything below the title bar).
    fn draw_panel(canvas: &mut Canvas, theme: &Theme, rect: Rect, title: &str) -> Rect {
        const TITLE_H: u32 = 20;
        if rect.w == 0 || rect.h == 0 {
            return Rect::new(rect.x, rect.y, 0, 0);
        }
        let title_h = rect.h.min(TITLE_H);
        canvas.fill_rect(rect, theme.panel);
        let title_rect = Rect::new(rect.x, rect.y, rect.w, title_h);
        canvas.fill_rect(title_rect, theme.panel_alt);
        sf_vcenter(
            canvas,
            title,
            rect.x + 8,
            rect.y,
            title_h,
            &TextStyle::new(FontRole::UiMedium, theme.accent),
        );
        if rect.h > TITLE_H {
            canvas.hbar(rect.x, rect.y + TITLE_H as i32, rect.w, 1, theme.border);
        }
        canvas.draw_rect(rect, theme.border);
        Rect::new(
            rect.x,
            rect.y + title_h as i32,
            rect.w,
            rect.h.saturating_sub(title_h),
        )
    }

    /// A prominent navigation button: arrow glyph + label. Enabled buttons get
    /// an accent arrow and accent underline so they read as the real actions.
    fn draw_nav_button(
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        symbol: UiSymbol,
        label: &str,
        enabled: bool,
    ) {
        let bg = if enabled {
            theme.panel
        } else {
            theme.panel_alt
        };
        canvas.fill_rect(rect, bg);
        canvas.draw_rect(rect, theme.border);
        if enabled {
            canvas.hbar(rect.x, rect.bottom() - 2, rect.w, 2, theme.accent);
        }

        let glyph_color = if enabled {
            theme.accent
        } else {
            theme.text_dim
        };
        let text_color = if enabled { theme.text } else { theme.text_dim };

        let gw = Canvas::measure_ui_symbol(symbol) as i32;
        let tw = sf_measure(label, FontRole::UiMedium).w as i32;
        let gap: i32 = 5;
        let unit = gw + gap + tw;
        let start = rect.x + (rect.w as i32 - unit) / 2;
        let glyph_ty = rect.y + (rect.h as i32 - 9) / 2;
        let after = canvas.draw_ui_symbol(start, glyph_ty, symbol, glyph_color);
        sf_vcenter(
            canvas,
            label,
            after + gap,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiMedium, text_color),
        );
    }

    /// A clearly-inactive placeholder chip: flat fill, faint border, dim text.
    /// When `symbol` is provided a small UiSymbol glyph is drawn before the
    /// label so the chip reads as a decorative tool button. Used for every
    /// placeholder edit control so none read as clickable.
    fn draw_placeholder_chip(
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        symbol: Option<UiSymbol>,
        label: &str,
    ) {
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.draw_rect(rect, theme.border);
        let dim_style = TextStyle::new(FontRole::UiSmall, theme.text_dim);
        let dim_color = theme.text_dim;

        if let Some(sym) = symbol {
            let sw = Canvas::measure_ui_symbol(sym) as i32;
            let tw = sf_measure(label, FontRole::UiSmall).w as i32;
            let gap = 4i32;
            let unit = sw + gap + tw;
            let start_x = rect.x + (rect.w as i32 - unit) / 2;
            canvas.draw_ui_symbol(start_x, rect.y + (rect.h as i32 - 9) / 2, sym, dim_color);
            sf_vcenter(
                canvas,
                label,
                start_x + sw + gap,
                rect.y,
                rect.h,
                &dim_style,
            );
        } else {
            sf_centered(canvas, rect, label, &dim_style);
        }
    }

    /// A small dim section heading with a faint separator trailing to the
    /// right edge. Returns the y just below the heading.
    fn draw_section_label(
        canvas: &mut Canvas,
        theme: &Theme,
        x: i32,
        y: i32,
        w: u32,
        label: &str,
    ) -> i32 {
        let role = FontRole::UiSmall;
        sf_draw(canvas, label, x, y, &TextStyle::new(role, theme.text_dim));
        let label_w = sf_measure(label, role).w as i32;
        let lh = sf_lh(role) as i32;
        let sep_x = x + label_w + 8;
        let sep_w = (w as i32 - label_w - 8).max(0) as u32;
        canvas.hline(sep_x, y + lh / 2, sep_w, theme.border);
        y + 18
    }

    /// Compute the deterministic Fit presentation rectangle owned by the
    /// image renderer. The layout system owns `viewport`; this helper never
    /// changes it and never returns geometry for an unusable source/viewport.
    fn fit_image_rect(viewport: Rect, width: u32, height: u32) -> Option<Rect> {
        if width == 0 || height == 0 || viewport.w == 0 || viewport.h == 0 {
            return None;
        }
        let scale_w = u64::from(viewport.w).saturating_mul(u64::from(height));
        let scale_h = u64::from(viewport.h).saturating_mul(u64::from(width));
        let (fit_w, fit_h) = if scale_w <= scale_h {
            let h = u64::from(viewport.w).saturating_mul(u64::from(height)) / u64::from(width);
            (viewport.w, (h as u32).max(1).min(viewport.h))
        } else {
            let w = u64::from(viewport.h).saturating_mul(u64::from(width)) / u64::from(height);
            ((w as u32).max(1).min(viewport.w), viewport.h)
        };
        Some(Rect::new(
            viewport.x + (viewport.w.saturating_sub(fit_w) / 2) as i32,
            viewport.y + (viewport.h.saturating_sub(fit_h) / 2) as i32,
            fit_w,
            fit_h,
        ))
    }

    /// Fit-blit decoded photo pixels with **forced opaque ARGB**.
    ///
    /// Same class of fix as File Manager preview: on alpha-capable window
    /// surfaces, samples without a solid alpha byte show as a flat gray
    /// panel hole under glass/compositor blend. Photo content is always
    /// treated as opaque for display (source alpha is ignored for RGB).
    fn draw_photo(canvas: &mut Canvas, img: &RgbaImage, dst: Rect) {
        if img.width == 0 || img.height == 0 || dst.w == 0 || dst.h == 0 {
            return;
        }
        let cx0 = dst.x.max(0) as u32;
        let cy0 = dst.y.max(0) as u32;
        let cx1 = dst.right().min(canvas.width as i32).max(0) as u32;
        let cy1 = dst.bottom().min(canvas.height as i32).max(0) as u32;
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        let dw = dst.w.max(1) as u64;
        let dh = dst.h.max(1) as u64;
        let sw = img.width as u64;
        let sh = img.height as u64;
        for dy in cy0..cy1 {
            let ly = (dy as i32 - dst.y) as u64;
            let sy = ((ly * sh) / dh) as u32;
            let row_off = dy as usize * canvas.stride as usize;
            for dx in cx0..cx1 {
                let lx = (dx as i32 - dst.x) as u64;
                let sx = ((lx * sw) / dw) as u32;
                let idx = row_off + dx as usize;
                if idx < canvas.pixels.len() {
                    // Force opaque so glass surfaces never composite a gray hole.
                    canvas.pixels[idx] = 0xFF00_0000 | (img.pixel(sx, sy) & 0x00FF_FFFF);
                }
            }
        }
    }

    fn has_previous(&self) -> bool {
        self.has_current_index && self.current_index > 0
    }

    fn has_next(&self) -> bool {
        self.has_current_index && self.current_index + 1 < self.sibling_count
    }

    fn show_previous(&mut self) -> bool {
        if !self.has_previous() {
            return false;
        }
        let target = self.sibling_paths[self.current_index - 1];
        self.open_path(target);
        true
    }

    fn show_next(&mut self) -> bool {
        if !self.has_next() {
            return false;
        }
        let target = self.sibling_paths[self.current_index + 1];
        self.open_path(target);
        true
    }

    fn open_path(&mut self, path: PathBuf) {
        self.current_path = path;
        self.image = None;
        self.info = ImageInfo::empty();
        self.load_state = LoadState::Empty;
        self.message.clear();
        self.load_current_image();
        // Only refresh the sibling list if the image loaded successfully;
        // on failure keep the previous sibling state so Back/Next remain
        // usable to navigate back to the last good image.
        if self.load_state == LoadState::Ready {
            self.collect_siblings();
        } else {
            // Search the existing sibling list for the failed path so the
            // folder-position display stays accurate.
            self.has_current_index = false;
            for (idx, sib) in self
                .sibling_paths
                .iter()
                .enumerate()
                .take(self.sibling_count)
            {
                if *sib == self.current_path {
                    self.current_index = idx;
                    self.has_current_index = true;
                    break;
                }
            }
        }
    }

    fn load_current_image(&mut self) {
        if self.current_path.is_empty() {
            self.load_state = LoadState::Empty;
            self.message.set_str("Open an image from Sunlight Files.");
            return;
        }

        let stat = match libc::stat(self.current_path.as_bytes()) {
            Ok(stat) => stat,
            Err(_) => {
                self.set_error("File not found or inaccessible.");
                return;
            }
        };
        self.info.size_bytes = stat.size;
        self.info.format = format_from_name(self.current_path.file_name_bytes());

        if stat.file_type != FT_FILE {
            self.set_error("The selected path is not a file.");
            return;
        }
        if stat.size == 0 {
            self.set_error("The image file is empty.");
            return;
        }
        if stat.size as usize > MAX_IMAGE_BYTES {
            self.set_error("Image is too large for Light Lens MVP.");
            return;
        }

        let fd = match libc::open(self.current_path.as_bytes()) {
            Ok(fd) => fd,
            Err(_) => {
                self.set_error("Could not open image file.");
                return;
            }
        };

        let mut total = 0usize;
        while total < stat.size as usize {
            let chunk = ((stat.size as usize) - total).min(8192);
            let read = unsafe { libc::read(fd, &mut IMAGE_BUF[total..total + chunk]).unwrap_or(0) };
            if read == 0 {
                break;
            }
            total += read;
        }
        let _ = libc::close(fd);

        unsafe {
            IMAGE_LEN = total;
        }

        // SIMG v2 needs only 4 magic bytes to detect; legacy TGA needs 18.
        if total < 4 {
            self.set_error("Unsupported or broken image.");
            return;
        }
        if total < 18 && unsafe { IMAGE_BUF[..4] != *b"SIMG" } {
            self.set_error("Unsupported or broken image.");
            return;
        }

        match unsafe { decode_runtime_image(total) } {
            Some((image, kind)) => {
                self.info.width = image.width;
                self.info.height = image.height;
                self.info.bpp = 32;
                self.info.format = kind;
                self.image = Some(image);
                self.load_state = LoadState::Ready;
                self.message.set_str("Ready");
            }
            None => self.set_error("Unsupported or broken image."),
        }
    }

    fn collect_siblings(&mut self) {
        self.sibling_count = 0;
        self.current_index = 0;
        self.has_current_index = false;

        let Some(parent) = self.current_path.parent() else {
            self.fallback_singleton();
            return;
        };

        let mut entries = [DirEntry::zeroed(); MAX_DIR_ENTRIES];
        let Ok(count) = libc::read_dir(parent.as_bytes(), &mut entries) else {
            self.fallback_singleton();
            return;
        };

        entries[..count.min(MAX_DIR_ENTRIES)]
            .sort_by(|a, b| cmp_ascii_ci(a.name_bytes(), b.name_bytes()));
        for entry in entries.iter().take(count.min(MAX_DIR_ENTRIES)) {
            if entry.file_type != FT_FILE || !is_supported_image_name(entry.name_bytes()) {
                continue;
            }
            let Some(full_path) = parent.join(entry.name_bytes()) else {
                continue;
            };
            if self.sibling_count < MAX_SIBLINGS {
                self.sibling_paths[self.sibling_count] = full_path;
                if full_path == self.current_path {
                    self.current_index = self.sibling_count;
                    self.has_current_index = true;
                }
                self.sibling_count += 1;
            }
        }

        if self.sibling_count == 0 || !self.has_current_index {
            self.fallback_singleton();
        }
    }

    fn fallback_singleton(&mut self) {
        if self.current_path.is_empty() {
            self.sibling_count = 0;
            self.current_index = 0;
            self.has_current_index = false;
            return;
        }
        self.sibling_paths[0] = self.current_path;
        self.sibling_count = 1;
        self.current_index = 0;
        self.has_current_index = true;
    }

    fn set_error(&mut self, text: &str) {
        self.load_state = LoadState::Error;
        self.message.set_str(text);
    }

    fn draw_header(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        canvas.fill_rect(rect, theme.panel);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        if let Some(icon) = self.app_icon {
            // Material icon (monochrome) tinted to foreground.
            canvas.draw_tga_icon_tinted(
                &icon,
                Rect::new(rect.x + 8, rect.y + 6, 22, 22),
                theme.icon_foreground,
            );
        }
        sf_vcenter(
            canvas,
            "Light Lens",
            rect.x + 38,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        let mut sub_buf: TextBuf<VALUE_LEN> = TextBuf::empty();
        sub_buf.set_str(if self.current_path.is_empty() {
            "Image Viewer"
        } else {
            self.current_path.file_name_str()
        });
        let subtitle = sub_buf.truncate_to_width(rect.w as i32 - 130, FontRole::UiSmall);
        sf_right(
            canvas,
            rect,
            subtitle,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            12,
        );
    }

    /// Subtle dim "◀ / ▶ browse" hint, centered in the free toolbar middle.
    fn draw_toolbar_hint(canvas: &mut Canvas, theme: &Theme, x0: i32, x1: i32, bar: Rect) {
        let back_w = Canvas::measure_ui_symbol(UiSymbol::Back);
        let fwd_w = Canvas::measure_ui_symbol(UiSymbol::Forward);
        let mid = " / ";
        let tail = "  browse";
        let role = FontRole::UiSmall;
        let total = back_w + sf_measure(mid, role).w + fwd_w + sf_measure(tail, role).w;
        let cx = x0 + ((x1 - x0) - total as i32) / 2;
        let glyph_ty = bar.y + (bar.h as i32 - 9) / 2;
        let dim = theme.text_dim;
        let style = TextStyle::new(role, dim);
        let mut x = cx;
        x = canvas.draw_ui_symbol(x, glyph_ty, UiSymbol::Back, dim);
        x = sf_vcenter(canvas, mid, x, bar.y, bar.h, &style);
        x = canvas.draw_ui_symbol(x, glyph_ty, UiSymbol::Forward, dim);
        sf_vcenter(canvas, tail, x, bar.y, bar.h, &style);
    }

    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);

        let back_rect = self.layout.back;
        let next_rect = self.layout.next;
        let tool_rects = self.layout.tools;

        // Functional navigation (left cluster) — prominent, accent-highlighted.
        Self::draw_nav_button(
            canvas,
            theme,
            back_rect,
            UiSymbol::Back,
            "Back",
            self.has_previous(),
        );
        Self::draw_nav_button(
            canvas,
            theme,
            next_rect,
            UiSymbol::Forward,
            "Next",
            self.has_next(),
        );

        // Decorative edit tools (right cluster) — all inactive placeholders
        // with UiSymbol glyphs so they read as polished tool buttons.
        let tools: [(Option<UiSymbol>, &str); 6] = [
            (Some(UiSymbol::Search), "Zoom +"),
            (Some(UiSymbol::Minus), "Zoom -"),
            (Some(UiSymbol::Back), "Rotate L"),
            (Some(UiSymbol::Forward), "Rotate R"),
            (Some(UiSymbol::Divide), "Crop"),
            (Some(UiSymbol::Multiply), "Flip H"),
        ];
        for (idx, (sym, label)) in tools.iter().enumerate() {
            Self::draw_placeholder_chip(canvas, theme, tool_rects[idx], *sym, label);
        }

        // Subtle keyboard hint in the free middle area, if there is room.
        let hint_x0 = next_rect.right() + 14;
        let hint_x1 = tool_rects[0].x.saturating_sub(14);
        if hint_x1 > hint_x0 + 40 {
            Self::draw_toolbar_hint(canvas, theme, hint_x0, hint_x1, rect);
        }
    }

    fn draw_left_panel(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let content = Self::draw_panel(canvas, theme, rect, "Edit Tools");
        if content.w == 0 || content.h == 0 {
            return;
        }
        let pad = 12i32;
        let inner_w = (content.w as i32 - pad * 2).max(0) as u32;
        let chip_h = 26u32;
        let chip_gap = 6i32;
        let dim = TextStyle::new(FontRole::UiSmall, theme.text_dim);

        // "Coming soon" caption — makes the placeholder intent unmistakable.
        let mut y = content.y + 12;
        canvas.fill_rect(Rect::new(content.x + pad, y + 2, 6, 6), theme.warn);
        sf_draw(canvas, "Coming soon", content.x + pad + 11, y, &dim);
        y += 24;

        // Section: Modes
        y = Self::draw_section_label(canvas, theme, content.x + pad, y, inner_w, "Modes");
        for label in ["Edit Mode", "AI Enhance", "Reset All"] {
            Self::draw_placeholder_chip(
                canvas,
                theme,
                Rect::new(content.x + pad, y, inner_w, chip_h),
                None,
                label,
            );
            y += chip_h as i32 + chip_gap;
        }

        // Section: Adjust — muted sliders, knob centered = no change applied.
        y += 4;
        y = Self::draw_section_label(canvas, theme, content.x + pad, y, inner_w, "Adjust");
        for label in ["Brightness", "Contrast", "Saturation"] {
            sf_draw(canvas, label, content.x + pad, y, &dim);
            let track = Rect::new(content.x + pad, y + 14, inner_w, 6);
            canvas.fill_rect(track, theme.panel_alt);
            canvas.draw_rect(track, theme.border);
            let knob = Rect::new(track.x + track.w as i32 / 2 - 5, track.y - 3, 10, 12);
            canvas.fill_rect(knob, theme.text_dim);
            y += 32;
        }

        // Section: Filters — two columns of inactive chips.
        y += 4;
        y = Self::draw_section_label(canvas, theme, content.x + pad, y, inner_w, "Filters");
        let col_gap = 8i32;
        let col_w = ((inner_w as i32 - col_gap) / 2).max(0) as u32;
        for (idx, label) in ["Filters", "Effects", "Crop", "Flip H"]
            .into_iter()
            .enumerate()
        {
            let col = (idx % 2) as i32;
            let row = (idx / 2) as i32;
            let cr = Rect::new(
                content.x + pad + col * (col_w as i32 + col_gap),
                y + row * (chip_h as i32 + chip_gap),
                col_w,
                chip_h,
            );
            Self::draw_placeholder_chip(canvas, theme, cr, None, label);
        }
        y += 2 * (chip_h as i32 + chip_gap);

        // Footer note.
        y += 10;
        sf_draw(canvas, "Editing arrives", content.x + pad, y, &dim);
        sf_draw(canvas, "in a later release.", content.x + pad, y + 14, &dim);
    }

    /// Subtle two-tone checkerboard — makes the preview read as an image
    /// surface so empty/letterboxed areas feel intentional rather than blank.
    fn draw_checkerboard(canvas: &mut Canvas, rect: Rect, base: Color, alt: Color, cell: i32) {
        let mut y = rect.y;
        let mut row = 0u32;
        while y < rect.bottom() {
            let h = cell.min(rect.bottom() - y) as u32;
            let mut x = rect.x;
            let mut col = 0u32;
            while x < rect.right() {
                let w = cell.min(rect.right() - x) as u32;
                let color = if (row + col) % 2 == 0 { base } else { alt };
                canvas.fill_rect(Rect::new(x, y, w, h), color);
                x += cell;
                col += 1;
            }
            y += cell;
            row += 1;
        }
    }

    fn draw_preview_panel(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        let content = Self::draw_panel(canvas, theme, rect, "Preview").inset(8);
        if content.w == 0 || content.h == 0 {
            return;
        }

        Self::draw_checkerboard(canvas, content, theme.bg, theme.bg.lighten(7), 18);
        canvas.draw_rect(content, theme.border);

        match (self.load_state, self.image.as_ref()) {
            (LoadState::Ready, Some(image)) => {
                // All photo operations use viewport-local coordinates. The
                // shared Canvas clip therefore makes it impossible for scaled
                // pixels to reach the toolbar, side panels, or status bar.
                let mut viewport_canvas = canvas.sub_canvas(content);
                let local_viewport = Rect::new(0, 0, content.w, content.h).inset(16);
                if let Some(image_rect) =
                    Self::fit_image_rect(local_viewport, image.width, image.height)
                {
                    viewport_canvas.fill_rect(
                        image_rect,
                        Color::rgb(theme.bg.r(), theme.bg.g(), theme.bg.b()),
                    );
                    Self::draw_photo(&mut viewport_canvas, image, image_rect);
                    viewport_canvas.draw_rect(image_rect, theme.border);
                    viewport_canvas.draw_rect(image_rect.inset(-1), theme.border);
                }
            }
            (LoadState::Error, _) => {
                if let Some(icon) = self.missing_icon {
                    let icon_rect = Rect::new(
                        content.x + (content.w as i32 - 64) / 2,
                        content.y + (content.h as i32 - 64) / 2 - 24,
                        64,
                        64,
                    );
                    canvas.draw_tga_icon_tinted(&icon, icon_rect, theme.icon_muted);
                }
                let msg_buf_px = sf_measure(self.message.as_str(), FontRole::UiRegular).w as i32;
                let msg_rect = Rect::new(
                    content.x + (content.w as i32 - msg_buf_px) / 2 - 16,
                    content.y + (content.h as i32 - 64) / 2 + 50,
                    (msg_buf_px + 32).max(40) as u32,
                    28,
                );
                sf_centered(
                    canvas,
                    msg_rect,
                    self.message.as_str(),
                    &TextStyle::new(FontRole::UiRegular, theme.danger),
                );
            }
            _ => {
                if let Some(icon) = self.app_icon {
                    let icon_rect = Rect::new(
                        content.x + (content.w as i32 - 72) / 2,
                        content.y + (content.h as i32 - 72) / 2 - 20,
                        72,
                        72,
                    );
                    canvas.draw_tga_icon_tinted(&icon, icon_rect, theme.icon_muted);
                }
                let msg_rect = Rect::new(
                    content.x + 20,
                    content.bottom() - 72,
                    content.w.saturating_sub(40),
                    24,
                );
                sf_centered(
                    canvas,
                    msg_rect,
                    self.message.as_str(),
                    &TextStyle::new(FontRole::UiRegular, theme.text_dim),
                );
            }
        }
    }

    fn draw_info_row(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        label: &str,
        value: &str,
    ) {
        sf_vcenter(
            canvas,
            label,
            rect.x,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        let val_rect = Rect::new(
            rect.x + INFO_LABEL_W,
            rect.y,
            (rect.w as i32 - INFO_LABEL_W).max(0) as u32,
            rect.h,
        );
        sf_right(
            canvas,
            val_rect,
            value,
            &TextStyle::new(FontRole::UiRegular, theme.text),
            0,
        );
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
    }

    fn draw_info_panel(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let content = Self::draw_panel(canvas, theme, rect, "Image Info").inset(12);
        if content.w == 0 || content.h == 0 {
            return;
        }

        const ROWS: usize = 8;
        const ROW_H: i32 = 32;

        for idx in 0..ROWS {
            let mut src = [0u8; VALUE_LEN];
            let mut dst: TextBuf<VALUE_LEN> = TextBuf::empty();
            let row = Rect::new(
                content.x,
                content.y + idx as i32 * ROW_H,
                content.w,
                ROW_H as u32,
            );
            let (label, value) = match idx {
                0 => ("Filename", self.current_path.file_name_str()),
                1 => ("Format", format_label(self.info.format)),
                2 => ("Resolution", fill_resolution(&self.info, &mut src)),
                3 => ("File Size", fill_size(self.info.size_bytes, &mut src)),
                4 => ("Zoom", "Fit"),
                5 => ("Color Depth", fill_bpp(self.info.bpp, &mut src)),
                6 => ("Modified", "Unknown"),
                _ => (
                    "Folder",
                    fill_folder_position(self.current_index, self.sibling_count, &mut src),
                ),
            };
            dst.set_str(value);
            let max_w = content.w as i32 - INFO_LABEL_W - 8;
            let shown = dst.truncate_to_width(max_w, FontRole::UiRegular);
            self.draw_info_row(canvas, theme, row, label, shown);
        }

        // Footer: parent folder location, truncated to fit.
        if !self.current_path.is_empty() {
            let footer_y = content.y + ROWS as i32 * ROW_H + 10;
            canvas.hbar(content.x, footer_y, content.w, 1, theme.border);
            sf_draw(
                canvas,
                "Location",
                content.x,
                footer_y + 10,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            let mut path_buf: TextBuf<VALUE_LEN> = TextBuf::empty();
            match self.current_path.parent() {
                Some(p) => path_buf.set_bytes(p.as_bytes()),
                None => path_buf.set_str("/"),
            }
            let shown = path_buf.truncate_to_width(content.w as i32, FontRole::UiSmall);
            sf_draw(
                canvas,
                shown,
                content.x,
                footer_y + 24,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
    }

    fn draw_status(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        let mut center_buf = [0u8; 48];
        let mut right_buf = [0u8; 24];
        let mut left_buf: TextBuf<VALUE_LEN> = TextBuf::empty();

        let raw = if self.current_path.is_empty() {
            "No image"
        } else if self.load_state == LoadState::Error {
            self.message.as_str()
        } else {
            self.current_path.file_name_str()
        };
        left_buf.set_str(raw);
        // Keep the filename within roughly the left third of the bar.
        let left = left_buf.truncate_to_width((rect.w as i32 / 3) - 16, FontRole::UiSmall);

        let center = fill_status_center(
            &self.info,
            self.current_index,
            self.sibling_count,
            &mut center_buf,
        );
        let right = fill_status_right(self.load_state, &mut right_buf);

        // Render the status bar with the MiniType font. The StatusBar widget
        // draws its own text with the bitmap font, so its three sections are
        // drawn directly here to match the rest of the UI.
        let h = StatusBar::HEIGHT;
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.hbar(rect.x, rect.y, rect.w, 1, theme.border);
        sf_vcenter(
            canvas,
            left,
            rect.x + 8,
            rect.y,
            h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        let cw = sf_measure(center, FontRole::UiRegular).w as i32;
        let cx = rect.x + (rect.w as i32 - cw) / 2;
        sf_vcenter(
            canvas,
            center,
            cx,
            rect.y,
            h,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        sf_right(
            canvas,
            rect,
            right,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            8,
        );
    }
}

impl App for LightLensApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        if self.client_bounds.size() != Size::new(canvas.width, canvas.height) {
            let _ = self.set_client_bounds(canvas.width, canvas.height);
        } else {
            let _ = self.ensure_layout();
        }
        let layout = self.layout;
        canvas.fill_rect(layout.root, theme.bg);
        self.draw_header(canvas, theme, layout.header);
        self.draw_toolbar(canvas, theme, layout.toolbar);
        self.draw_left_panel(canvas, theme, layout.left);
        self.draw_preview_panel(canvas, theme, layout.viewport);
        self.draw_info_panel(canvas, theme, layout.right);
        self.draw_status(canvas, theme, layout.status);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                if self.layout.back.contains(Point::new(x, y)) {
                    return self.show_previous();
                }
                if self.layout.next.contains(Point::new(x, y)) {
                    return self.show_next();
                }
                false
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ctrl,
                ..
            } => {
                if keycode == KEY_ESC || (ctrl && keycode == KEY_Q) {
                    request_close();
                    return true;
                }
                match keycode {
                    KEY_LEFT => self.show_previous(),
                    KEY_RIGHT => self.show_next(),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.set_client_bounds(width, height)
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[LIGHT-LENS] panic\n");
    loop {
        process_yield();
    }
}

#[no_mangle]
#[cfg(not(test))]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=light-lens",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let initial_path = parse_user_path_arg(argc, argv);
    let mut app = LightLensApp::new(initial_path);
    let mut window = match Window::connect_with_material(
        WindowConfig {
            width: WIN_W,
            height: WIN_H,
            title: "Light Lens",
            decoration: WindowDecoration::Normal,
        },
        // Match Files / Control Panel so photo pixels use the same
        // alpha-capable surface path (opaque ARGB samples).
        WindowMaterial::WindowGlass,
    ) {
        Some(window) => window,
        None => {
            debug_log("[LIGHT-LENS] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}

fn parse_user_path_arg(argc: u64, argv: *const *const u8) -> Option<PathBuf> {
    let mut raw = [core::ptr::null::<u8>(); libc::MAX_ARGS];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut raw) };
    for ptr in raw.iter().take(count).skip(1) {
        let len = unsafe { crt0::cstr_len(*ptr, MAX_PATH) };
        if len == 0 {
            continue;
        }
        let bytes = unsafe { core::slice::from_raw_parts(*ptr, len) };
        if is_ignored_launch_arg(bytes) {
            continue;
        }
        return PathBuf::from_bytes(bytes);
    }
    None
}

fn is_ignored_launch_arg(arg: &[u8]) -> bool {
    arg.is_empty()
        || arg.starts_with(b"--sunlight-")
        || find_bytes(arg, b"--sunlight-launch").is_some()
        || arg[0] == b'?'
        || arg[0] == b'-'
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Decode runtime image bytes from `IMAGE_BUF` into an owned ARGB buffer.
///
/// Uses the shared `decode_simg` path (SIMG v2 strict, else TGA type-2) so
/// Light Lens matches File Manager preview decode. Drawing forces opaque
/// ARGB (see [`LightLensApp::draw_photo`]) to avoid the gray alpha-hole that
/// previously affected the Files preview pane on glass surfaces.
unsafe fn decode_runtime_image(total: usize) -> Option<(RgbaImage, FormatKind)> {
    if total < 4 {
        return None;
    }
    let kind = if IMAGE_BUF[..4] == *b"SIMG" {
        FormatKind::SimgV2
    } else {
        FormatKind::Tga
    };
    let src = &IMAGE_BUF[..total];
    let decoded = decode_simg(src).ok()?;
    if decoded.width == 0 || decoded.height == 0 {
        return None;
    }
    // Drop the source file bytes from the static buffer after a successful
    // decode — the owned RgbaImage is the display source of truth.
    IMAGE_LEN = 0;
    Some((decoded, kind))
}

fn format_from_name(name: &[u8]) -> FormatKind {
    if ends_with_ignore_ascii_case(name, b".simg") {
        // Content may still be SIMG v2; refined after parse.
        FormatKind::SimgLegacy
    } else if ends_with_ignore_ascii_case(name, b".tga") {
        FormatKind::Tga
    } else {
        FormatKind::Unknown
    }
}

fn format_label(kind: FormatKind) -> &'static str {
    match kind {
        FormatKind::SimgLegacy => "SIMG (legacy TGA)",
        FormatKind::SimgV2 => "SIMG v2",
        FormatKind::Tga => "TGA",
        FormatKind::Unknown => "Unknown",
    }
}

fn is_supported_image_name(name: &[u8]) -> bool {
    ends_with_ignore_ascii_case(name, b".simg") || ends_with_ignore_ascii_case(name, b".tga")
}

fn ends_with_ignore_ascii_case(name: &[u8], suffix: &[u8]) -> bool {
    if name.len() < suffix.len() {
        return false;
    }
    let start = name.len() - suffix.len();
    for idx in 0..suffix.len() {
        if name[start + idx].to_ascii_lowercase() != suffix[idx].to_ascii_lowercase() {
            return false;
        }
    }
    true
}

fn cmp_ascii_ci(a: &[u8], b: &[u8]) -> Ordering {
    let len = a.len().min(b.len());
    for idx in 0..len {
        let la = a[idx].to_ascii_lowercase();
        let lb = b[idx].to_ascii_lowercase();
        match la.cmp(&lb) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }
    a.len().cmp(&b.len())
}

fn write_bytes(out: &mut [u8], src: &[u8]) -> usize {
    let len = src.len().min(out.len());
    out[..len].copy_from_slice(&src[..len]);
    len
}

fn write_u32(out: &mut [u8], mut value: u32) -> usize {
    let mut buf = [0u8; 16];
    let mut idx = buf.len();
    if value == 0 {
        return write_bytes(out, b"0");
    }
    while value != 0 {
        idx -= 1;
        buf[idx] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    write_bytes(out, &buf[idx..])
}

fn write_u64(out: &mut [u8], mut value: u64) -> usize {
    let mut buf = [0u8; 24];
    let mut idx = buf.len();
    if value == 0 {
        return write_bytes(out, b"0");
    }
    while value != 0 {
        idx -= 1;
        buf[idx] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    write_bytes(out, &buf[idx..])
}

fn fill_resolution<'a>(info: &ImageInfo, out: &'a mut [u8; VALUE_LEN]) -> &'a str {
    if info.width == 0 || info.height == 0 {
        return "Unknown";
    }
    let mut len = write_u32(out, info.width);
    len += write_bytes(&mut out[len..], b"x");
    len += write_u32(&mut out[len..], info.height);
    core::str::from_utf8(&out[..len]).unwrap_or("Unknown")
}

fn fill_bpp<'a>(bpp: u8, out: &'a mut [u8; VALUE_LEN]) -> &'a str {
    if bpp == 0 {
        return "Unknown";
    }
    let mut len = write_u32(out, bpp as u32);
    len += write_bytes(&mut out[len..], b"-bit");
    core::str::from_utf8(&out[..len]).unwrap_or("Unknown")
}

fn fill_size<'a>(size: u64, out: &'a mut [u8; VALUE_LEN]) -> &'a str {
    if size >= 1024 * 1024 {
        let whole = size / (1024 * 1024);
        let frac = ((size % (1024 * 1024)) * 10) / (1024 * 1024);
        let mut len = write_u64(out, whole);
        len += write_bytes(&mut out[len..], b".");
        len += write_u64(&mut out[len..], frac);
        len += write_bytes(&mut out[len..], b" MB");
        core::str::from_utf8(&out[..len]).unwrap_or("Unknown")
    } else if size >= 1024 {
        let whole = size / 1024;
        let frac = ((size % 1024) * 10) / 1024;
        let mut len = write_u64(out, whole);
        len += write_bytes(&mut out[len..], b".");
        len += write_u64(&mut out[len..], frac);
        len += write_bytes(&mut out[len..], b" KB");
        core::str::from_utf8(&out[..len]).unwrap_or("Unknown")
    } else {
        let mut len = write_u64(out, size);
        len += write_bytes(&mut out[len..], b" B");
        core::str::from_utf8(&out[..len]).unwrap_or("Unknown")
    }
}

fn fill_folder_position<'a>(index: usize, count: usize, out: &'a mut [u8; VALUE_LEN]) -> &'a str {
    if count == 0 {
        return "1 / 1";
    }
    let mut len = write_u32(out, (index + 1) as u32);
    len += write_bytes(&mut out[len..], b" / ");
    len += write_u32(&mut out[len..], count as u32);
    core::str::from_utf8(&out[..len]).unwrap_or("1 / 1")
}

fn fill_status_center<'a>(
    info: &ImageInfo,
    index: usize,
    count: usize,
    out: &'a mut [u8; 48],
) -> &'a str {
    let mut len = 0usize;
    if info.width != 0 && info.height != 0 {
        len += write_u32(&mut out[len..], info.width);
        len += write_bytes(&mut out[len..], b"x");
        len += write_u32(&mut out[len..], info.height);
        len += write_bytes(&mut out[len..], b" | Fit");
    } else {
        len += write_bytes(&mut out[len..], b"Fit");
    }
    if count != 0 {
        len += write_bytes(&mut out[len..], b" | ");
        len += write_u32(&mut out[len..], (index + 1) as u32);
        len += write_bytes(&mut out[len..], b"/");
        len += write_u32(&mut out[len..], count as u32);
    }
    core::str::from_utf8(&out[..len]).unwrap_or("Fit")
}

fn fill_status_right<'a>(state: LoadState, out: &'a mut [u8; 24]) -> &'a str {
    let text = match state {
        LoadState::Ready => b"Ready".as_slice(),
        LoadState::Error => b"Load Error".as_slice(),
        LoadState::Empty => b"Waiting".as_slice(),
    };
    let len = write_bytes(out, text);
    core::str::from_utf8(&out[..len]).unwrap_or("Waiting")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn fit(viewport: Rect, width: u32, height: u32) -> Rect {
        LightLensApp::fit_image_rect(viewport, width, height).expect("valid fit")
    }

    #[test]
    fn responsive_layout_assigns_toolbar_and_remaining_viewport() {
        let root = Rect::new(0, 0, 1220, 760);
        let layout = LightLensApp::compute_layout(root);
        assert_eq!(layout.root, root);
        assert_eq!(layout.toolbar.w, root.w);
        assert_eq!(
            layout.viewport.w,
            1220 - 2 * OUTER_PAD as u32 - LEFT_W - RIGHT_W - 20
        );
        assert_eq!(layout.viewport.h, layout.body.h.saturating_sub(20));
        assert_eq!(layout.left.h, layout.viewport.h);
        assert_eq!(layout.right.h, layout.viewport.h);
        assert_eq!(layout.status.bottom(), root.bottom());
    }

    #[test]
    fn resizing_recomputes_toolbar_and_viewport_without_stale_geometry() {
        let initial = LightLensApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H));
        let wider = LightLensApp::compute_layout(Rect::new(0, 0, WIN_W + 200, WIN_H));
        let taller = LightLensApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H + 160));
        let smaller = LightLensApp::compute_layout(Rect::new(0, 0, 900, 520));
        assert_eq!(wider.toolbar.w, initial.toolbar.w + 200);
        assert_eq!(wider.viewport.w, initial.viewport.w + 200);
        assert_eq!(taller.viewport.h, initial.viewport.h + 160);
        assert!(smaller.viewport.w < initial.viewport.w);
        assert!(smaller.viewport.h < initial.viewport.h);
    }

    #[test]
    fn repeated_and_grow_shrink_grow_layouts_are_stable() {
        let large_bounds = Rect::new(0, 0, 1440, 900);
        let first = LightLensApp::compute_layout(large_bounds);
        let _small = LightLensApp::compute_layout(Rect::new(0, 0, 760, 420));
        let restored = LightLensApp::compute_layout(large_bounds);
        assert_eq!(first, restored);
        assert_eq!(first, LightLensApp::compute_layout(large_bounds));
    }

    #[test]
    fn tiny_and_zero_client_dimensions_are_safe() {
        for bounds in [
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 1),
            Rect::new(0, 0, 400, TOOLBAR_H),
        ] {
            let layout = LightLensApp::compute_layout(bounds);
            assert_eq!(layout.root, bounds);
            assert_eq!(layout.viewport.w, 0);
            assert_eq!(layout.viewport.h, 0);
        }
    }

    #[test]
    fn landscape_fits_inside_portrait_viewport_and_centers_vertically() {
        let viewport = Rect::new(10, 20, 300, 600);
        let image = fit(viewport, 1600, 900);
        assert_eq!(image, Rect::new(10, 236, 300, 168));
        assert!(image.y > viewport.y);
        assert!(image.bottom() <= viewport.bottom());
    }

    #[test]
    fn portrait_fits_inside_landscape_viewport_and_centers_horizontally() {
        let viewport = Rect::new(10, 20, 600, 300);
        let image = fit(viewport, 900, 1600);
        assert_eq!(image, Rect::new(226, 20, 168, 300));
        assert!(image.x > viewport.x);
        assert!(image.right() <= viewport.right());
    }

    #[test]
    fn square_and_exact_ratio_fits_preserve_aspect_and_center() {
        let square = fit(Rect::new(0, 0, 300, 200), 100, 100);
        assert_eq!(square, Rect::new(50, 0, 200, 200));

        let exact = fit(Rect::new(7, 11, 1000, 600), 4, 3);
        assert_eq!(exact, Rect::new(107, 11, 800, 600));
        assert_eq!(u64::from(exact.w) * 3, u64::from(exact.h) * 4);
    }

    #[test]
    fn fitted_image_never_exceeds_viewport() {
        for (viewport, width, height) in [
            (Rect::new(3, 5, 1, 1), 1, 1),
            (Rect::new(3, 5, 37, 91), 4000, 3),
            (Rect::new(3, 5, 91, 37), 3, 4000),
            (Rect::new(3, 5, 640, 480), 1920, 1080),
        ] {
            let image = fit(viewport, width, height);
            assert!(image.x >= viewport.x && image.y >= viewport.y);
            assert!(image.right() <= viewport.right());
            assert!(image.bottom() <= viewport.bottom());
        }
    }

    #[test]
    fn zero_source_or_viewport_produces_no_image_geometry() {
        let viewport = Rect::new(0, 0, 100, 100);
        assert_eq!(LightLensApp::fit_image_rect(viewport, 0, 10), None);
        assert_eq!(LightLensApp::fit_image_rect(viewport, 10, 0), None);
        assert_eq!(
            LightLensApp::fit_image_rect(Rect::new(0, 0, 0, 100), 10, 10),
            None
        );
        assert_eq!(
            LightLensApp::fit_image_rect(Rect::new(0, 0, 100, 0), 10, 10),
            None
        );
    }

    #[test]
    fn fit_is_deterministic_and_large_dimensions_do_not_overflow() {
        let viewport = Rect::new(17, 29, 4096, 2160);
        let first = fit(viewport, u32::MAX, u32::MAX - 1);
        let second = fit(viewport, u32::MAX, u32::MAX - 1);
        assert_eq!(first, second);
        assert!(first.w <= viewport.w && first.h <= viewport.h);
        assert!(first.right() <= viewport.right());
        assert!(first.bottom() <= viewport.bottom());
    }

    #[test]
    fn viewport_clip_prevents_photo_pixels_from_reaching_toolbar() {
        const TOOLBAR: u32 = 5;
        const WIDTH: u32 = 20;
        const HEIGHT: u32 = 20;
        let toolbar_pixel = 0xFF11_2233;
        let photo_pixel = 0xFFAA_BBCC;
        let mut pixels = vec![toolbar_pixel; (WIDTH * HEIGHT) as usize];
        let image = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![photo_pixel],
        };
        let mut canvas = Canvas::new(&mut pixels, WIDTH, WIDTH, HEIGHT);
        {
            let mut viewport =
                canvas.sub_canvas(Rect::new(0, TOOLBAR as i32, WIDTH, HEIGHT - TOOLBAR));
            LightLensApp::draw_photo(&mut viewport, &image, Rect::new(-5, -5, 30, 30));
        }
        assert!(pixels[..(WIDTH * TOOLBAR) as usize]
            .iter()
            .all(|pixel| *pixel == toolbar_pixel));
        assert!(pixels[(WIDTH * TOOLBAR) as usize..]
            .iter()
            .all(|pixel| *pixel == photo_pixel));
    }
}
