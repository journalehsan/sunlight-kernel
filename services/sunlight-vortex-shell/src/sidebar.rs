//! Vortex's bounded system Sidebar composition.
//!
//! Rendering only consumes small view models.  The shell remains responsible
//! for telemetry ownership and URL/application activation.

use sun_font::{draw_text_vcenter, measure_text, FontRole, TextStyle};
use sunlight_ui::{
    image::TgaImage,
    widgets::{ArticleListItem, MetricBar, UnitToggle, WidgetCard},
    Canvas, Event, Material, Point, Rect, SurfaceRole, Theme,
};

const SIDEBAR_MARGIN: i32 = 8;
const SIDEBAR_PREFERRED_W: u32 = 384;
const SIDEBAR_MIN_W: u32 = 220;
const SIDEBAR_MAX_W: u32 = 420;
const HEADER_H: u32 = 40;
const CONTENT_PAD: i32 = 12;
const CARD_GAP: i32 = 10;
const WEATHER_CARD_H: u32 = 126;
const MONITOR_CARD_H: u32 = 170;
const NEWS_ROW_H: u32 = 48;
const NEWS_CARD_H: u32 = 36 + NEWS_ROW_H * NEWS_PREVIEW.len() as u32 + 12;
const SCROLL_STEP: u32 = 64;

const KEY_TAB: u8 = 0x0F;
const KEY_ENTER: u8 = 0x1C;
const KEY_SPACE: u8 = 0x39;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;

/// Static weather data. A future live provider replaces this construction,
/// not the card renderer or unit-selection state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct WeatherViewData {
    pub location: &'static str,
    pub condition: &'static str,
    pub temperature_c: i16,
    pub preview: bool,
}

pub(crate) const WEATHER_PREVIEW: WeatherViewData = WeatherViewData {
    location: "Tehran",
    condition: "Clear skies",
    temperature_c: 24,
    preview: true,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemperatureUnit {
    Celsius,
    Fahrenheit,
}

impl TemperatureUnit {
    const fn index(self) -> usize {
        match self {
            Self::Celsius => 0,
            Self::Fahrenheit => 1,
        }
    }
}

pub(crate) fn temperature_for_unit(celsius: i16, unit: TemperatureUnit) -> i16 {
    match unit {
        TemperatureUnit::Celsius => celsius,
        TemperatureUnit::Fahrenheit => celsius.saturating_mul(9) / 5 + 32,
    }
}

/// Bounded article data for the preview provider.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct NewsArticleViewData {
    pub category: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub url: &'static str,
}

pub(crate) const NEWS_PREVIEW: [NewsArticleViewData; 3] = [
    NewsArticleViewData {
        category: "Project source",
        title: "SunlightOS kernel repository",
        summary: "Browse the project source and issue tracker.",
        url: "https://github.com/journalehsan/sunlight-kernel",
    },
    NewsArticleViewData {
        category: "Boot stack",
        title: "Limine bootloader",
        summary: "Reference for the boot protocol used by SunlightOS.",
        url: "https://limine-bootloader.org/",
    },
    NewsArticleViewData {
        category: "Kernel design",
        title: "BORE scheduler notes",
        summary: "Read the scheduler reference linked by project docs.",
        url: "https://github.com/firelzrd/bore-scheduler",
    },
];

/// Sanitized telemetry values already expressed in the system's established
/// aggregate-per-core-normalized basis-point semantics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SystemMonitorViewData {
    pub cpu_bp: u16,
    pub ram_bp: u16,
    pub used_ram_kb: u64,
    pub total_ram_kb: u64,
    pub task_count: usize,
    pub zram_orig_kb: u64,
    pub zram_comp_kb: u64,
}

impl SystemMonitorViewData {
    pub(crate) fn from_values(
        cpu_bp: u16,
        used_ram_kb: u64,
        total_ram_kb: u64,
        task_count: usize,
        zram_orig_kb: u64,
        zram_comp_kb: u64,
    ) -> Option<Self> {
        if total_ram_kb == 0 || used_ram_kb > total_ram_kb {
            return None;
        }
        let ram_bp = used_ram_kb
            .saturating_mul(10_000)
            .checked_div(total_ram_kb)
            .unwrap_or(0)
            .min(10_000) as u16;
        Some(Self {
            cpu_bp: cpu_bp.min(10_000),
            ram_bp,
            used_ram_kb,
            total_ram_kb,
            task_count,
            zram_orig_kb,
            zram_comp_kb,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarFocus {
    Unit,
    Article(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarHit {
    Unit(usize),
    Article(usize),
}

#[derive(Clone, Copy)]
struct SidebarLayout {
    panel: Rect,
    content: Rect,
    weather: Rect,
    monitor: Rect,
    news: Rect,
    unit_toggle: Rect,
    article_rows: [Rect; NEWS_PREVIEW.len()],
    max_scroll: u32,
}

impl SidebarLayout {
    fn compute(screen_w: u32, top: i32, bottom: i32, scroll: u32) -> Self {
        let available_w = screen_w.saturating_sub((SIDEBAR_MARGIN as u32).saturating_mul(2));
        let small_cap = available_w.saturating_mul(3) / 4;
        let width = SIDEBAR_PREFERRED_W
            .min(SIDEBAR_MAX_W)
            .min(small_cap.max(SIDEBAR_MIN_W))
            .min(available_w.max(1));
        let usable_h = bottom.saturating_sub(top).max(1) as u32;
        let panel = Rect::new(SIDEBAR_MARGIN, top, width, usable_h);
        let content = Rect::new(
            panel.x + CONTENT_PAD,
            panel.y + HEADER_H as i32,
            panel
                .w
                .saturating_sub((CONTENT_PAD as u32).saturating_mul(2)),
            panel.h.saturating_sub(HEADER_H + CONTENT_PAD as u32),
        );
        let total_content_h = WEATHER_CARD_H
            .saturating_add(MONITOR_CARD_H)
            .saturating_add(NEWS_CARD_H)
            .saturating_add((CARD_GAP as u32).saturating_mul(2));
        let max_scroll = total_content_h.saturating_sub(content.h);
        let offset = scroll.min(max_scroll) as i32;
        let weather = Rect::new(content.x, content.y - offset, content.w, WEATHER_CARD_H);
        let monitor = Rect::new(
            content.x,
            weather.bottom() + CARD_GAP,
            content.w,
            MONITOR_CARD_H,
        );
        let news = Rect::new(
            content.x,
            monitor.bottom() + CARD_GAP,
            content.w,
            NEWS_CARD_H,
        );
        let unit_toggle = Rect::new(weather.right() - 86, weather.y + 38, 74, 24);
        let article_rows = core::array::from_fn(|index| {
            Rect::new(
                news.x + 8,
                news.y + 32 + (index as u32).saturating_mul(NEWS_ROW_H) as i32,
                news.w.saturating_sub(16),
                NEWS_ROW_H.saturating_sub(2),
            )
        });
        Self {
            panel,
            content,
            weather,
            monitor,
            news,
            unit_toggle,
            article_rows,
            max_scroll,
        }
    }

    fn hit_test(&self, point: Point) -> Option<SidebarHit> {
        if let Some(unit) = UnitToggle::new(self.unit_toggle, &["C", "F"], 0).hit_test(point) {
            return Some(SidebarHit::Unit(unit));
        }
        self.article_rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.contains(point))
            .map(|(index, _)| SidebarHit::Article(index))
    }
}

pub(crate) enum SidebarAction {
    None,
    Close,
    OpenUrl(&'static str),
}

/// Session-local Sidebar state. It is bounded and does not own timers,
/// networking, or an application identity.
pub(crate) struct SidebarState {
    open: bool,
    unit: TemperatureUnit,
    scroll: u32,
    focus: SidebarFocus,
    hover: Option<SidebarHit>,
    telemetry: Option<SystemMonitorViewData>,
    telemetry_unavailable: bool,
}

impl SidebarState {
    pub(crate) const fn new() -> Self {
        Self {
            open: false,
            unit: TemperatureUnit::Celsius,
            scroll: 0,
            focus: SidebarFocus::Unit,
            hover: None,
            telemetry: None,
            telemetry_unavailable: true,
        }
    }

    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub(crate) const fn unit(&self) -> TemperatureUnit {
        self.unit
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    pub(crate) fn close(&mut self) -> bool {
        let was_open = self.open;
        self.open = false;
        self.hover = None;
        was_open
    }

    pub(crate) fn contains(&self, point: Point, screen_w: u32, top: i32, bottom: i32) -> bool {
        self.open
            && SidebarLayout::compute(screen_w, top, bottom, self.scroll)
                .panel
                .contains(point)
    }

    /// Accepts the shell's already-owned telemetry snapshot. Hidden Sidebars
    /// do not request updates and retain only one last-valid sample.
    ///
    /// A failed / missing sample does **not** wipe a previously good sample —
    /// the card keeps showing the last valid numbers and only reports
    /// "Telemetry unavailable" when no sample has ever been accepted.
    pub(crate) fn observe_telemetry(&mut self, data: Option<SystemMonitorViewData>) -> bool {
        if !self.open {
            return false;
        }
        match data {
            Some(data) => {
                let changed = self.telemetry != Some(data) || self.telemetry_unavailable;
                self.telemetry = Some(data);
                self.telemetry_unavailable = false;
                changed
            }
            None => {
                // Keep the last-valid sample visible. Only flip to unavailable
                // when there is nothing useful to show yet.
                let unavailable = self.telemetry.is_none();
                let changed = self.telemetry_unavailable != unavailable;
                self.telemetry_unavailable = unavailable;
                changed
            }
        }
    }

    pub(crate) fn view(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        sunny_icon: Option<TgaImage>,
        article_icon: Option<TgaImage>,
        screen_w: u32,
        top: i32,
        bottom: i32,
    ) {
        if !self.open {
            return;
        }
        let layout = SidebarLayout::compute(screen_w, top, bottom, self.scroll);
        canvas.fill_material(
            layout.panel,
            Material::for_role(SurfaceRole::SystemOverlay, theme),
        );
        draw_text_vcenter(
            canvas,
            "Sidebar",
            layout.panel.x + 12,
            layout.panel.y + 6,
            28,
            &TextStyle::new(FontRole::UiLarge, theme.text),
        );
        draw_text_vcenter(
            canvas,
            "Widgets",
            layout.panel.right() - 70,
            layout.panel.y + 8,
            24,
            &TextStyle::new(FontRole::UiMedium, theme.accent),
        );
        canvas.hbar(
            layout.panel.x + 12,
            layout.content.y - 8,
            layout.panel.w.saturating_sub(24),
            1,
            theme.border,
        );

        self.draw_weather(canvas, theme, &layout, sunny_icon);
        self.draw_monitor(canvas, theme, &layout);
        self.draw_news(canvas, theme, &layout, article_icon);
    }

    fn draw_weather(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        layout: &SidebarLayout,
        sunny_icon: Option<TgaImage>,
    ) {
        WidgetCard::new(layout.weather, "Weather")
            .with_badge("Preview")
            .draw_chrome(canvas, theme);
        draw_card_heading(canvas, layout.weather, "Weather", theme);
        draw_card_badge(canvas, layout.weather, "Preview", theme);
        let temp = temperature_for_unit(WEATHER_PREVIEW.temperature_c, self.unit);
        let unit = match self.unit {
            TemperatureUnit::Celsius => "C",
            TemperatureUnit::Fahrenheit => "F",
        };
        let mut temp_text = [0u8; 8];
        let len = write_signed_into(temp, &mut temp_text);
        let text = core::str::from_utf8(&temp_text[..len]).unwrap_or("--");
        draw_text_vcenter(
            canvas,
            text,
            layout.weather.x + 14,
            layout.weather.y + 36,
            28,
            &TextStyle::new(FontRole::UiLarge, theme.accent),
        );
        let temp_width = measure_text(text, FontRole::UiLarge).w as i32;
        draw_text_vcenter(
            canvas,
            unit,
            layout.weather.x + 18 + temp_width,
            layout.weather.y + 42,
            18,
            &TextStyle::new(FontRole::UiRegular, theme.text_dim),
        );
        draw_text_vcenter(
            canvas,
            WEATHER_PREVIEW.location,
            layout.weather.x + 14,
            layout.weather.y + 66,
            16,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        draw_text_vcenter(
            canvas,
            WEATHER_PREVIEW.condition,
            layout.weather.x + 14,
            layout.weather.y + 84,
            16,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        if let Some(icon) = sunny_icon {
            canvas.draw_tga_icon_tinted(
                &icon,
                Rect::new(layout.weather.right() - 44, layout.weather.y + 74, 20, 20),
                theme.accent,
            );
        }
        let unit_toggle = UnitToggle::new(layout.unit_toggle, &["C", "F"], self.unit.index())
            .with_focus(matches!(self.focus, SidebarFocus::Unit));
        unit_toggle.draw_chrome(canvas, theme);
        for (index, label) in unit_toggle.labels.iter().enumerate() {
            draw_text_centered(
                canvas,
                unit_toggle.item_rect(index).inset(2),
                label,
                FontRole::UiMedium,
                if index == self.unit.index() {
                    theme.text
                } else {
                    theme.text_dim
                },
            );
        }
    }

    fn draw_monitor(&self, canvas: &mut Canvas, theme: &Theme, layout: &SidebarLayout) {
        WidgetCard::new(layout.monitor, "System Monitor").draw_chrome(canvas, theme);
        draw_card_heading(canvas, layout.monitor, "System Monitor", theme);
        // Prefer any retained last-valid sample. `telemetry_unavailable` is
        // only true when no sample has been accepted yet.
        let Some(data) = self.telemetry.as_ref().filter(|_| !self.telemetry_unavailable) else {
            draw_text_vcenter(
                canvas,
                "Telemetry unavailable",
                layout.monitor.x + 14,
                layout.monitor.y + 48,
                18,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
            draw_text_vcenter(
                canvas,
                "Other widgets remain available.",
                layout.monitor.x + 14,
                layout.monitor.y + 70,
                16,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
            return;
        };

        let mut cpu = [0u8; 12];
        let cpu_len = write_bp_percent_into(data.cpu_bp, &mut cpu);
        draw_text_vcenter(
            canvas,
            "CPU",
            layout.monitor.x + 14,
            layout.monitor.y + 36,
            18,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        draw_text_right(
            canvas,
            Rect::new(layout.monitor.right() - 70, layout.monitor.y + 36, 56, 18),
            core::str::from_utf8(&cpu[..cpu_len]).unwrap_or("--"),
            FontRole::UiRegular,
            theme.text,
        );
        MetricBar::new(
            Rect::new(
                layout.monitor.x + 14,
                layout.monitor.y + 58,
                layout.monitor.w.saturating_sub(28),
                8,
            ),
            data.cpu_bp,
        )
        .draw(canvas, theme);

        let mut used = [0u8; 16];
        let mut total = [0u8; 16];
        let used_len = write_mib_into(data.used_ram_kb, &mut used);
        let total_len = write_mib_into(data.total_ram_kb, &mut total);
        draw_text_vcenter(
            canvas,
            "RAM",
            layout.monitor.x + 14,
            layout.monitor.y + 76,
            18,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        draw_text_vcenter(
            canvas,
            core::str::from_utf8(&used[..used_len]).unwrap_or("--"),
            layout.monitor.x + 52,
            layout.monitor.y + 76,
            18,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        draw_text_vcenter(
            canvas,
            "of",
            layout.monitor.x + 112,
            layout.monitor.y + 76,
            18,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        draw_text_vcenter(
            canvas,
            core::str::from_utf8(&total[..total_len]).unwrap_or("--"),
            layout.monitor.x + 130,
            layout.monitor.y + 76,
            18,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        MetricBar::new(
            Rect::new(
                layout.monitor.x + 14,
                layout.monitor.y + 98,
                layout.monitor.w.saturating_sub(28),
                8,
            ),
            data.ram_bp,
        )
        .draw(canvas, theme);

        let mut tasks = [0u8; 10];
        let task_len = write_u64_into(data.task_count as u64, &mut tasks);
        draw_text_vcenter(
            canvas,
            "Tasks",
            layout.monitor.x + 14,
            layout.monitor.y + 112,
            18,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        draw_text_vcenter(
            canvas,
            core::str::from_utf8(&tasks[..task_len]).unwrap_or("0"),
            layout.monitor.x + 54,
            layout.monitor.y + 112,
            18,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );

        let mut zram_orig = [0u8; 16];
        let mut zram_comp = [0u8; 16];
        let zram_orig_len = write_mib_into(data.zram_orig_kb, &mut zram_orig);
        let zram_comp_len = write_mib_into(data.zram_comp_kb, &mut zram_comp);
        draw_text_vcenter(
            canvas,
            "ZRAM/Swap",
            layout.monitor.x + 14,
            layout.monitor.y + 132,
            18,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        if data.zram_orig_kb > 0 {
            draw_text_vcenter(
                canvas,
                core::str::from_utf8(&zram_comp[..zram_comp_len]).unwrap_or("0"),
                layout.monitor.x + 102,
                layout.monitor.y + 132,
                18,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
            draw_text_vcenter(
                canvas,
                "compressed from",
                layout.monitor.x + 166,
                layout.monitor.y + 132,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            draw_text_vcenter(
                canvas,
                core::str::from_utf8(&zram_orig[..zram_orig_len]).unwrap_or("0"),
                layout.monitor.x + 296,
                layout.monitor.y + 132,
                18,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
            let ratio = if data.zram_orig_kb > 0 {
                (data.zram_comp_kb as u32).saturating_mul(100).checked_div(
                    data.zram_orig_kb as u32,
                ).unwrap_or(0)
            } else {
                0
            };
            let mut comp_text = [0u8; 8];
            let comp_len = write_u64_into(ratio as u64, &mut comp_text);
            draw_text_vcenter(
                canvas,
                core::str::from_utf8(&comp_text[..comp_len]).unwrap_or("0"),
                layout.monitor.x + 346,
                layout.monitor.y + 132,
                18,
                &TextStyle::new(FontRole::UiRegular, theme.accent),
            );
            draw_text_vcenter(
                canvas,
                "% ratio",
                layout.monitor.x + 366,
                layout.monitor.y + 132,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        } else {
            draw_text_vcenter(
                canvas,
                "No compressed pages",
                layout.monitor.x + 102,
                layout.monitor.y + 132,
                18,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
        }
    }

    fn draw_news(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        layout: &SidebarLayout,
        article_icon: Option<TgaImage>,
    ) {
        WidgetCard::new(layout.news, "Sunlight News")
            .with_badge("Preview")
            .draw_chrome(canvas, theme);
        draw_card_heading(canvas, layout.news, "Sunlight News", theme);
        draw_card_badge(canvas, layout.news, "Preview", theme);
        for (index, article) in NEWS_PREVIEW.iter().enumerate() {
            let focused = self.focus == SidebarFocus::Article(index);
            let hovered = self.hover == Some(SidebarHit::Article(index));
            let row = ArticleListItem::new(
                layout.article_rows[index],
                article.title,
                article.summary,
                article.category,
            )
            .with_interaction(hovered, focused);
            row.draw_chrome(canvas, theme);
            let text_x = if let Some(icon) = article_icon {
                canvas.draw_tga_icon_tinted(
                    &icon,
                    Rect::new(row.rect.x + 7, row.rect.y + 14, 16, 16),
                    if focused || hovered {
                        theme.accent
                    } else {
                        theme.text_dim
                    },
                );
                row.rect.x + 30
            } else {
                row.rect.x + 8
            };
            draw_text_vcenter(
                canvas,
                article.title,
                text_x,
                row.rect.y + 3,
                15,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
            draw_text_vcenter(
                canvas,
                article.summary,
                text_x,
                row.rect.y + 18,
                14,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            draw_text_vcenter(
                canvas,
                article.category,
                text_x,
                row.rect.y + 33,
                12,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if focused || hovered {
                        theme.accent
                    } else {
                        theme.text_muted
                    },
                ),
            );
        }
    }

    pub(crate) fn handle_event(
        &mut self,
        event: Event,
        screen_w: u32,
        top: i32,
        bottom: i32,
    ) -> SidebarAction {
        if !self.open {
            return SidebarAction::None;
        }
        let layout = SidebarLayout::compute(screen_w, top, bottom, self.scroll);
        match event {
            Event::Click { x, y } => {
                let point = Point::new(x, y);
                if !layout.panel.contains(point) {
                    self.close();
                    return SidebarAction::Close;
                }
                match layout.hit_test(point) {
                    Some(SidebarHit::Unit(index)) => {
                        self.unit = if index == 1 {
                            TemperatureUnit::Fahrenheit
                        } else {
                            TemperatureUnit::Celsius
                        };
                        self.focus = SidebarFocus::Unit;
                        SidebarAction::None
                    }
                    Some(SidebarHit::Article(index)) => {
                        self.focus = SidebarFocus::Article(index);
                        self.close();
                        SidebarAction::OpenUrl(NEWS_PREVIEW[index].url)
                    }
                    None => SidebarAction::None,
                }
            }
            Event::MouseMove { x, y } => {
                self.hover = layout.hit_test(Point::new(x, y));
                SidebarAction::None
            }
            Event::Key('\x1b') => {
                self.close();
                SidebarAction::Close
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                shift,
                ..
            } if keycode == KEY_TAB => {
                self.step_focus(shift);
                SidebarAction::None
            }
            Event::KeyPress {
                keycode: KEY_LEFT,
                pressed: true,
                ..
            } => {
                self.select_previous();
                SidebarAction::None
            }
            Event::KeyPress {
                keycode: KEY_RIGHT,
                pressed: true,
                ..
            } => {
                self.select_next();
                SidebarAction::None
            }
            Event::KeyPress {
                keycode: KEY_UP,
                pressed: true,
                ..
            } => {
                self.scroll = self.scroll.saturating_sub(SCROLL_STEP);
                SidebarAction::None
            }
            Event::KeyPress {
                keycode: KEY_DOWN,
                pressed: true,
                ..
            } => {
                self.scroll = self
                    .scroll
                    .saturating_add(SCROLL_STEP)
                    .min(layout.max_scroll);
                SidebarAction::None
            }
            Event::KeyPress {
                keycode: KEY_PGUP,
                pressed: true,
                ..
            } => {
                self.scroll = self.scroll.saturating_sub(layout.content.h);
                SidebarAction::None
            }
            Event::KeyPress {
                keycode: KEY_PGDN,
                pressed: true,
                ..
            } => {
                self.scroll = self
                    .scroll
                    .saturating_add(layout.content.h)
                    .min(layout.max_scroll);
                SidebarAction::None
            }
            Event::KeyPress {
                keycode: KEY_ENTER | KEY_SPACE,
                pressed: true,
                ..
            } => match self.focus {
                SidebarFocus::Unit => {
                    self.unit = match self.unit {
                        TemperatureUnit::Celsius => TemperatureUnit::Fahrenheit,
                        TemperatureUnit::Fahrenheit => TemperatureUnit::Celsius,
                    };
                    SidebarAction::None
                }
                SidebarFocus::Article(index) => {
                    self.close();
                    SidebarAction::OpenUrl(NEWS_PREVIEW[index].url)
                }
            },
            _ => SidebarAction::None,
        }
    }

    fn step_focus(&mut self, reverse: bool) {
        self.focus = match (self.focus, reverse) {
            (SidebarFocus::Unit, false) => SidebarFocus::Article(0),
            (SidebarFocus::Unit, true) => SidebarFocus::Article(NEWS_PREVIEW.len() - 1),
            (SidebarFocus::Article(index), false) if index + 1 < NEWS_PREVIEW.len() => {
                SidebarFocus::Article(index + 1)
            }
            (SidebarFocus::Article(_), false) => SidebarFocus::Unit,
            (SidebarFocus::Article(0), true) => SidebarFocus::Unit,
            (SidebarFocus::Article(index), true) => SidebarFocus::Article(index - 1),
        };
    }

    fn select_previous(&mut self) {
        if self.focus == SidebarFocus::Unit {
            self.unit = TemperatureUnit::Celsius;
        }
    }

    fn select_next(&mut self) {
        if self.focus == SidebarFocus::Unit {
            self.unit = TemperatureUnit::Fahrenheit;
        }
    }
}

fn draw_card_heading(canvas: &mut Canvas, card: Rect, title: &str, theme: &Theme) {
    draw_text_vcenter(
        canvas,
        title,
        card.x + 12,
        card.y + 7,
        18,
        &TextStyle::new(FontRole::UiMedium, theme.text),
    );
}

fn draw_card_badge(canvas: &mut Canvas, card: Rect, badge: &str, theme: &Theme) {
    let width = measure_text(badge, FontRole::UiSmall)
        .w
        .saturating_add(12)
        .min(card.w.saturating_sub(24));
    let badge_rect = Rect::new(card.right() - width as i32 - 10, card.y + 7, width, 16);
    canvas.fill_rounded_rect(badge_rect, 5, theme.panel_alt);
    draw_text_centered(canvas, badge_rect, badge, FontRole::UiSmall, theme.text_dim);
}

fn draw_text_centered(
    canvas: &mut Canvas,
    rect: Rect,
    text: &str,
    role: FontRole,
    color: sunlight_ui::Color,
) {
    let width = measure_text(text, role).w as i32;
    draw_text_vcenter(
        canvas,
        text,
        rect.x + (rect.w as i32 - width) / 2,
        rect.y,
        rect.h,
        &TextStyle::new(role, color),
    );
}

fn draw_text_right(
    canvas: &mut Canvas,
    rect: Rect,
    text: &str,
    role: FontRole,
    color: sunlight_ui::Color,
) {
    let width = measure_text(text, role).w as i32;
    draw_text_vcenter(
        canvas,
        text,
        rect.right() - width,
        rect.y,
        rect.h,
        &TextStyle::new(role, color),
    );
}

fn write_u64_into(mut value: u64, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }
    if value == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    while value > 0 && len < digits.len() {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    let take = len.min(out.len());
    for index in 0..take {
        out[index] = digits[len - index - 1];
    }
    take
}

fn write_signed_into(value: i16, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }
    if value < 0 {
        out[0] = b'-';
        return 1 + write_u64_into(value.unsigned_abs() as u64, &mut out[1..]);
    }
    write_u64_into(value as u64, out)
}

fn write_bp_percent_into(bp: u16, out: &mut [u8]) -> usize {
    let whole = (bp.min(10_000) / 100) as u64;
    let len = write_u64_into(whole, out);
    if len < out.len() {
        out[len] = b'%';
        len + 1
    } else {
        len
    }
}

fn write_mib_into(kb: u64, out: &mut [u8]) -> usize {
    let len = write_u64_into(kb / 1024, out);
    if len + 4 <= out.len() {
        out[len..len + 4].copy_from_slice(b" MiB");
        len + 4
    } else {
        len
    }
}

#[cfg(test)]
mod tests {
    use super::{
        temperature_for_unit, NewsArticleViewData, SidebarAction, SidebarLayout, SidebarState,
        SystemMonitorViewData, TemperatureUnit, NEWS_PREVIEW, WEATHER_PREVIEW,
    };
    use sunlight_ui::Event;

    #[test]
    fn weather_preview_is_deterministic() {
        assert_eq!(WEATHER_PREVIEW.location, "Tehran");
        assert_eq!(WEATHER_PREVIEW.temperature_c, 24);
    }

    #[test]
    fn celsius_to_fahrenheit_is_exact_for_preview_value() {
        assert_eq!(temperature_for_unit(24, TemperatureUnit::Fahrenheit), 75);
    }

    #[test]
    fn repeated_unit_toggle_keeps_canonical_temperature() {
        let celsius = WEATHER_PREVIEW.temperature_c;
        for _ in 0..32 {
            assert_eq!(
                temperature_for_unit(celsius, TemperatureUnit::Fahrenheit),
                75
            );
            assert_eq!(temperature_for_unit(celsius, TemperatureUnit::Celsius), 24);
        }
    }

    #[test]
    fn unit_selection_survives_close_and_reopen() {
        let mut sidebar = SidebarState::new();
        sidebar.open();
        let _ = sidebar.handle_event(
            Event::key_press(0x4D, true, false, false, false, false),
            1280,
            50,
            840,
        );
        sidebar.close();
        sidebar.open();
        assert_eq!(sidebar.unit(), TemperatureUnit::Fahrenheit);
    }

    #[test]
    fn news_fixture_is_small_and_urls_are_https() {
        assert!(NEWS_PREVIEW.len() <= 5);
        assert!(NEWS_PREVIEW
            .iter()
            .all(|article| article.url.starts_with("https://")));
    }

    #[test]
    fn malformed_ram_is_rejected_and_cpu_is_clamped() {
        assert!(SystemMonitorViewData::from_values(12_000, 10, 0, 1, 0, 0).is_none());
        assert!(SystemMonitorViewData::from_values(12_000, 11, 10, 1, 0, 0).is_none());
        assert_eq!(
            SystemMonitorViewData::from_values(12_000, 10, 10, 1, 0, 0)
                .unwrap()
                .cpu_bp,
            10_000
        );
    }

    #[test]
    fn telemetry_missing_before_first_sample_is_unavailable() {
        let mut sidebar = SidebarState::new();
        sidebar.open();
        assert!(sidebar.observe_telemetry(None));
        assert!(sidebar.telemetry.is_none());
        assert!(sidebar.telemetry_unavailable);
        // Idempotent while still empty.
        assert!(!sidebar.observe_telemetry(None));
    }

    #[test]
    fn telemetry_retains_last_valid_sample_on_transient_failure() {
        let mut sidebar = SidebarState::new();
        sidebar.open();
        let sample = SystemMonitorViewData::from_values(684, 424_800, 3_658_500, 27, 0, 0)
            .expect("valid sample");
        assert!(sidebar.observe_telemetry(Some(sample)));
        assert_eq!(sidebar.telemetry, Some(sample));
        assert!(!sidebar.telemetry_unavailable);

        // A later failed sample must not hide the retained metrics.
        assert!(!sidebar.observe_telemetry(None));
        assert_eq!(sidebar.telemetry, Some(sample));
        assert!(!sidebar.telemetry_unavailable);
    }

    #[test]
    fn telemetry_updates_when_new_sample_arrives() {
        let mut sidebar = SidebarState::new();
        sidebar.open();
        let first = SystemMonitorViewData::from_values(100, 100, 1000, 1, 0, 0).unwrap();
        let second = SystemMonitorViewData::from_values(200, 200, 1000, 2, 0, 0).unwrap();
        assert!(sidebar.observe_telemetry(Some(first)));
        assert!(sidebar.observe_telemetry(Some(second)));
        assert_eq!(sidebar.telemetry, Some(second));
        assert!(!sidebar.telemetry_unavailable);
    }

    #[test]
    fn click_outside_and_escape_close_sidebar() {
        let mut sidebar = SidebarState::new();
        sidebar.open();
        assert!(matches!(
            sidebar.handle_event(Event::click(900, 400), 1280, 50, 840),
            SidebarAction::Close
        ));
        sidebar.open();
        assert!(matches!(
            sidebar.handle_event(Event::key('\x1b'), 1280, 50, 840),
            SidebarAction::Close
        ));
    }

    #[test]
    fn small_and_large_layouts_are_bounded() {
        let small = SidebarLayout::compute(320, 50, 540, 0);
        let large = SidebarLayout::compute(1920, 50, 1040, 0);
        assert!(small.panel.w <= 240);
        assert!((340..=420).contains(&large.panel.w));
    }

    #[test]
    fn article_activation_returns_static_url() {
        let mut sidebar = SidebarState::new();
        sidebar.open();
        let layout = SidebarLayout::compute(1280, 50, 840, 0);
        let row = layout.article_rows[1];
        assert!(matches!(
            sidebar.handle_event(Event::click(row.x + 2, row.y + 2), 1280, 50, 840),
            SidebarAction::OpenUrl(url) if url == NEWS_PREVIEW[1].url
        ));
    }

    const _: NewsArticleViewData = NEWS_PREVIEW[0];
}
