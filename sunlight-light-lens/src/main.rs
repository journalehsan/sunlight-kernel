#![no_std]
#![no_main]

use core::alloc::{GlobalAlloc, Layout};
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
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::StatusBar;
use sunlight_ui::{
    request_close, App, Canvas, Color, Event, Point, Rect, Theme, UiSymbol, Window, WindowConfig,
    WindowDecoration,
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

static APP_ICON_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/apps/48/accessories-image-viewer.tga");
static MISSING_ICON_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/status/64/image-missing.tga");

static mut IMAGE_BUF: [u8; MAX_IMAGE_BYTES] = [0u8; MAX_IMAGE_BYTES];
static mut IMAGE_LEN: usize = 0;

struct NoAlloc;

unsafe impl GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _: Layout) -> *mut u8 {
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

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
    Simg,
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
    image: Option<TgaImage>,
    info: ImageInfo,
    load_state: LoadState,
    message: TextBuf<MSG_LEN>,
    app_icon: Option<TgaImage>,
    missing_icon: Option<TgaImage>,
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
        };
        if let Some(path) = initial_path {
            app.open_path(path);
        } else {
            app.message.set_str("Open an image from Sunlight Files.");
        }
        app
    }

    fn root_layout() -> (Rect, Rect, Rect, Rect, Rect, Rect) {
        let header = Rect::new(0, 0, WIN_W, HEADER_H);
        let toolbar = Rect::new(0, HEADER_H as i32, WIN_W, TOOLBAR_H);
        let status = Rect::new(0, WIN_H as i32 - STATUS_H as i32, WIN_W, STATUS_H);
        let body_y = toolbar.bottom() + GAP;
        let body_h = status.y - GAP - body_y;
        let left = Rect::new(OUTER_PAD, body_y, LEFT_W, body_h.max(0) as u32);
        let right = Rect::new(
            WIN_W as i32 - OUTER_PAD - RIGHT_W as i32,
            body_y,
            RIGHT_W,
            body_h.max(0) as u32,
        );
        let center_x = left.right() + GAP;
        let center_w = (right.x - GAP - center_x).max(0) as u32;
        let center = Rect::new(center_x, body_y, center_w, body_h.max(0) as u32);
        (header, toolbar, left, center, right, status)
    }

    /// Layout the toolbar: a left cluster of navigation buttons (Back, Next)
    /// and a right cluster of compact decorative tool chips. The wide middle
    /// stays free for a subtle keyboard hint.
    fn toolbar_buttons(toolbar: Rect) -> (Rect, Rect, [Rect; 6]) {
        let btn_h = toolbar.h.saturating_sub(16);
        let pad = 10i32;

        // Navigation cluster (left): Back then Next.
        let nav_w = 84u32;
        let nav_gap = 6i32;
        let back = Rect::new(toolbar.x + pad, toolbar.y + 8, nav_w, btn_h);
        let next = Rect::new(back.right() + nav_gap, toolbar.y + 8, nav_w, btn_h);

        // Decorative tool cluster (right): six compact placeholder chips.
        let tool_w = [54u32, 54, 62, 62, 48, 56];
        let tool_gap = 6u32;
        let total_w = tool_w.iter().copied().sum::<u32>() + (tool_w.len() as u32 - 1) * tool_gap;
        let mut x = toolbar.right() - pad - total_w as i32;
        let mut tools = [Rect::new(0, 0, 0, 0); 6];
        for (idx, &width) in tool_w.iter().enumerate() {
            tools[idx] = Rect::new(x, toolbar.y + 8, width, btn_h);
            x += width as i32 + tool_gap as i32;
        }
        (back, next, tools)
    }

    /// Draw a titled panel using the MiniType vector font, matching the look
    /// of `Panel::with_title` but without the bitmap title that widget draws.
    /// Returns the content rect (everything below the title bar).
    fn draw_panel(canvas: &mut Canvas, theme: &Theme, rect: Rect, title: &str) -> Rect {
        const TITLE_H: u32 = 20;
        canvas.fill_rect(rect, theme.panel);
        let title_rect = Rect::new(rect.x, rect.y, rect.w, TITLE_H);
        canvas.fill_rect(title_rect, theme.panel_alt);
        sf_vcenter(
            canvas,
            title,
            rect.x + 8,
            rect.y,
            TITLE_H,
            &TextStyle::new(FontRole::UiMedium, theme.accent),
        );
        canvas.hbar(rect.x, rect.y + TITLE_H as i32, rect.w, 1, theme.border);
        canvas.draw_rect(rect, theme.border);
        Rect::new(
            rect.x,
            rect.y + TITLE_H as i32,
            rect.w,
            rect.h.saturating_sub(TITLE_H),
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

    fn preview_fit_rect(viewport: Rect, img: TgaImage) -> Rect {
        let area = viewport.inset(16);
        if img.width == 0 || img.height == 0 || area.w == 0 || area.h == 0 {
            return area;
        }
        let scale_w = area.w as u64 * img.height as u64;
        let scale_h = area.h as u64 * img.width as u64;
        let (fit_w, fit_h) = if scale_w <= scale_h {
            let h = (area.w as u64 * img.height as u64 / img.width as u64) as u32;
            (area.w.max(1), h.max(1))
        } else {
            let w = (area.h as u64 * img.width as u64 / img.height as u64) as u32;
            (w.max(1), area.h.max(1))
        };
        Rect::new(
            area.x + (area.w as i32 - fit_w as i32) / 2,
            area.y + (area.h as i32 - fit_h as i32) / 2,
            fit_w,
            fit_h,
        )
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

        if total < 18 {
            self.set_error("Unsupported or broken image.");
            return;
        }

        let parsed = unsafe { parse_runtime_tga(total) };
        match parsed {
            Some((image, width, height, bpp)) => {
                self.image = Some(image);
                self.info.width = width;
                self.info.height = height;
                self.info.bpp = bpp;
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
        canvas.fill_rect(rect, theme.panel);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        if let Some(icon) = self.app_icon {
            canvas.draw_tga_icon(&icon, Rect::new(rect.x + 8, rect.y + 6, 22, 22));
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
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);

        let (back_rect, next_rect, tool_rects) = Self::toolbar_buttons(rect);

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
        let content = Self::draw_panel(canvas, theme, rect, "Preview").inset(8);

        Self::draw_checkerboard(canvas, content, theme.bg, theme.bg.lighten(7), 18);
        canvas.draw_rect(content, theme.border);

        match (self.load_state, self.image) {
            (LoadState::Ready, Some(image)) => {
                let fit = Self::preview_fit_rect(content, image);
                canvas.fill_rect(fit, theme.bg);
                canvas.draw_tga_icon(&image, fit);
                canvas.draw_rect(fit, theme.border);
                let outer = fit.inset(-1);
                canvas.draw_rect(outer, theme.border);
            }
            (LoadState::Error, _) => {
                if let Some(icon) = self.missing_icon {
                    let icon_rect = Rect::new(
                        content.x + (content.w as i32 - 64) / 2,
                        content.y + (content.h as i32 - 64) / 2 - 24,
                        64,
                        64,
                    );
                    canvas.draw_tga_icon(&icon, icon_rect);
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
                    canvas.draw_tga_icon(&icon, icon_rect);
                }
                let msg_rect = Rect::new(content.x + 20, content.bottom() - 72, content.w - 40, 24);
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
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        let (header, toolbar, left, center, right, status) = Self::root_layout();
        self.draw_header(canvas, theme, header);
        self.draw_toolbar(canvas, theme, toolbar);
        self.draw_left_panel(canvas, theme, left);
        self.draw_preview_panel(canvas, theme, center);
        self.draw_info_panel(canvas, theme, right);
        self.draw_status(canvas, theme, status);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                let (_header, toolbar, _left, _center, _right, _status) = Self::root_layout();
                let (back_rect, next_rect, _tool_rects) = Self::toolbar_buttons(toolbar);
                if back_rect.contains(Point::new(x, y)) {
                    return self.show_previous();
                }
                if next_rect.contains(Point::new(x, y)) {
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
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[LIGHT-LENS] panic\n");
    loop {
        process_yield();
    }
}

#[no_mangle]
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
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Light Lens",
        decoration: WindowDecoration::Normal,
    }) {
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

unsafe fn parse_runtime_tga(total: usize) -> Option<(TgaImage, u32, u32, u8)> {
    if total < 18 {
        return None;
    }
    if IMAGE_BUF[2] != 2 {
        return None;
    }
    let bpp = IMAGE_BUF[16];
    if bpp != 24 && bpp != 32 {
        return None;
    }
    let width = u16::from_le_bytes([IMAGE_BUF[12], IMAGE_BUF[13]]) as u32;
    let height = u16::from_le_bytes([IMAGE_BUF[14], IMAGE_BUF[15]]) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let bytes: &'static [u8] = &IMAGE_BUF[..total];
    let image = TgaImage::parse(bytes).ok()?;
    Some((image, width, height, bpp))
}

fn format_from_name(name: &[u8]) -> FormatKind {
    if ends_with_ignore_ascii_case(name, b".simg") {
        FormatKind::Simg
    } else if ends_with_ignore_ascii_case(name, b".tga") {
        FormatKind::Tga
    } else {
        FormatKind::Unknown
    }
}

fn format_label(kind: FormatKind) -> &'static str {
    match kind {
        FormatKind::Simg => "SIMG",
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
