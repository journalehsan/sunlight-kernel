//! SunlightOS graphical boot TUI
//!
//! Pure Rust, no_std, no heap, no floats.
//! Renders directly to Limine framebuffer.

#![no_std]
#![allow(dead_code)]

mod draw;
pub mod fmt;
mod font;
mod framebuffer;
pub mod layout;
mod modes;
mod splash;

pub use layout::ANSI_COLORS;
pub use modes::debug::LogBuffer;
pub use splash::{BootMode, SplashScreen};

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

    // Header
    font::draw_str(&mut fb, 16, 16, "*", layout::palette::ACCENT, 1);
    font::draw_str(&mut fb, 32, 16, "SunlightOS", layout::palette::TEXT, 1);
    let mode_label = "TTY";
    let mode_w = font::text_width(mode_label, 1);
    font::draw_str(
        &mut fb,
        fb_width.saturating_sub(mode_w + 16),
        16,
        mode_label,
        layout::palette::ACCENT_DIM,
        1,
    );

    // Tab bar — 26px high strip immediately below the header separator
    const TAB_H: u32 = 26;
    let tab_y = layout.main.y;
    fb.fill_rect(0, tab_y, fb_width, TAB_H, layout::palette::SURFACE);
    fb.hline(0, tab_y + TAB_H, fb_width, layout::palette::SEPARATOR);

    let mut tx = 8u32;
    for i in 0..tab_count.min(10) {
        let is_active = i == active_tab;
        let fg = if is_active {
            layout::palette::ACCENT
        } else {
            layout::palette::TEXT_DIM
        };
        let tab_text = " shell ";
        let tw = font::text_width(tab_text, 1);
        fb.fill_rect(tx, tab_y + 3, tw + 2, TAB_H - 6, layout::palette::BG);
        font::draw_str(&mut fb, tx + 1, tab_y + 5, tab_text, fg, 1);
        if is_active {
            // Orange underline on active tab
            fb.hline(tx, tab_y + TAB_H - 2, tw + 2, layout::palette::ACCENT);
        }
        tx += tw + 8;
    }

    // Content area — from below tab bar to above footer
    const CHAR_H: u32 = 18; // 16px glyph + 2px leading
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
) {
    let mut fb = framebuffer::Framebuffer::from_limine(fb_addr, fb_width, fb_height, fb_pitch);
    let layout = layout::Layout::new(fb_width, fb_height);
    layout.draw_chrome(&mut fb);

    // Header: logo left, clock right, mode label left of the clock
    font::draw_str(&mut fb, 16, 16, "*", layout::palette::ACCENT, 1);
    font::draw_str(&mut fb, 32, 16, "SunlightOS", layout::palette::TEXT, 1);
    let clock_w = clock.len() as u32 * 8;
    let clock_x = fb_width.saturating_sub(clock_w + 16);
    tty_draw_line(&mut fb, clock_x, 16, clock, layout::palette::TEXT, 1);
    let mode_label = "TTY";
    let mode_w = font::text_width(mode_label, 1);
    font::draw_str(
        &mut fb,
        clock_x.saturating_sub(mode_w + 24),
        16,
        mode_label,
        layout::palette::ACCENT_DIM,
        1,
    );

    // Tab bar
    const TAB_H: u32 = 26;
    let tab_y = layout.main.y;
    fb.fill_rect(0, tab_y, fb_width, TAB_H, layout::palette::SURFACE);
    fb.hline(0, tab_y + TAB_H, fb_width, layout::palette::SEPARATOR);

    let mut tx = 8u32;
    for (i, label) in tab_labels.iter().take(10).enumerate() {
        let is_active = i == active_tab;
        let fg = if is_active {
            layout::palette::ACCENT
        } else {
            layout::palette::TEXT_DIM
        };
        // Compose the visible label: " NAME " (uppercase title), with a
        // trailing "*" when a background tab still has a running app.
        let mut buf = [b' '; 28];
        let mut n = 1usize; // leading space
        let name = if label.name_len == 0 {
            &b"SHELL"[..]
        } else {
            &label.name[..label.name_len.min(24)]
        };
        for &b in name {
            if n < buf.len() - 2 {
                // ASCII uppercase so titles read "TOP", "CURL", etc.
                buf[n] = if b.is_ascii_lowercase() { b - 32 } else { b };
                n += 1;
            }
        }
        if label.running && !is_active && n < buf.len() - 1 {
            buf[n] = b'*';
            n += 1;
        }
        buf[n] = b' '; // trailing space
        n += 1;
        let tab_text = core::str::from_utf8(&buf[..n]).unwrap_or(" SHELL ");
        let tw = font::text_width(tab_text, 1);
        fb.fill_rect(tx, tab_y + 3, tw + 2, TAB_H - 6, layout::palette::BG);
        font::draw_str(&mut fb, tx + 1, tab_y + 5, tab_text, fg, 1);
        if is_active {
            fb.hline(tx, tab_y + TAB_H - 2, tw + 2, layout::palette::ACCENT);
        }
        tx += tw + 8;
    }

    // Content area: render the grid
    const CHAR_H: u32 = 18;
    const MARGIN: u32 = 16;
    let content_y = tab_y + TAB_H + 4;
    let avail_h = layout.footer.y.saturating_sub(content_y + 4);
    let max_visible = (avail_h / CHAR_H) as usize;

    // Only show the last `max_visible` rows
    let start_row = if rows > max_visible {
        rows - max_visible
    } else {
        0
    };
    for row in start_row..rows {
        let screen_row = row - start_row;
        let y = content_y + (screen_row as u32) * CHAR_H;

        for col in 0..cols {
            let cell_idx = row * cols + col;
            if cell_idx >= cells.len() {
                break;
            }
            let cell = cells[cell_idx];
            let x = MARGIN + (col as u32) * 8;

            // Draw background
            fb.fill_rect(x, y, 8, 16, cell.bg);

            // Draw character if not space
            if cell.ch != b' ' && cell.ch >= 0x20 && cell.ch <= 0x7E {
                font::draw_char(&mut fb, x, y, cell.ch, cell.fg, 1);
            }
        }
    }

    // Draw the grid cursor only when the caller is not rendering a separate
    // prompt/input area. The TTY shell keeps the live prompt in the footer.
    if prompt.is_empty() && input_line.is_empty() && cursor_row >= start_row && cursor_row < rows {
        let screen_row = cursor_row - start_row;
        let y = content_y + (screen_row as u32) * CHAR_H;
        let x = MARGIN + (cursor_col as u32) * 8;

        // Inverted: swap fg/bg for cursor cell
        let cell_idx = cursor_row * cols + cursor_col.min(cols - 1);
        if cell_idx < cells.len() {
            let cell = cells[cell_idx];
            fb.fill_rect(x, y, 8, 16, cell.fg); // fg becomes bg
            if cell.ch != b' ' && cell.ch >= 0x20 && cell.ch <= 0x7E {
                font::draw_char(&mut fb, x, y, cell.ch, cell.bg, 1); // bg becomes fg
            }
        } else {
            // Empty cell: just draw inverted space
            fb.fill_rect(x, y, 8, 16, layout::palette::TEXT);
        }
    }

    // Footer: prompt + input + command cursor
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
    let cursor_x = MARGIN + (prompt.len() + input_line.len()) as u32 * 8;
    fb.fill_rect(cursor_x, footer_text_y, 8, 16, layout::palette::ACCENT);
}

/// Render the login screen with live state (username typed, password dots, cursor, message).
///
/// SAFETY: `fb_addr` must point to a valid writable framebuffer mapping.
pub unsafe fn render_login_dynamic(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    username: &[u8],
    password_len: usize,
    focused_password: bool,
    message: &str,
) {
    let mut fb = framebuffer::Framebuffer::from_limine(fb_addr, fb_width, fb_height, fb_pitch);
    let layout = layout::Layout::new(fb_width, fb_height);
    layout.draw_chrome(&mut fb);

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

/// Render the initial static login screen (before any input).
///
/// SAFETY: `fb_addr` must point to a valid writable framebuffer mapping with
/// the provided dimensions and pitch.
pub unsafe fn render_login_screen(fb_addr: *mut u32, fb_width: u32, fb_height: u32, fb_pitch: u32) {
    // Delegate to the dynamic renderer with the pre-filled "root" username
    // (matching the static display users see on first boot) and focus on password.
    unsafe {
        render_login_dynamic(
            fb_addr,
            fb_width,
            fb_height,
            fb_pitch,
            b"root",
            0,
            true,
            "Welcome. Please log in.",
        );
    }
}
