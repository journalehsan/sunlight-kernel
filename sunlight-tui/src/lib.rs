//! SunlightOS graphical boot TUI
//!
//! Pure Rust, no_std, no heap, no floats.
//! Renders directly to Limine framebuffer.

#![no_std]
#![allow(dead_code)]

mod draw;
pub mod fmt;
pub mod font;
pub mod fontatlas;
pub mod framebuffer;
pub mod interaction;
pub mod layout;
mod modes;
mod splash;
pub mod tga;

pub use layout::ANSI_COLORS;
pub use modes::debug::LogBuffer;
pub use splash::{BootMode, SplashScreen};

// Framebuffer login screen icons (32×32 TGA, transparent-background).
// Now generated at build time from Material Icons TTF (see sunlight-tui/build.rs).
// We use the tinted drawer so the icons take the current accent/dim color.
const ICON_USERS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_users.tga"));
const ICON_LUGGAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_luggage.tga"));
const ICON_REBOOT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_reboot.tga"));
const ICON_SHUTDOWN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_shutdown.tga"));

/// A terminal cell with character and pre-resolved RGB colors.
#[derive(Clone, Copy, Debug)]
pub struct TermCell {
    pub ch: u8,
    pub fg: u32, // RGB color
    pub bg: u32, // RGB color
}

/// Display info for a single tab in the tab bar.
///
/// `name`/`name_len` hold the title text (e.g. "SHELL" or "TOP"). `running`
/// marks a tab whose foreground app is still alive — the renderer appends a
/// `*` when such a tab is not the active one.
#[derive(Clone, Copy)]
pub struct TabLabel {
    pub name: [u8; 24],
    pub name_len: usize,
    pub running: bool,
}

impl TabLabel {
    pub const fn empty() -> Self {
        Self {
            name: [0; 24],
            name_len: 0,
            running: false,
        }
    }
}

pub const TERMINAL_TAB_WIDGET_BASE: u16 = 0x0300;
pub const TERMINAL_ADD_TAB_WIDGET: interaction::WidgetId = interaction::WidgetId(0x03ff);
const MAX_TERMINAL_TABS: usize = 10;
const TERMINAL_TAB_H: u32 = 26;

pub const fn terminal_tab_widget(index: usize) -> interaction::WidgetId {
    interaction::WidgetId(TERMINAL_TAB_WIDGET_BASE + index as u16)
}

pub const fn terminal_tab_index(id: interaction::WidgetId) -> Option<usize> {
    if id.0 >= TERMINAL_TAB_WIDGET_BASE
        && id.0 < TERMINAL_TAB_WIDGET_BASE + MAX_TERMINAL_TABS as u16
    {
        Some((id.0 - TERMINAL_TAB_WIDGET_BASE) as usize)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalPointerVisual {
    pub hovered: Option<interaction::WidgetId>,
    pub pressed: Option<interaction::WidgetId>,
}

fn terminal_tab_caption(label: &TabLabel, is_active: bool, buf: &mut [u8; 28]) -> usize {
    *buf = [b' '; 28];
    let mut len = 1usize;
    let name = if label.name_len == 0 {
        &b"SHELL"[..]
    } else {
        &label.name[..label.name_len.min(24)]
    };
    for &byte in name {
        if len >= buf.len() - 2 {
            break;
        }
        buf[len] = if byte.is_ascii_lowercase() {
            byte - 32
        } else {
            byte
        };
        len += 1;
    }
    if label.running && !is_active && len < buf.len() - 1 {
        buf[len] = b'*';
        len += 1;
    }
    buf[len] = b' ';
    len + 1
}

/// Final tab-bar pixel geometry shared by rendering and pointer hit testing.
pub struct TerminalTabLayout {
    tab_bounds: [interaction::Rect; MAX_TERMINAL_TABS],
    pub add_tab: interaction::Rect,
    widgets: [interaction::Widget; MAX_TERMINAL_TABS + 1],
    widget_count: usize,
}

impl TerminalTabLayout {
    pub fn new(
        fb_width: u32,
        fb_height: u32,
        labels: &[TabLabel],
        active_tab: usize,
        add_enabled: bool,
    ) -> Self {
        let screen = layout::Layout::new(fb_width, fb_height);
        let tab_y = screen.main.y;
        let empty_rect = interaction::Rect::new(0, 0, 0, 0);
        let empty_widget = interaction::Widget::new(
            interaction::WidgetId(0),
            empty_rect,
            interaction::WidgetKind::Selectable,
        )
        .hidden();
        let mut tab_bounds = [empty_rect; MAX_TERMINAL_TABS];
        let mut widgets = [empty_widget; MAX_TERMINAL_TABS + 1];
        let count = labels.len().min(MAX_TERMINAL_TABS);
        let mut tx = 8u32;
        for (index, label) in labels.iter().take(count).enumerate() {
            let mut caption = [b' '; 28];
            let caption_len = terminal_tab_caption(label, index == active_tab, &mut caption);
            let caption = core::str::from_utf8(&caption[..caption_len]).unwrap_or(" SHELL ");
            let width = fontatlas::measure_text(caption, fontatlas::FontSize::Regular) + 2;
            let bounds = interaction::Rect::new(tx, tab_y + 3, width, TERMINAL_TAB_H - 6);
            tab_bounds[index] = bounds;
            widgets[index] = interaction::Widget::new(
                terminal_tab_widget(index),
                bounds,
                interaction::WidgetKind::Selectable,
            );
            tx = tx.saturating_add(width + 6);
        }

        let add_tab = interaction::Rect::new(tx, tab_y + 3, 24, TERMINAL_TAB_H - 6);
        let add_widget = interaction::Widget::new(
            TERMINAL_ADD_TAB_WIDGET,
            add_tab,
            interaction::WidgetKind::Button,
        );
        widgets[count] = if add_enabled {
            add_widget
        } else {
            add_widget.unavailable()
        };

        Self {
            tab_bounds,
            add_tab,
            widgets,
            widget_count: count + 1,
        }
    }

    pub fn tab(&self, index: usize) -> Option<interaction::Rect> {
        self.tab_bounds
            .get(index)
            .copied()
            .filter(|bounds| bounds.w != 0 && bounds.h != 0)
    }

    pub fn widgets(&self) -> &[interaction::Widget] {
        &self.widgets[..self.widget_count]
    }
}

/// Render the TTY shell screen. Called after successful login and on every key event.
///
/// SAFETY: `fb_addr` must point to a valid writable framebuffer mapping.
pub unsafe fn render_tty_shell(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    tab_count: usize,
    active_tab: usize,
    output: &[u8],
    input_line: &[u8],
    prompt: &[u8],
) {
    let mut fb = framebuffer::Framebuffer::from_limine(fb_addr, fb_width, fb_height, fb_pitch);
    let layout = layout::Layout::new(fb_width, fb_height);
    layout.draw_chrome(&mut fb);

    // Vertically center UI text in header (48px)
    let ui_lh = fontatlas::line_height(fontatlas::FontSize::Regular);
    let hdr_text_y = layout::HEADER_HEIGHT.saturating_sub(ui_lh) / 2;

    // Header — UI atlas for labels, bitmap "*" bullet kept as-is
    font::draw_str(&mut fb, 16, hdr_text_y + 1, "*", layout::palette::ACCENT, 1);
    fontatlas::draw_text(
        &mut fb,
        "SunlightOS",
        32,
        hdr_text_y,
        layout::palette::TEXT,
        fontatlas::FontSize::Regular,
    );
    let mode_label = "TTY";
    let mode_w = fontatlas::measure_text(mode_label, fontatlas::FontSize::Regular);
    fontatlas::draw_text(
        &mut fb,
        mode_label,
        fb_width.saturating_sub(mode_w + 16),
        hdr_text_y,
        layout::palette::ACCENT_DIM,
        fontatlas::FontSize::Regular,
    );

    // Tab bar — 26px high strip immediately below the header separator
    const TAB_H: u32 = 26;
    let tab_y = layout.main.y;
    fb.fill_rect(0, tab_y, fb_width, TAB_H, layout::palette::SURFACE);
    fb.hline(0, tab_y + TAB_H, fb_width, layout::palette::SEPARATOR);

    let tab_text_y = tab_y + (TAB_H.saturating_sub(ui_lh)) / 2;
    let mut tx = 8u32;
    for i in 0..tab_count.min(10) {
        let is_active = i == active_tab;
        let fg = if is_active {
            layout::palette::ACCENT
        } else {
            layout::palette::TEXT_DIM
        };
        let tab_text = " shell ";
        let tw = fontatlas::measure_text(tab_text, fontatlas::FontSize::Regular);
        fb.fill_rect(tx, tab_y + 3, tw + 2, TAB_H - 6, layout::palette::BG);
        fontatlas::draw_text(
            &mut fb,
            tab_text,
            tx + 1,
            tab_text_y,
            fg,
            fontatlas::FontSize::Regular,
        );
        if is_active {
            // Orange underline on active tab
            fb.hline(tx, tab_y + TAB_H - 2, tw + 2, layout::palette::ACCENT);
        }
        tx += tw + 8;
    }

    // Content area — from below tab bar to above footer
    const CHAR_H: u32 = TERM_CHAR_H; // bitmap fallback height for simple shell view
    const MARGIN: u32 = 16;
    let content_y = tab_y + TAB_H + 4;
    let avail_h = layout.footer.y.saturating_sub(content_y + 4);
    let max_visible = (avail_h / CHAR_H) as usize;

    // Split output buffer into lines (collect start offsets)
    let mut line_starts = [0usize; 32];
    let mut line_count = 0usize;
    let mut ls = 0usize;
    for (i, &b) in output.iter().enumerate() {
        if b == b'\n' {
            if line_count < 32 {
                line_starts[line_count] = ls;
                line_count += 1;
            }
            ls = i + 1;
        }
    }
    if ls < output.len() && line_count < 32 {
        line_starts[line_count] = ls;
        line_count += 1;
    }

    let start_line = if line_count > max_visible {
        line_count - max_visible
    } else {
        0
    };
    for li in start_line..line_count {
        let ly = content_y + (li - start_line) as u32 * CHAR_H;
        let lstart = line_starts[li];
        let lend = if li + 1 < line_count {
            line_starts[li + 1].saturating_sub(1)
        } else {
            output.len()
        };
        tty_draw_line(
            &mut fb,
            MARGIN,
            ly,
            &output[lstart..lend.min(output.len())],
            layout::palette::TEXT,
            1,
        );
    }

    // Footer — prompt in accent colour, then current input in white
    let footer_text_y = layout.footer.y + 8;
    tty_draw_line(
        &mut fb,
        MARGIN,
        footer_text_y,
        prompt,
        layout::palette::ACCENT,
        1,
    );
    let prompt_w = prompt.len() as u32 * 8; // scale-1 glyph is always 8px wide
    tty_draw_line(
        &mut fb,
        MARGIN + prompt_w,
        footer_text_y,
        input_line,
        layout::palette::TEXT,
        1,
    );
    // Block cursor
    let cursor_x = MARGIN + (prompt.len() + input_line.len()) as u32 * 8;
    fb.fill_rect(cursor_x, footer_text_y, 8, 16, layout::palette::ACCENT);
}

/// Draw a slice of ASCII bytes as a single line (stops at `\n`, `\r`, or `\0`).
fn tty_draw_line(
    fb: &mut framebuffer::Framebuffer,
    mut x: u32,
    y: u32,
    bytes: &[u8],
    color: u32,
    scale: u32,
) {
    for &b in bytes {
        if b == b'\n' || b == b'\r' || b == 0 {
            break;
        }
        if b >= 0x20 && b <= 0x7E {
            font::draw_char(fb, x, y, b, color, scale);
            x += 8 * scale;
        }
    }
}

/// Render a 2D character grid with full color support (VT100 terminal).
/// Called after successful login to display terminal content with ANSI colors.
///
/// SAFETY: `fb_addr` must point to a valid writable framebuffer mapping.
/// Terminal cell metrics and tab-bar height, shared by the grid sizer and the
/// renderer so they never disagree (a mismatch clips rows off the top).
pub const TERM_CHAR_W: u32 = 8;
pub const TERM_CHAR_H: u32 = 18;
pub const TERM_TAB_BAR_H: u32 = 26;

/// Authoritative terminal renderer cell metrics. Grid sizing and any frontend
/// winsize publication must use this function rather than duplicating atlas
/// fallback constants.
pub fn terminal_cell_metrics() -> (u32, u32) {
    fontatlas::cell_metrics(fontatlas::FontSize::MonoRegular)
}

/// Compute the (cols, rows) of the terminal content area for a framebuffer.
/// Callers must size the grid with exactly these dimensions so the renderer
/// shows every row from the top with no clipping.
///
/// Uses Fira Code mono atlas cell metrics when available; falls back to
/// `TERM_CHAR_W`/`TERM_CHAR_H` (bitmap dimensions) if the atlas is missing.
pub fn terminal_dims(fb_width: u32, fb_height: u32) -> (usize, usize) {
    let layout = layout::Layout::new(fb_width, fb_height);
    let (cell_w, cell_h) = terminal_cell_metrics();
    let content_y = layout.main.y + TERM_TAB_BAR_H + 4;
    let avail_h = layout.footer.y.saturating_sub(content_y + 4);
    let rows = (avail_h / cell_h) as usize;
    // subtract 16px left margin + 16px right margin
    let avail_w = fb_width.saturating_sub(32);
    let cols = (avail_w / cell_w) as usize;
    (cols, rows)
}

pub unsafe fn render_terminal_grid_interactive(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    tab_labels: &[TabLabel],
    active_tab: usize,
    cols: usize,
    rows: usize,
    cells: &[TermCell],
    cursor_row: usize,
    cursor_col: usize,
    input_line: &[u8],
    prompt: &[u8],
    clock: &[u8],
    input_cursor: usize,
    pointer: TerminalPointerVisual,
    add_enabled: bool,
) {
    let mut fb = framebuffer::Framebuffer::from_limine(fb_addr, fb_width, fb_height, fb_pitch);
    let layout = layout::Layout::new(fb_width, fb_height);
    layout.draw_chrome(&mut fb);

    // Mono cell metrics — Fira Code atlas or bitmap fallback (8, 18).
    // Must match terminal_dims() exactly so rows aren't clipped.
    let (cell_w, cell_h) = terminal_cell_metrics();

    // Vertically center UI text in header (48px)
    let ui_lh = fontatlas::line_height(fontatlas::FontSize::Regular);
    let hdr_text_y = layout::HEADER_HEIGHT.saturating_sub(ui_lh) / 2;

    // Header: logo left, clock right, mode label left of the clock — UI atlas
    font::draw_str(&mut fb, 16, hdr_text_y + 1, "*", layout::palette::ACCENT, 1);
    fontatlas::draw_text(
        &mut fb,
        "SunlightOS",
        32,
        hdr_text_y,
        layout::palette::ACCENT,
        fontatlas::FontSize::Regular,
    );
    let clock_str = core::str::from_utf8(clock).unwrap_or("");
    let clock_w = fontatlas::measure_text(clock_str, fontatlas::FontSize::Regular);
    let clock_x = fb_width.saturating_sub(clock_w + 16);
    if !clock_str.is_empty() {
        fontatlas::draw_text(
            &mut fb,
            clock_str,
            clock_x,
            hdr_text_y,
            layout::palette::TEXT,
            fontatlas::FontSize::Regular,
        );
    }
    let mode_label = "TTY";
    let mode_w = fontatlas::measure_text(mode_label, fontatlas::FontSize::Regular);
    fontatlas::draw_text(
        &mut fb,
        mode_label,
        clock_x.saturating_sub(mode_w + 24),
        hdr_text_y,
        layout::palette::ACCENT_DIM,
        fontatlas::FontSize::Regular,
    );

    // Tab bar
    let tab_y = layout.main.y;
    fb.fill_rect(0, tab_y, fb_width, TERMINAL_TAB_H, layout::palette::SURFACE);
    fb.hline(
        0,
        tab_y + TERMINAL_TAB_H,
        fb_width,
        layout::palette::SEPARATOR,
    );

    let tab_layout =
        TerminalTabLayout::new(fb_width, fb_height, tab_labels, active_tab, add_enabled);
    let tab_text_y = tab_y + TERMINAL_TAB_H.saturating_sub(ui_lh) / 2;
    for (i, label) in tab_labels.iter().take(MAX_TERMINAL_TABS).enumerate() {
        let is_active = i == active_tab;
        let id = terminal_tab_widget(i);
        let hovered = pointer.hovered == Some(id);
        let pressed = pointer.pressed == Some(id);
        let fg = if is_active || hovered {
            layout::palette::ACCENT
        } else {
            layout::palette::TEXT_DIM
        };
        let mut buf = [b' '; 28];
        let n = terminal_tab_caption(label, is_active, &mut buf);
        let tab_text = core::str::from_utf8(&buf[..n]).unwrap_or(" SHELL ");
        let Some(bounds) = tab_layout.tab(i) else {
            continue;
        };
        fb.fill_rect(
            bounds.x,
            bounds.y,
            bounds.w,
            bounds.h,
            if pressed {
                0x241200
            } else if hovered {
                0x171008
            } else {
                layout::palette::BG
            },
        );
        fontatlas::draw_text(
            &mut fb,
            tab_text,
            bounds.x + 1,
            tab_text_y,
            fg,
            fontatlas::FontSize::Regular,
        );
        if is_active {
            fb.hline(
                bounds.x,
                tab_y + TERMINAL_TAB_H - 2,
                bounds.w,
                layout::palette::ACCENT,
            );
        }
    }

    let add_hovered = pointer.hovered == Some(TERMINAL_ADD_TAB_WIDGET);
    let add_pressed = pointer.pressed == Some(TERMINAL_ADD_TAB_WIDGET);
    let add = tab_layout.add_tab;
    fb.fill_rect(
        add.x,
        add.y,
        add.w,
        add.h,
        if add_pressed {
            0x241200
        } else if add_hovered {
            0x171008
        } else {
            layout::palette::BG
        },
    );
    fontatlas::draw_text(
        &mut fb,
        "+",
        add.x + 7,
        tab_text_y,
        if add_enabled {
            if add_hovered {
                layout::palette::ACCENT
            } else {
                layout::palette::TEXT
            }
        } else {
            layout::palette::TEXT_DIM
        },
        fontatlas::FontSize::Regular,
    );

    // Content area: render the grid using Fira Code mono atlas.
    // cell_h must match terminal_dims() exactly so rows aren't clipped off the top.
    const MARGIN: u32 = 16;
    let content_y = tab_y + TERMINAL_TAB_H + 4;
    let avail_h = layout.footer.y.saturating_sub(content_y + 4);
    let max_visible = (avail_h / cell_h) as usize;

    // Only show the last `max_visible` rows
    let start_row = if rows > max_visible {
        rows - max_visible
    } else {
        0
    };
    for row in start_row..rows {
        let screen_row = row - start_row;
        let y = content_y + (screen_row as u32) * cell_h;

        for col in 0..cols {
            let cell_idx = row * cols + col;
            if cell_idx >= cells.len() {
                break;
            }
            let cell = cells[cell_idx];
            let x = MARGIN + (col as u32) * cell_w;

            // Fill cell background first, then alpha-blend the glyph on top
            fb.fill_rect(x, y, cell_w, cell_h, cell.bg);
            if cell.ch != b' ' && cell.ch >= 0x20 && cell.ch <= 0x7E {
                fontatlas::draw_mono_char(
                    &mut fb,
                    x,
                    y,
                    cell_h,
                    cell.ch,
                    cell.fg,
                    fontatlas::FontSize::MonoRegular,
                );
            }
        }
    }

    // Draw the grid cursor only when the caller is not rendering a separate
    // prompt/input area. The TTY shell keeps the live prompt in the footer.
    if prompt.is_empty() && input_line.is_empty() && cursor_row >= start_row && cursor_row < rows {
        let screen_row = cursor_row - start_row;
        let y = content_y + (screen_row as u32) * cell_h;
        let x = MARGIN + (cursor_col as u32) * cell_w;

        // Inverted: swap fg/bg for cursor cell
        let cell_idx = cursor_row * cols + cursor_col.min(cols - 1);
        if cell_idx < cells.len() {
            let cell = cells[cell_idx];
            fb.fill_rect(x, y, cell_w, cell_h, cell.fg); // fg becomes bg
            if cell.ch != b' ' && cell.ch >= 0x20 && cell.ch <= 0x7E {
                fontatlas::draw_mono_char(
                    &mut fb,
                    x,
                    y,
                    cell_h,
                    cell.ch,
                    cell.bg,
                    fontatlas::FontSize::MonoRegular,
                ); // bg becomes fg
            }
        } else {
            // Empty cell: just draw inverted space
            fb.fill_rect(x, y, cell_w, cell_h, layout::palette::TEXT);
        }
    }

    // Footer: prompt + input + command cursor (bitmap — keeps cursor math simple)
    let footer_text_y = layout.footer.y + 8;
    tty_draw_line(
        &mut fb,
        MARGIN,
        footer_text_y,
        prompt,
        layout::palette::ACCENT,
        1,
    );
    let prompt_w = prompt.len() as u32 * 8;
    tty_draw_line(
        &mut fb,
        MARGIN + prompt_w,
        footer_text_y,
        input_line,
        layout::palette::TEXT,
        1,
    );
    // Edit cursor: drawn at `input_cursor` within the input line (defaults to
    // end of line). When the cursor sits over an existing character (mid-line
    // editing via Left/Right), draw an inverted block so the character stays
    // visible; at end-of-line draw a solid block.
    let cur = input_cursor.min(input_line.len());
    let cursor_x = MARGIN + (prompt.len() + cur) as u32 * 8;
    fb.fill_rect(cursor_x, footer_text_y, 8, 16, layout::palette::ACCENT);
    if cur < input_line.len() {
        let ch = input_line[cur];
        if ch >= 0x20 && ch <= 0x7E {
            font::draw_char(&mut fb, cursor_x, footer_text_y, ch, layout::palette::BG, 1);
        }
    }
}

/// Compatibility renderer for terminal users that do not provide pointer
/// interaction state.
pub unsafe fn render_terminal_grid(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    tab_labels: &[TabLabel],
    active_tab: usize,
    cols: usize,
    rows: usize,
    cells: &[TermCell],
    cursor_row: usize,
    cursor_col: usize,
    input_line: &[u8],
    prompt: &[u8],
    clock: &[u8],
    input_cursor: usize,
) {
    render_terminal_grid_interactive(
        fb_addr,
        fb_width,
        fb_height,
        fb_pitch,
        tab_labels,
        active_tab,
        cols,
        rows,
        cells,
        cursor_row,
        cursor_col,
        input_line,
        prompt,
        clock,
        input_cursor,
        TerminalPointerVisual::default(),
        tab_labels.len() < MAX_TERMINAL_TABS,
    );
}

/// Which login widget has keyboard focus (mirrors `sunlight_tty::login::FocusArea`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginFocus {
    UserSlot(usize),
    Password,
    Dropdown,
    Reboot,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginUserIcon {
    User,
    Luggage,
}

pub const LOGIN_USER_WIDGET_BASE: u16 = 0x0100;
pub const LOGIN_PASSWORD_WIDGET: interaction::WidgetId = interaction::WidgetId(0x0200);
pub const LOGIN_DROPDOWN_WIDGET: interaction::WidgetId = interaction::WidgetId(0x0201);
pub const LOGIN_REBOOT_WIDGET: interaction::WidgetId = interaction::WidgetId(0x0202);
pub const LOGIN_SHUTDOWN_WIDGET: interaction::WidgetId = interaction::WidgetId(0x0203);
const MAX_LOGIN_USER_WIDGETS: usize = 6;
const MAX_LOGIN_WIDGETS: usize = MAX_LOGIN_USER_WIDGETS + 4;

pub const fn login_user_widget(index: usize) -> interaction::WidgetId {
    interaction::WidgetId(LOGIN_USER_WIDGET_BASE + index as u16)
}

pub const fn login_user_index(id: interaction::WidgetId) -> Option<usize> {
    if id.0 >= LOGIN_USER_WIDGET_BASE
        && id.0 < LOGIN_USER_WIDGET_BASE + MAX_LOGIN_USER_WIDGETS as u16
    {
        Some((id.0 - LOGIN_USER_WIDGET_BASE) as usize)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoginPointerVisual {
    pub hovered: Option<interaction::WidgetId>,
    pub pressed: Option<interaction::WidgetId>,
}

/// Final pixel geometry for the login controls. Rendering and hit testing both
/// consume this structure so their bounds cannot silently drift apart.
pub struct LoginLayout {
    pub panel: interaction::Rect,
    user_slots: [interaction::Rect; MAX_LOGIN_USER_WIDGETS],
    pub password: interaction::Rect,
    pub dropdown: interaction::Rect,
    pub reboot: interaction::Rect,
    pub shutdown: interaction::Rect,
    widgets: [interaction::Widget; MAX_LOGIN_WIDGETS],
    widget_count: usize,
}

impl LoginLayout {
    pub fn new(fb_width: u32, fb_height: u32, active_count: usize, enabled: bool) -> Self {
        const SLOT_W: u32 = 96;
        const USER_H: u32 = 64;
        const BTN_H: u32 = 40;
        const BTN_ICON: u32 = 32;
        const BTN_GAP: u32 = 12;
        const BTN_ICON_PAD: u32 = 4;
        const BTN_TEXT_GAP: u32 = 6;
        const BTN_RIGHT_PAD: u32 = 10;

        let screen = layout::Layout::new(fb_width, fb_height);
        let panel_w = 520u32.min(screen.main.w.saturating_sub(32));
        let panel_h = 360u32;
        let panel_x = screen.main.x + screen.main.w.saturating_sub(panel_w) / 2;
        let panel_y = screen.main.y + screen.main.h.saturating_sub(panel_h) / 2;
        let panel = interaction::Rect::new(panel_x, panel_y, panel_w, panel_h);

        let count = active_count.min(MAX_LOGIN_USER_WIDGETS);
        let row_w = count as u32 * SLOT_W;
        let row_x = panel_x + panel_w.saturating_sub(row_w) / 2;
        let mut user_slots = [interaction::Rect::new(0, 0, 0, 0); MAX_LOGIN_USER_WIDGETS];
        for (index, slot) in user_slots.iter_mut().enumerate().take(count) {
            *slot =
                interaction::Rect::new(row_x + index as u32 * SLOT_W, panel_y + 68, SLOT_W, USER_H);
        }

        let field_x = panel_x + 40;
        let field_right = panel_x + panel_w.saturating_sub(40);
        let password_x =
            field_x + fontatlas::measure_text("Password: ", fontatlas::FontSize::Regular);
        let password = interaction::Rect::new(
            password_x,
            panel_y + 148,
            field_right.saturating_sub(password_x),
            26,
        );
        let dropdown_x =
            field_x + fontatlas::measure_text("Session:  ", fontatlas::FontSize::Regular);
        let dropdown = interaction::Rect::new(dropdown_x, panel_y + 192, 130, 26);

        let reboot_w = BTN_ICON_PAD
            + BTN_ICON
            + BTN_TEXT_GAP
            + fontatlas::measure_text("Reboot", fontatlas::FontSize::Regular)
            + BTN_RIGHT_PAD;
        let shutdown_w = BTN_ICON_PAD
            + BTN_ICON
            + BTN_TEXT_GAP
            + fontatlas::measure_text("Shutdown", fontatlas::FontSize::Regular)
            + BTN_RIGHT_PAD;
        let shutdown_x = panel_x + panel_w.saturating_sub(shutdown_w + 14);
        let reboot_x = shutdown_x.saturating_sub(reboot_w + BTN_GAP);
        let reboot = interaction::Rect::new(reboot_x, panel_y + 286, reboot_w, BTN_H);
        let shutdown = interaction::Rect::new(shutdown_x, panel_y + 286, shutdown_w, BTN_H);

        let empty = interaction::Widget::new(
            interaction::WidgetId(0),
            interaction::Rect::new(0, 0, 0, 0),
            interaction::WidgetKind::Button,
        )
        .hidden();
        let mut widgets = [empty; MAX_LOGIN_WIDGETS];
        let mut widget_count = 0;
        for (index, bounds) in user_slots.iter().copied().enumerate().take(count) {
            let widget = interaction::Widget::new(
                login_user_widget(index),
                bounds,
                interaction::WidgetKind::Selectable,
            );
            widgets[widget_count] = if enabled {
                widget
            } else {
                widget.unavailable()
            };
            widget_count += 1;
        }
        for widget in [
            interaction::Widget::new(
                LOGIN_PASSWORD_WIDGET,
                password,
                interaction::WidgetKind::TextInput,
            ),
            interaction::Widget::new(
                LOGIN_DROPDOWN_WIDGET,
                dropdown,
                interaction::WidgetKind::Button,
            ),
            interaction::Widget::new(LOGIN_REBOOT_WIDGET, reboot, interaction::WidgetKind::Button),
            interaction::Widget::new(
                LOGIN_SHUTDOWN_WIDGET,
                shutdown,
                interaction::WidgetKind::Button,
            ),
        ] {
            widgets[widget_count] = if enabled {
                widget
            } else {
                widget.unavailable()
            };
            widget_count += 1;
        }

        Self {
            panel,
            user_slots,
            password,
            dropdown,
            reboot,
            shutdown,
            widgets,
            widget_count,
        }
    }

    pub fn user_slot(&self, index: usize) -> Option<interaction::Rect> {
        self.user_slots
            .get(index)
            .copied()
            .filter(|bounds| bounds.w != 0 && bounds.h != 0)
    }

    pub fn widgets(&self) -> &[interaction::Widget] {
        &self.widgets[..self.widget_count]
    }
}

/// Draw a user avatar tile: 32×32 Material Icon (or TGA) inside a 40×40 slot box.
///
/// Falls back to a letter glyph when `icon` is `None` (icon failed to load).
/// Orange border on focused/selected; subtle border otherwise.
fn draw_user_avatar(
    fb: &mut framebuffer::Framebuffer,
    cx: u32,       // horizontal center of the slot
    icon_top: u32, // top of the 32×32 icon area
    icon: Option<&tga::TgaImage<'_>>,
    name: &[u8],
    is_custom: bool,
    selected: bool,
    focused: bool,
    hovered: bool,
    pressed: bool,
) {
    const ICON_SZ: u32 = 32;
    const PAD: u32 = 4;
    const BOX_SZ: u32 = ICON_SZ + PAD * 2; // 40×40 slot box

    let box_x = cx.saturating_sub(BOX_SZ / 2);
    let box_y = icon_top.saturating_sub(PAD);

    // Tile background: warm orange tint when selected, dark when inactive.
    let fill = if pressed {
        0x2A1100
    } else if focused {
        0x1C0C00
    } else if hovered {
        0x140A02
    } else if selected {
        0x160900
    } else {
        0x0A0A0A
    };
    fb.fill_rect(box_x, box_y, BOX_SZ, BOX_SZ, fill);

    let border_color = if focused || hovered {
        layout::palette::ACCENT
    } else if selected {
        layout::palette::ACCENT_DIM
    } else {
        layout::palette::SEPARATOR
    };
    draw::rect_outline(
        fb,
        box_x,
        box_y,
        BOX_SZ,
        BOX_SZ,
        if focused || pressed { 2 } else { 1 },
        border_color,
    );

    // Fallback icon color: orange for focused, dimmed orange for selected,
    // muted gray for inactive.
    let icon_color = if focused || hovered {
        layout::palette::ACCENT
    } else if selected {
        layout::palette::ACCENT_DIM
    } else {
        layout::palette::TEXT_DIM
    };

    let icon_x = cx.saturating_sub(ICON_SZ / 2);
    if let Some(img) = icon {
        tga::draw_tga_icon_tinted(
            fb,
            Some(img),
            icon_x,
            icon_top,
            ICON_SZ,
            ICON_SZ,
            icon_color,
        );
    } else {
        // Fallback: draw the first letter of the name
        let ch = if name.is_empty() {
            if is_custom {
                b'+'
            } else {
                b'?'
            }
        } else {
            let c = name[0];
            if c >= b'a' && c <= b'z' {
                c - 32
            } else {
                c
            }
        };
        font::draw_char(fb, icon_x + 12, icon_top + 9, ch, icon_color, 1);
    }
}

/// Render the grid-based login screen with user avatars, password, and session dropdown.
///
/// Background: TGA image with dark overlay (falls back to solid dark if decode fails).
/// User avatars: 32×32 Material Icon (account_circle) with orange selection ring.
/// Action buttons: 32×32 Material Icons (restart_alt / power_settings_new) with label.
/// Password and session fields: outlined input boxes, orange border when focused.
/// Icons: generated from Material-Icons TTF via sunlight-tui/build.rs (no more checked-in TGAs)
///
/// SAFETY: `fb_addr` must point to a valid writable framebuffer mapping.
pub unsafe fn render_login_grid_interactive(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    bg: Option<LoginBackground<'_>>,
    user_bufs: &[[u8; 64]],
    user_lens: &[usize],
    user_labels: &[&str],
    user_icons: &[LoginUserIcon],
    is_custom: &[bool],
    active_count: usize,
    selected_user_idx: usize,
    focus: LoginFocus,
    session_label: &str,
    password_len: usize,
    message: &str,
    pointer: LoginPointerVisual,
) {
    // Parse login icons — cheap (header-only), no heap allocation.
    let icon_users = tga::TgaImage::parse(ICON_USERS);
    let icon_luggage = tga::TgaImage::parse(ICON_LUGGAGE);
    let icon_reboot = tga::TgaImage::parse(ICON_REBOOT);
    let icon_shutdown = tga::TgaImage::parse(ICON_SHUTDOWN);

    let mut fb = framebuffer::Framebuffer::from_limine(fb_addr, fb_width, fb_height, fb_pitch);
    let layout = layout::Layout::new(fb_width, fb_height);

    paint_login_background(&mut fb, &layout, bg, 110);

    // ── Top bar ─────────────────────────────────────────────────────────────
    font::draw_str(&mut fb, 16, 16, "*", layout::palette::ACCENT, 1);
    font::draw_str(&mut fb, 32, 16, "SunlightOS", layout::palette::TEXT, 1);
    let mode = "TTY Login";
    let mode_w = fontatlas::measure_text(mode, fontatlas::FontSize::Regular);
    fontatlas::draw_text(
        &mut fb,
        mode,
        fb_width.saturating_sub(mode_w + 16),
        16,
        layout::palette::TEXT_DIM,
        fontatlas::FontSize::Regular,
    );

    // ── Login card ──────────────────────────────────────────────────────────
    let login_layout = LoginLayout::new(fb_width, fb_height, active_count, true);
    let panel_x = login_layout.panel.x;
    let panel_y = login_layout.panel.y;
    let panel_w = login_layout.panel.w;
    let panel_h = login_layout.panel.h;

    // Card: solid dark fill + 2 px border for stronger separation
    fb.fill_rect(panel_x, panel_y, panel_w, panel_h, layout::palette::BG);
    draw::rect_outline(&mut fb, panel_x, panel_y, panel_w, panel_h, 2, 0x3A3A3A);

    // ── Title ───────────────────────────────────────────────────────────────
    let title = "Welcome to SunlightOS";
    let title_w = fontatlas::measure_text(title, fontatlas::FontSize::Title);
    fontatlas::draw_text(
        &mut fb,
        title,
        panel_x + panel_w.saturating_sub(title_w) / 2,
        panel_y + 20,
        layout::palette::ACCENT,
        fontatlas::FontSize::Title,
    );

    // ── User avatar row ─────────────────────────────────────────────────────
    // Each slot: 96 px wide, 32×32 icon centred, name 8 px below the icon.
    const ICON_SZ: u32 = 32;
    let avatar_icon_top = panel_y + 72;

    for i in 0..active_count
        .min(user_bufs.len())
        .min(user_lens.len())
        .min(user_labels.len())
        .min(user_icons.len())
        .min(is_custom.len())
    {
        let Some(slot_bounds) = login_layout.user_slot(i) else {
            continue;
        };
        let slot_cx = slot_bounds.x + slot_bounds.w / 2;
        let name = &user_bufs[i][..user_lens[i]];
        let focused = focus == LoginFocus::UserSlot(i);
        let selected = i == selected_user_idx;
        let icon = match user_icons[i] {
            LoginUserIcon::User => icon_users.as_ref(),
            LoginUserIcon::Luggage => icon_luggage.as_ref(),
        };

        draw_user_avatar(
            &mut fb,
            slot_cx,
            avatar_icon_top,
            icon,
            name,
            is_custom[i],
            selected,
            focused,
            pointer.hovered == Some(login_user_widget(i)),
            pointer.pressed == Some(login_user_widget(i)),
        );

        let label = user_labels[i];
        let label_w = fontatlas::measure_text(label, fontatlas::FontSize::Regular);
        fontatlas::draw_text(
            &mut fb,
            label,
            slot_cx.saturating_sub(label_w / 2),
            avatar_icon_top + ICON_SZ + 8,
            if selected {
                layout::palette::TEXT
            } else {
                layout::palette::TEXT_DIM
            },
            fontatlas::FontSize::Regular,
        );
    }

    // ── Form fields ─────────────────────────────────────────────────────────
    // Both password and session use the same outlined-box language.
    let field_x = panel_x + 40;

    // — Password —
    let pass_y = panel_y + 152;
    fontatlas::draw_text(
        &mut fb,
        "Password:",
        field_x,
        pass_y,
        layout::palette::TEXT_DIM,
        fontatlas::FontSize::Regular,
    );
    let pw_box_x = login_layout.password.x;
    let pw_box_w = login_layout.password.w;
    let pw_box_h = login_layout.password.h;
    let pw_focused = focus == LoginFocus::Password;
    let pw_hovered = pointer.hovered == Some(LOGIN_PASSWORD_WIDGET);

    // Box fill: slightly brighter when active to hint at focus
    fb.fill_rect(
        pw_box_x,
        pass_y.saturating_sub(4),
        pw_box_w,
        pw_box_h,
        if pointer.pressed == Some(LOGIN_PASSWORD_WIDGET) {
            0x181000
        } else if pw_focused {
            0x0E0D00
        } else if pw_hovered {
            0x10100A
        } else {
            0x080808
        },
    );
    draw::rect_outline(
        &mut fb,
        pw_box_x,
        pass_y.saturating_sub(4),
        pw_box_w,
        pw_box_h,
        if pw_focused { 2 } else { 1 },
        if pw_focused || pw_hovered {
            layout::palette::ACCENT
        } else {
            layout::palette::SEPARATOR
        },
    );
    // Bullet dots
    let dot_count = password_len.min(24) as u32;
    for i in 0..dot_count {
        font::draw_char(
            &mut fb,
            pw_box_x + 6 + i * 8,
            pass_y,
            b'*',
            layout::palette::TEXT,
            1,
        );
    }
    // Caret
    if pw_focused {
        let cx = pw_box_x + 6 + dot_count * 8;
        fb.fill_rect(cx, pass_y, 8, 14, layout::palette::ACCENT);
    }

    // — Session selector —
    let drop_y = panel_y + 196;
    fontatlas::draw_text(
        &mut fb,
        "Session:",
        field_x,
        drop_y,
        layout::palette::TEXT_DIM,
        fontatlas::FontSize::Regular,
    );
    let drop_box_x = login_layout.dropdown.x;
    let drop_box_w = login_layout.dropdown.w;
    let drop_box_h = login_layout.dropdown.h;
    let drop_focused = focus == LoginFocus::Dropdown;
    let drop_hovered = pointer.hovered == Some(LOGIN_DROPDOWN_WIDGET);

    fb.fill_rect(
        drop_box_x,
        drop_y.saturating_sub(4),
        drop_box_w,
        drop_box_h,
        if pointer.pressed == Some(LOGIN_DROPDOWN_WIDGET) {
            0x181000
        } else if drop_focused {
            0x0E0D00
        } else if drop_hovered {
            0x10100A
        } else {
            0x080808
        },
    );
    draw::rect_outline(
        &mut fb,
        drop_box_x,
        drop_y.saturating_sub(4),
        drop_box_w,
        drop_box_h,
        if drop_focused { 2 } else { 1 },
        if drop_focused || drop_hovered {
            layout::palette::ACCENT
        } else {
            layout::palette::SEPARATOR
        },
    );
    fontatlas::draw_text(
        &mut fb,
        session_label,
        drop_box_x + 8,
        drop_y,
        layout::palette::TEXT,
        fontatlas::FontSize::Regular,
    );
    // Dropdown arrow indicator
    font::draw_str(
        &mut fb,
        drop_box_x + drop_box_w.saturating_sub(16),
        drop_y,
        "v",
        layout::palette::TEXT_DIM,
        1,
    );

    // ── Status message ───────────────────────────────────────────────────────
    if !message.is_empty() {
        let msg_y = panel_y + 248;
        let msg_w = fontatlas::measure_text(message, fontatlas::FontSize::Regular);
        fontatlas::draw_text(
            &mut fb,
            message,
            panel_x + panel_w.saturating_sub(msg_w) / 2,
            msg_y,
            layout::palette::TEXT_DIM,
            fontatlas::FontSize::Regular,
        );
    }

    // ── Action buttons ───────────────────────────────────────────────────────
    // Layout: icon (32×32) on the left, label centred vertically on the right.
    // Button height = 40 px (4 px padding each side of the 32 px icon).
    const BTN_H: u32 = 40;
    const BTN_ICON: u32 = 32;
    const BTN_ICON_PAD: u32 = 4; // padding left of icon
    const BTN_TEXT_GAP: u32 = 6; // gap between icon right edge and label
    const BTN_RIGHT_PAD: u32 = 10; // padding right of label

    let btn_y = panel_y + 286;
    let reboot_focused = focus == LoginFocus::Reboot;
    let shutdown_focused = focus == LoginFocus::Shutdown;

    let rb_label = "Reboot";
    let rb_lw = fontatlas::measure_text(rb_label, fontatlas::FontSize::Regular);
    let rb_btn_w = BTN_ICON_PAD + BTN_ICON + BTN_TEXT_GAP + rb_lw + BTN_RIGHT_PAD;

    let sd_label = "Shutdown";
    let sd_lw = fontatlas::measure_text(sd_label, fontatlas::FontSize::Regular);
    let sd_btn_w = BTN_ICON_PAD + BTN_ICON + BTN_TEXT_GAP + sd_lw + BTN_RIGHT_PAD;

    let shutdown_x = login_layout.shutdown.x;
    let reboot_x = login_layout.reboot.x;

    // Reboot button
    {
        let reboot_hovered = pointer.hovered == Some(LOGIN_REBOOT_WIDGET);
        let bg = if pointer.pressed == Some(LOGIN_REBOOT_WIDGET) {
            0x2A1100
        } else if reboot_focused {
            0x1C0C00
        } else if reboot_hovered {
            0x140A02
        } else {
            0x080808
        };
        fb.fill_rect(reboot_x, btn_y, rb_btn_w, BTN_H, bg);
        draw::rect_outline(
            &mut fb,
            reboot_x,
            btn_y,
            rb_btn_w,
            BTN_H,
            if reboot_focused { 2 } else { 1 },
            if reboot_focused || reboot_hovered {
                layout::palette::ACCENT
            } else {
                layout::palette::SEPARATOR
            },
        );
        tga::draw_tga_icon_tinted(
            &mut fb,
            icon_reboot.as_ref(),
            reboot_x + BTN_ICON_PAD,
            btn_y + (BTN_H.saturating_sub(BTN_ICON)) / 2,
            BTN_ICON,
            BTN_ICON,
            if reboot_focused || reboot_hovered {
                layout::palette::ACCENT
            } else {
                layout::palette::TEXT_DIM
            },
        );
        fontatlas::draw_text(
            &mut fb,
            rb_label,
            reboot_x + BTN_ICON_PAD + BTN_ICON + BTN_TEXT_GAP,
            btn_y + (BTN_H.saturating_sub(14)) / 2,
            if reboot_focused || reboot_hovered {
                layout::palette::ACCENT
            } else {
                layout::palette::TEXT_DIM
            },
            fontatlas::FontSize::Regular,
        );
    }

    // Shutdown button
    {
        let shutdown_hovered = pointer.hovered == Some(LOGIN_SHUTDOWN_WIDGET);
        let bg = if pointer.pressed == Some(LOGIN_SHUTDOWN_WIDGET) {
            0x2A1100
        } else if shutdown_focused {
            0x1C0C00
        } else if shutdown_hovered {
            0x140A02
        } else {
            0x080808
        };
        fb.fill_rect(shutdown_x, btn_y, sd_btn_w, BTN_H, bg);
        draw::rect_outline(
            &mut fb,
            shutdown_x,
            btn_y,
            sd_btn_w,
            BTN_H,
            if shutdown_focused { 2 } else { 1 },
            if shutdown_focused || shutdown_hovered {
                layout::palette::ACCENT
            } else {
                layout::palette::SEPARATOR
            },
        );
        tga::draw_tga_icon_tinted(
            &mut fb,
            icon_shutdown.as_ref(),
            shutdown_x + BTN_ICON_PAD,
            btn_y + (BTN_H.saturating_sub(BTN_ICON)) / 2,
            BTN_ICON,
            BTN_ICON,
            if shutdown_focused || shutdown_hovered {
                layout::palette::ACCENT
            } else {
                layout::palette::TEXT_DIM
            },
        );
        fontatlas::draw_text(
            &mut fb,
            sd_label,
            shutdown_x + BTN_ICON_PAD + BTN_ICON + BTN_TEXT_GAP,
            btn_y + (BTN_H.saturating_sub(14)) / 2,
            if shutdown_focused || shutdown_hovered {
                layout::palette::ACCENT
            } else {
                layout::palette::TEXT_DIM
            },
            fontatlas::FontSize::Regular,
        );
    }

    // ── Bottom hint bar ──────────────────────────────────────────────────────
    let footer = "Tab to navigate   Enter to select   Space/Up/Down toggle session";
    let footer_w = fontatlas::measure_text(footer, fontatlas::FontSize::Regular);
    fontatlas::draw_text(
        &mut fb,
        footer,
        fb_width.saturating_sub(footer_w) / 2,
        fb_height.saturating_sub(22),
        layout::palette::TEXT_DIM,
        fontatlas::FontSize::Regular,
    );
}

/// Compatibility renderer for callers that do not yet expose pointer state.
#[allow(clippy::too_many_arguments)]
pub unsafe fn render_login_grid(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    bg: Option<LoginBackground<'_>>,
    user_bufs: &[[u8; 64]],
    user_lens: &[usize],
    user_labels: &[&str],
    user_icons: &[LoginUserIcon],
    is_custom: &[bool],
    active_count: usize,
    selected_user_idx: usize,
    focus: LoginFocus,
    session_label: &str,
    password_len: usize,
    message: &str,
) {
    unsafe {
        render_login_grid_interactive(
            fb_addr,
            fb_width,
            fb_height,
            fb_pitch,
            bg,
            user_bufs,
            user_lens,
            user_labels,
            user_icons,
            is_custom,
            active_count,
            selected_user_idx,
            focus,
            session_label,
            password_len,
            message,
            LoginPointerVisual::default(),
        );
    }
}

pub unsafe fn render_login_dynamic(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    bg: Option<LoginBackground<'_>>,
    username: &[u8],
    password_len: usize,
    focused_password: bool,
    message: &str,
) {
    let mut fb = framebuffer::Framebuffer::from_limine(fb_addr, fb_width, fb_height, fb_pitch);
    let layout = layout::Layout::new(fb_width, fb_height);

    paint_login_background(&mut fb, &layout, bg, 100);

    font::draw_str(&mut fb, 16, 16, "*", layout::palette::ACCENT, 1);
    font::draw_str(&mut fb, 32, 16, "SunlightOS", layout::palette::TEXT, 1);
    let mode = "TTY Login";
    let mode_w = font::text_width(mode, 1);
    font::draw_str(
        &mut fb,
        fb_width.saturating_sub(mode_w + 16),
        16,
        mode,
        layout::palette::TEXT_DIM,
        1,
    );

    let main = &layout.main;
    let panel_w = 360u32.min(main.w.saturating_sub(32));
    let panel_h = 160u32;
    let panel_x = main.x + main.w.saturating_sub(panel_w) / 2;
    let panel_y = main.y + main.h.saturating_sub(panel_h) / 2;

    // Clear the panel area before redrawing
    fb.fill_rect(panel_x, panel_y, panel_w, panel_h, layout::palette::BG);
    draw::rect_outline(
        &mut fb,
        panel_x,
        panel_y,
        panel_w,
        panel_h,
        1,
        layout::palette::SEPARATOR,
    );

    let title = "Welcome to SunlightOS";
    let title_w = font::text_width(title, 2);
    font::draw_str(
        &mut fb,
        panel_x + panel_w.saturating_sub(title_w) / 2,
        panel_y + 24,
        title,
        layout::palette::ACCENT,
        2,
    );

    // Username row
    let user_label_x = panel_x + 32;
    let user_y = panel_y + 72;
    let pass_y = panel_y + 100;
    font::draw_str(
        &mut fb,
        user_label_x,
        user_y,
        "login:    ",
        layout::palette::TEXT_DIM,
        1,
    );
    let uval_x = user_label_x + font::text_width("login:    ", 1);
    tty_draw_line(&mut fb, uval_x, user_y, username, layout::palette::TEXT, 1);
    if !focused_password {
        // Cursor after username text
        let cx = uval_x + username.len() as u32 * 8;
        fb.fill_rect(cx, user_y, 8, 14, layout::palette::ACCENT);
    }

    // Password row
    font::draw_str(
        &mut fb,
        user_label_x,
        pass_y,
        "password: ",
        layout::palette::TEXT_DIM,
        1,
    );
    let pval_x = user_label_x + font::text_width("password: ", 1);
    // Show one '*' per typed character
    let dot_count = password_len.min(20) as u32;
    for i in 0..dot_count {
        font::draw_char(
            &mut fb,
            pval_x + i * 8,
            pass_y,
            b'*',
            layout::palette::TEXT,
            1,
        );
    }
    if focused_password {
        let cx = pval_x + dot_count * 8;
        fb.fill_rect(cx, pass_y, 8, 14, layout::palette::ACCENT);
    }

    // Status message
    if !message.is_empty() {
        let msg_y = panel_y + 130;
        let msg_w = font::text_width(message, 1);
        font::draw_str(
            &mut fb,
            panel_x + panel_w.saturating_sub(msg_w) / 2,
            msg_y,
            message,
            layout::palette::TEXT_DIM,
            1,
        );
    }

    let footer = "Type username, Tab, type password, Enter";
    let footer_w = font::text_width(footer, 1);
    font::draw_str(
        &mut fb,
        fb_width.saturating_sub(footer_w + 16),
        fb_height.saturating_sub(24),
        footer,
        layout::palette::TEXT_DIM,
        1,
    );
}

/// Login wallpaper source. Prefer [`LoginBackground::Argb`] for SIMG v2
/// (decoded by the caller — this crate stays heap-free).
#[derive(Clone, Copy)]
pub enum LoginBackground<'a> {
    /// Legacy TGA type-2 bytes (zero-alloc view).
    Tga(&'a [u8]),
    /// Pre-decoded ARGB8888 top-down buffer (SIMG v2 path).
    Argb {
        width: u32,
        height: u32,
        pixels: &'a [u32],
    },
}

fn paint_login_background(
    fb: &mut framebuffer::Framebuffer,
    layout: &layout::Layout,
    bg: Option<LoginBackground<'_>>,
    overlay_alpha: u8,
) {
    match bg {
        Some(LoginBackground::Tga(tga_data)) => {
            if let Some(img) = tga::TgaImage::parse(tga_data) {
                tga::draw_tga_background(fb, &img, overlay_alpha);
            } else {
                fb.fill_rect(0, 0, fb.width(), fb.height(), layout::palette::BG);
            }
            layout.draw_chrome_overlay(fb);
        }
        Some(LoginBackground::Argb {
            width,
            height,
            pixels,
        }) => {
            tga::draw_argb_background(fb, width, height, pixels, overlay_alpha);
            layout.draw_chrome_overlay(fb);
        }
        None => {
            layout.draw_chrome(fb);
        }
    }
}

/// Render the initial static login screen (before any input).
///
/// SAFETY: `fb_addr` must point to a valid writable framebuffer mapping with
/// the provided dimensions and pitch.
pub unsafe fn render_login_screen(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    bg: Option<LoginBackground<'_>>,
) {
    let mut users = [[0u8; 64]; 6];
    users[0][..4].copy_from_slice(b"root");
    users[1][..4].copy_from_slice(b"user");
    let user_lens = [4usize, 4, 0, 0, 0, 0];
    let user_labels = ["root", "Guest", "Other", "", "", ""];
    let user_icons = [
        LoginUserIcon::User,
        LoginUserIcon::Luggage,
        LoginUserIcon::User,
        LoginUserIcon::User,
        LoginUserIcon::User,
        LoginUserIcon::User,
    ];
    let is_custom = [false, false, true, false, false, false];
    render_login_grid(
        fb_addr,
        fb_width,
        fb_height,
        fb_pitch,
        bg,
        &users,
        &user_lens,
        &user_labels,
        &user_icons,
        &is_custom,
        3,
        0,
        LoginFocus::UserSlot(0),
        "TTY",
        0,
        "Welcome. Please log in.",
    );
}

#[cfg(test)]
mod login_interaction_tests {
    extern crate std;

    use super::*;
    use crate::interaction::{hit_test, Point};

    #[test]
    fn login_hit_testing_uses_render_layout_bounds() {
        let layout = LoginLayout::new(1280, 720, 3, true);
        let password = layout.password;
        assert_eq!(
            hit_test(
                layout.widgets(),
                Point {
                    x: password.x + password.w / 2,
                    y: password.y + password.h / 2,
                },
            ),
            Some(LOGIN_PASSWORD_WIDGET)
        );
        let reboot = layout.reboot;
        assert_eq!(
            hit_test(
                layout.widgets(),
                Point {
                    x: reboot.x + 1,
                    y: reboot.y + 1,
                },
            ),
            Some(LOGIN_REBOOT_WIDGET)
        );
    }

    #[test]
    fn locked_login_controls_are_not_pointer_targets() {
        let layout = LoginLayout::new(1280, 720, 3, false);
        let password = layout.password;
        assert_eq!(
            hit_test(
                layout.widgets(),
                Point {
                    x: password.x,
                    y: password.y,
                },
            ),
            None
        );
    }

    #[test]
    fn terminal_tab_and_add_hit_targets_use_rendered_bounds() {
        let labels = [TabLabel::empty(), TabLabel::empty()];
        let layout = TerminalTabLayout::new(1280, 720, &labels, 0, true);
        let second = layout.tab(1).unwrap();
        assert_eq!(
            hit_test(
                layout.widgets(),
                Point {
                    x: second.x + second.w / 2,
                    y: second.y + second.h / 2,
                },
            ),
            Some(terminal_tab_widget(1))
        );
        assert_eq!(
            hit_test(
                layout.widgets(),
                Point {
                    x: layout.add_tab.x + layout.add_tab.w / 2,
                    y: layout.add_tab.y + layout.add_tab.h / 2,
                },
            ),
            Some(TERMINAL_ADD_TAB_WIDGET)
        );
    }

    #[test]
    fn terminal_add_target_is_disabled_at_capacity() {
        let labels = [TabLabel::empty(); MAX_TERMINAL_TABS];
        let layout = TerminalTabLayout::new(1280, 720, &labels, 0, false);
        assert_eq!(
            hit_test(
                layout.widgets(),
                Point {
                    x: layout.add_tab.x + 1,
                    y: layout.add_tab.y + 1,
                },
            ),
            None
        );
    }

    #[test]
    fn controlled_native_geometry_probe() {
        let (cols, rows) = terminal_dims(1280, 800);
        std::println!("rows={rows} cols={cols}");
        assert!(cols > 80);
        assert!(rows > 25);
    }
}
