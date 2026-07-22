//! About This Computer and About SunlightOS pages.

use core::fmt::Write;
use sun_font::{self, FontRole, TextStyle, Typography};
use sunlight_ui::{
    image::TgaImage,
    widgets::{Button, ButtonState, ProgressBar},
    Canvas, Color, Event, Point, Rect, Theme,
};

use crate::sysinfo::{ComponentEntry, FixedStr, SystemInfoSnapshot, CORE_COMPONENTS};

const NA: &str = "Not available";
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;
const SCROLL_STEP: i32 = 24;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AboutAction {
    None,
    Back,
    Refresh,
    Copy,
    NavigateComputer,
    NavigateOs,
}

pub struct AboutPageState {
    pub scroll_y: i32,
    pub max_scroll: i32,
    pub status: FixedStr<64>,
    pub show_uname: bool,
}

impl AboutPageState {
    pub fn new() -> Self {
        Self {
            scroll_y: 0,
            max_scroll: 0,
            status: FixedStr::empty(),
            show_uname: false,
        }
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status.set(msg);
    }

    pub fn clear_status(&mut self) {
        self.status.clear();
    }
}

// ── Layout helpers ───────────────────────────────────────────────────────────

fn content_rect(win_w: u32, win_h: u32) -> Rect {
    Rect::new(10, 10, win_w - 20, win_h - 20)
}

fn action_bar_y(win_h: u32) -> i32 {
    win_h as i32 - 40
}

fn btn_back(win_h: u32) -> Rect {
    Rect::new(18, action_bar_y(win_h), 72, 26)
}

fn btn_refresh(win_w: u32, win_h: u32) -> Rect {
    Rect::new(win_w as i32 / 2 - 90, action_bar_y(win_h), 80, 26)
}

fn btn_copy(win_w: u32, win_h: u32) -> Rect {
    Rect::new(win_w as i32 / 2 - 2, action_bar_y(win_h), 108, 26)
}

fn btn_nav(win_w: u32, win_h: u32) -> Rect {
    Rect::new(win_w as i32 - 138, action_bar_y(win_h), 120, 26)
}

/// Same MiniType path as the rest of Control Panel (`Typography::UI_MEDIUM`).
fn draw_button(canvas: &mut Canvas, theme: &Theme, rect: Rect, label: &str, primary: bool) {
    let mut b = if primary {
        Button::new(rect, label)
    } else {
        Button::secondary(rect, label)
    };
    b.state = ButtonState::Normal;
    b.with_font(&Typography::UI_MEDIUM).draw(canvas, theme);
}

fn draw_text(canvas: &mut Canvas, x: i32, y: i32, h: u32, text: &str, color: Color, role: FontRole) {
    sun_font::draw_text_vcenter(canvas, text, x, y, h, &TextStyle::new(role, color));
}

fn ellipsize<'a>(text: &'a str, max_chars: usize, scratch: &'a mut FixedStr<96>) -> &'a str {
    if text.chars().count() <= max_chars {
        return text;
    }
    scratch.clear();
    for (i, ch) in text.chars().enumerate() {
        if i + 1 >= max_chars {
            scratch.push_str("…");
            break;
        }
        let mut buf = [0u8; 4];
        scratch.push_str(ch.encode_utf8(&mut buf));
    }
    scratch.as_str()
}

fn info_card(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    title: &str,
    lines: &[(&str, &str)],
) {
    canvas.fill_material(rect, sunlight_ui::Material::card(theme));
    draw_text(
        canvas,
        rect.x + 10,
        rect.y + 6,
        16,
        title,
        theme.accent,
        FontRole::UiMedium,
    );
    let mut y = rect.y + 28;
    let mut scratch = FixedStr::<96>::empty();
    for (label, value) in lines {
        draw_text(
            canvas,
            rect.x + 10,
            y,
            14,
            label,
            theme.text_dim,
            FontRole::UiSmall,
        );
        let shown = ellipsize(value, 28, &mut scratch);
        draw_text(
            canvas,
            rect.x + 10,
            y + 14,
            14,
            shown,
            theme.text,
            FontRole::UiSmall,
        );
        y += 32;
        if y + 28 > rect.bottom() {
            break;
        }
    }
}

fn detail_row(
    canvas: &mut Canvas,
    theme: &Theme,
    x: i32,
    y: i32,
    w: u32,
    label: &str,
    value: &str,
) {
    draw_text(canvas, x, y, 14, label, theme.text_dim, FontRole::UiSmall);
    let mut scratch = FixedStr::<96>::empty();
    let shown = ellipsize(value, 42, &mut scratch);
    draw_text(
        canvas,
        x + 120,
        y,
        14,
        shown,
        theme.text,
        FontRole::UiSmall,
    );
    let _ = w;
}

fn or_na(s: &str) -> &str {
    if s.is_empty() {
        NA
    } else {
        s
    }
}

fn opt_u32(v: Option<u32>, buf: &mut FixedStr<16>) -> &str {
    match v {
        Some(n) => {
            buf.clear();
            let _ = write!(buf, "{}", n);
            buf.as_str()
        }
        None => NA,
    }
}

// ── About This Computer ──────────────────────────────────────────────────────

pub fn draw_computer_page(
    canvas: &mut Canvas,
    theme: &Theme,
    win_w: u32,
    win_h: u32,
    info: &SystemInfoSnapshot,
    state: &AboutPageState,
    icon_computer: Option<TgaImage>,
) {
    canvas.fill_rect(Rect::new(0, 0, win_w, win_h), theme.bg);
    let content = content_rect(win_w, win_h);
    canvas.fill_rounded_rect(content, 10, theme.panel_alt);
    canvas.stroke_rounded_rect(content, 10, 1, theme.border);

    // Clip drawing to content above the action bar.
    let view_bottom = action_bar_y(win_h) - 6;
    let scroll = state.scroll_y;

    // Hero
    let hero_y = content.y + 8 - scroll;
    if let Some(icon) = icon_computer {
        canvas.draw_tga_icon(&icon, Rect::new(content.x + 14, hero_y, 40, 40));
    } else {
        canvas.fill_rounded_rect(
            Rect::new(content.x + 14, hero_y, 40, 40),
            8,
            theme.accent.darken(120),
        );
    }
    draw_text(
        canvas,
        content.x + 66,
        hero_y,
        18,
        or_na(info.hostname.as_str()),
        theme.text,
        FontRole::UiTitle,
    );
    let mut sub = FixedStr::<96>::empty();
    let _ = write!(
        &mut sub,
        "{} · {} · ",
        or_na(info.platform.as_str()),
        or_na(info.architecture.as_str())
    );
    match info.cpu_cores {
        Some(n) => {
            let _ = write!(&mut sub, "{} cores", n);
        }
        None => {
            let _ = write!(&mut sub, "cores {}", NA);
        }
    }
    draw_text(
        canvas,
        content.x + 66,
        hero_y + 20,
        14,
        sub.as_str(),
        theme.text_dim,
        FontRole::UiSmall,
    );

    // Info cards 2×2
    let card_w = (content.w - 36) / 2;
    let card_h = 118u32;
    let gap = 8i32;
    let grid_y = hero_y + 52;
    let c1 = Rect::new(content.x + 12, grid_y, card_w, card_h);
    let c2 = Rect::new(content.x + 12 + card_w as i32 + gap, grid_y, card_w, card_h);
    let c3 = Rect::new(content.x + 12, grid_y + card_h as i32 + gap, card_w, card_h);
    let c4 = Rect::new(
        content.x + 12 + card_w as i32 + gap,
        grid_y + card_h as i32 + gap,
        card_w,
        card_h,
    );

    let mut scratch_a = FixedStr::<32>::empty();
    let mut scratch_b = FixedStr::<32>::empty();
    let mut scratch_c = FixedStr::<32>::empty();
    let mut cores_buf = FixedStr::<16>::empty();

    let cpu_lines = [
        ("Model", or_na(info.cpu_model.as_str())),
        ("Cores", opt_u32(info.cpu_cores, &mut cores_buf)),
        ("Architecture", or_na(info.architecture.as_str())),
    ];
    if c1.y < view_bottom {
        info_card(canvas, theme, c1, "Processor", &cpu_lines);
    }

    if let Some(v) = info.mem_total_kb {
        SystemInfoSnapshot::format_kb_human(v, &mut scratch_a);
    } else {
        scratch_a.set(NA);
    }
    if let Some(v) = info.mem_used_kb {
        SystemInfoSnapshot::format_kb_human(v, &mut scratch_b);
    } else {
        scratch_b.set(NA);
    }
    if let Some(v) = info.mem_available_kb {
        SystemInfoSnapshot::format_kb_human(v, &mut scratch_c);
    } else {
        scratch_c.set(NA);
    }
    let mem_lines = [
        ("Total", scratch_a.as_str()),
        ("Used", scratch_b.as_str()),
        ("Available", scratch_c.as_str()),
    ];
    if c2.y < view_bottom {
        info_card(canvas, theme, c2, "Memory", &mem_lines);
        // Usage bar inside memory card
        let bar = Rect::new(c2.x + 10, c2.bottom() - 18, c2.w - 20, 8);
        ProgressBar::new(bar, info.mem_usage_ratio())
            .auto_color(theme)
            .draw(canvas, theme);
    }

    let zram_state = match info.zram_enabled {
        Some(true) => "Enabled",
        Some(false) => "Disabled",
        None => NA,
    };
    let mut zcap = FixedStr::<32>::empty();
    let mut zused = FixedStr::<32>::empty();
    let mut zcomp = FixedStr::<32>::empty();
    match info.zram_capacity_kb {
        Some(v) => SystemInfoSnapshot::format_kb_human(v, &mut zcap),
        None => zcap.set(NA),
    }
    match info.zram_used_kb {
        Some(v) => SystemInfoSnapshot::format_kb_human(v, &mut zused),
        None => zused.set(NA),
    }
    let zcomp_str = match info.zram_compressed_kb {
        Some(v) if v > 0 => {
            SystemInfoSnapshot::format_kb_human(v, &mut zcomp);
            zcomp.as_str()
        }
        _ => NA,
    };
    let zram_lines = [
        ("State", zram_state),
        ("Capacity", zcap.as_str()),
        ("Used", zused.as_str()),
        ("Compressed", zcomp_str),
    ];
    if c3.y < view_bottom {
        info_card(canvas, theme, c3, "ZRAM / Swap", &zram_lines);
    }

    let mut res_buf = FixedStr::<32>::empty();
    match (info.display_w, info.display_h) {
        (Some(w), Some(h)) => {
            let _ = write!(&mut res_buf, "{} × {}", w, h);
        }
        _ => res_buf.set(NA),
    }
    let gfx_lines = [
        ("Adapter", or_na(info.graphics_adapter.as_str())),
        ("Backend", or_na(info.graphics_backend.as_str())),
        ("Resolution", res_buf.as_str()),
    ];
    if c4.y < view_bottom {
        info_card(canvas, theme, c4, "Graphics", &gfx_lines);
    }

    // Computer details
    let details_y = c3.bottom() + 12;
    if details_y < view_bottom {
        let details = Rect::new(
            content.x + 12,
            details_y,
            content.w - 24,
            110,
        );
        canvas.fill_rounded_rect(details, 8, theme.panel);
        canvas.stroke_rounded_rect(details, 8, 1, theme.border);
        draw_text(
            canvas,
            details.x + 10,
            details.y + 6,
            16,
            "Computer details",
            theme.accent,
            FontRole::UiMedium,
        );
        let mut up = FixedStr::<48>::empty();
        match info.uptime_secs {
            Some(u) => SystemInfoSnapshot::format_uptime(u, &mut up),
            None => up.set(NA),
        }
        let mut ksum = FixedStr::<64>::empty();
        let _ = write!(
            &mut ksum,
            "{} {}",
            or_na(info.kernel_name.as_str()),
            or_na(info.kernel_release.as_str())
        );
        let rows = [
            ("Computer name", or_na(info.hostname.as_str())),
            ("Platform", or_na(info.platform.as_str())),
            ("Architecture", or_na(info.architecture.as_str())),
            ("Uptime", up.as_str()),
            ("Kernel", ksum.as_str()),
        ];
        let mut ry = details.y + 28;
        for (l, v) in rows {
            detail_row(canvas, theme, details.x + 10, ry, details.w - 20, l, v);
            ry += 15;
        }
    }

    // Status
    if !state.status.is_empty() {
        draw_text(
            canvas,
            content.x + 14,
            view_bottom - 16,
            14,
            state.status.as_str(),
            theme.text_dim,
            FontRole::UiSmall,
        );
    }

    // Action bar background
    canvas.fill_rect(
        Rect::new(0, action_bar_y(win_h) - 4, win_w, 44),
        theme.bg,
    );
    draw_button(canvas, theme, btn_back(win_h), "Back", false);
    draw_button(canvas, theme, btn_refresh(win_w, win_h), "Refresh", false);
    draw_button(canvas, theme, btn_copy(win_w, win_h), "Copy Summary", false);
    draw_button(canvas, theme, btn_nav(win_w, win_h), "About OS", true);
}

pub fn computer_content_height(win_w: u32) -> i32 {
    let content = content_rect(win_w, 400);
    let card_h = 118i32;
    // hero 52 + 2 rows cards + gap + details 110 + padding
    8 + 52 + card_h + 8 + card_h + 12 + 110 + 20 + content.y
}

pub fn update_computer_page(
    event: Event,
    win_w: u32,
    win_h: u32,
    state: &mut AboutPageState,
) -> AboutAction {
    handle_scroll(event, state, computer_content_height(win_w), win_h);
    if let Event::Click { x, y } = event {
        let pt = Point::new(x, y);
        if btn_back(win_h).contains(pt) {
            return AboutAction::Back;
        }
        if btn_refresh(win_w, win_h).contains(pt) {
            return AboutAction::Refresh;
        }
        if btn_copy(win_w, win_h).contains(pt) {
            return AboutAction::Copy;
        }
        if btn_nav(win_w, win_h).contains(pt) {
            return AboutAction::NavigateOs;
        }
    }
    AboutAction::None
}

// ── About SunlightOS ─────────────────────────────────────────────────────────

pub fn draw_os_page(
    canvas: &mut Canvas,
    theme: &Theme,
    win_w: u32,
    win_h: u32,
    info: &SystemInfoSnapshot,
    state: &AboutPageState,
    logo: Option<TgaImage>,
) {
    canvas.fill_rect(Rect::new(0, 0, win_w, win_h), theme.bg);
    let content = content_rect(win_w, win_h);
    canvas.fill_rounded_rect(content, 10, theme.panel_alt);
    canvas.stroke_rounded_rect(content, 10, 1, theme.border);

    let view_bottom = action_bar_y(win_h) - 6;
    let scroll = state.scroll_y;
    let mut y = content.y + 10 - scroll;

    // Hero with logo
    if let Some(logo) = logo {
        let lw = logo.width.min(128);
        let lh = logo.height.min(48);
        let lx = content.x + (content.w as i32 - lw as i32) / 2;
        // Subtle warm glow under the logo
        canvas.fill_rounded_rect(
            Rect::new(lx - 6, y - 4, lw + 12, lh + 8),
            10,
            Color::rgba(255, 160, 64, 0x28),
        );
        canvas.draw_tga_icon(&logo, Rect::new(lx, y, lw, lh));
        y += lh as i32 + 8;
    } else {
        draw_text(
            canvas,
            content.x + 16,
            y,
            20,
            "SunlightOS",
            theme.accent,
            FontRole::UiTitle,
        );
        y += 28;
    }

    draw_text(
        canvas,
        content.x + 16,
        y,
        18,
        or_na(info.os_name.as_str()),
        theme.text,
        FontRole::UiTitle,
    );
    y += 20;
    let mut ver = FixedStr::<80>::empty();
    let _ = write!(
        &mut ver,
        "Version {} · Build {}",
        or_na(info.os_version.as_str()),
        or_na(info.os_build.as_str())
    );
    draw_text(
        canvas,
        content.x + 16,
        y,
        14,
        ver.as_str(),
        theme.accent,
        FontRole::UiSmall,
    );
    y += 18;
    draw_text(
        canvas,
        content.x + 16,
        y,
        14,
        or_na(info.os_description.as_str()),
        theme.text_dim,
        FontRole::UiSmall,
    );
    y += 24;

    // Section: Operating System
    y = draw_section(
        canvas,
        theme,
        content.x + 12,
        y,
        content.w - 24,
        "Operating System",
        &[
            ("Edition", or_na(info.os_edition.as_str())),
            ("Version", or_na(info.os_version.as_str())),
            ("Build", or_na(info.os_build.as_str())),
            ("Architecture", or_na(info.architecture.as_str())),
            ("Channel", or_na(info.os_channel.as_str())),
        ],
        view_bottom,
    );

    // Section: Kernel
    let mut cores = FixedStr::<16>::empty();
    y = draw_section(
        canvas,
        theme,
        content.x + 12,
        y,
        content.w - 24,
        "SunlightX Kernel",
        &[
            ("Name", or_na(info.kernel_name.as_str())),
            ("Release", or_na(info.kernel_release.as_str())),
            ("Build", or_na(info.kernel_version.as_str())),
            ("Architecture", or_na(info.kernel_arch.as_str())),
            ("Logical CPUs", opt_u32(info.cpu_cores, &mut cores)),
        ],
        view_bottom,
    );

    // Section: Components (data-driven)
    y = draw_components_section(
        canvas,
        theme,
        content.x + 12,
        y,
        content.w - 24,
        CORE_COMPONENTS,
        view_bottom,
    );

    // Technical kernel string (subdued / expandable)
    if y < view_bottom {
        let tech = Rect::new(content.x + 12, y, content.w - 24, if state.show_uname { 52 } else { 28 });
        canvas.fill_rounded_rect(tech, 6, theme.panel);
        canvas.stroke_rounded_rect(tech, 6, 1, theme.border);
        let label = if state.show_uname {
            "Technical kernel string  (click to hide)"
        } else {
            "Technical kernel string  (click to show)"
        };
        draw_text(
            canvas,
            tech.x + 10,
            tech.y + 6,
            14,
            label,
            theme.text_dim,
            FontRole::UiSmall,
        );
        if state.show_uname {
            let mut scratch = FixedStr::<96>::empty();
            let shown = ellipsize(or_na(info.uname_string.as_str()), 56, &mut scratch);
            draw_text(
                canvas,
                tech.x + 10,
                tech.y + 24,
                14,
                shown,
                theme.text,
                FontRole::UiSmall,
            );
        }
    }

    if !state.status.is_empty() {
        draw_text(
            canvas,
            content.x + 14,
            view_bottom - 16,
            14,
            state.status.as_str(),
            theme.text_dim,
            FontRole::UiSmall,
        );
    }

    canvas.fill_rect(
        Rect::new(0, action_bar_y(win_h) - 4, win_w, 44),
        theme.bg,
    );
    draw_button(canvas, theme, btn_back(win_h), "Back", false);
    draw_button(canvas, theme, btn_refresh(win_w, win_h), "Refresh", false);
    draw_button(
        canvas,
        theme,
        btn_copy(win_w, win_h),
        "Copy Report",
        false,
    );
    draw_button(
        canvas,
        theme,
        btn_nav(win_w, win_h),
        "This Computer",
        true,
    );
}

fn draw_section(
    canvas: &mut Canvas,
    theme: &Theme,
    x: i32,
    y: i32,
    w: u32,
    title: &str,
    rows: &[(&str, &str)],
    view_bottom: i32,
) -> i32 {
    let h = 24 + rows.len() as i32 * 16 + 8;
    if y >= view_bottom {
        return y + h + 8;
    }
    let rect = Rect::new(x, y, w, h as u32);
    canvas.fill_rounded_rect(rect, 8, theme.panel);
    canvas.stroke_rounded_rect(rect, 8, 1, theme.border);
    draw_text(
        canvas,
        x + 10,
        y + 6,
        16,
        title,
        theme.accent,
        FontRole::UiMedium,
    );
    let mut ry = y + 26;
    for (l, v) in rows {
        if ry < view_bottom {
            detail_row(canvas, theme, x + 10, ry, w - 20, l, v);
        }
        ry += 16;
    }
    y + h + 8
}

fn draw_components_section(
    canvas: &mut Canvas,
    theme: &Theme,
    x: i32,
    y: i32,
    w: u32,
    components: &[ComponentEntry],
    view_bottom: i32,
) -> i32 {
    let h = 24 + components.len() as i32 * 16 + 8;
    if y >= view_bottom {
        return y + h + 8;
    }
    let rect = Rect::new(x, y, w, h as u32);
    canvas.fill_rounded_rect(rect, 8, theme.panel);
    canvas.stroke_rounded_rect(rect, 8, 1, theme.border);
    draw_text(
        canvas,
        x + 10,
        y + 6,
        16,
        "Desktop and Core Components",
        theme.accent,
        FontRole::UiMedium,
    );
    let mut ry = y + 26;
    for c in components {
        let ver = c.version.unwrap_or(NA);
        if ry < view_bottom {
            detail_row(canvas, theme, x + 10, ry, w - 20, c.name, ver);
        }
        ry += 16;
    }
    y + h + 8
}

pub fn os_content_height(win_w: u32) -> i32 {
    let _ = win_w;
    // Approximate full content length for scroll clamp.
    10 + 56 + 60 + (5 * 16 + 32) * 2 + (4 * 16 + 32) + 60 + 40
}

fn tech_row_rect(win_w: u32, win_h: u32, scroll_y: i32, expanded: bool) -> Rect {
    let content = content_rect(win_w, win_h);
    // Approximate y matches draw_os_page: hero + sections stack.
    let y = content.y + 10 - scroll_y + 56 + 60 + (5 * 16 + 32) * 2 + (4 * 16 + 32) + 8;
    Rect::new(
        content.x + 12,
        y,
        content.w - 24,
        if expanded { 52 } else { 28 },
    )
}

pub fn update_os_page(
    event: Event,
    win_w: u32,
    win_h: u32,
    state: &mut AboutPageState,
) -> AboutAction {
    handle_scroll(event, state, os_content_height(win_w), win_h);
    if let Event::Click { x, y } = event {
        let pt = Point::new(x, y);
        if btn_back(win_h).contains(pt) {
            return AboutAction::Back;
        }
        if btn_refresh(win_w, win_h).contains(pt) {
            return AboutAction::Refresh;
        }
        if btn_copy(win_w, win_h).contains(pt) {
            return AboutAction::Copy;
        }
        if btn_nav(win_w, win_h).contains(pt) {
            return AboutAction::NavigateComputer;
        }
        if tech_row_rect(win_w, win_h, state.scroll_y, state.show_uname).contains(pt) {
            state.show_uname = !state.show_uname;
            return AboutAction::None;
        }
    }
    AboutAction::None
}

fn handle_scroll(event: Event, state: &mut AboutPageState, content_h: i32, win_h: u32) {
    let view_h = action_bar_y(win_h) - 20;
    state.max_scroll = (content_h - view_h).max(0);
    state.scroll_y = state.scroll_y.clamp(0, state.max_scroll);
    if let Event::KeyPress {
        keycode,
        pressed: true,
        ..
    } = event
    {
        match keycode {
            KEY_UP => state.scroll_y = (state.scroll_y - SCROLL_STEP).max(0),
            KEY_DOWN => state.scroll_y = (state.scroll_y + SCROLL_STEP).min(state.max_scroll),
            KEY_PGUP => state.scroll_y = (state.scroll_y - SCROLL_STEP * 4).max(0),
            KEY_PGDN => state.scroll_y = (state.scroll_y + SCROLL_STEP * 4).min(state.max_scroll),
            _ => {}
        }
    }
}
