//! Developer widget gallery — DigitalNumber, SolarClock, WorldMap preview.
//!
//! Not integrated into Control Panel, Lock Screen, or timezone services.
//! Launch with `/bin/widget-gallery` for visual verification in QEMU.

#![no_std]
#![no_main]

use sun_font::{self, FontRole, VecFont};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_ui::paint::Canvas;
use sunlight_ui::theme::Theme;
use sunlight_ui::widgets::{
    DigitalAlign, DigitalNumberWidget, GeoCoord, MapHit, MapMarker, SolarClockSnapshot,
    SolarClockWidget, WorldMapWidget,
};
use sunlight_ui::{
    request_close, App, Event, Point, Rect, VecText, Window, WindowConfig, WindowDecoration,
    WindowMaterial,
};

static F_MED: VecFont = VecFont(FontRole::UiMedium);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const WIN_W: u32 = 920;
const WIN_H: u32 = 640;
const KEY_ESC: u8 = 0x01;

// Representative Solar Clock times (cycled with keys 1–5 / left-right).
const DEMO_TIMES: [(u8, u8, u8); 5] = [
    (0, 0, 0),
    (3, 15, 15),
    (6, 30, 30),
    (9, 45, 45),
    (23, 59, 59),
];

const DEMO_LABELS: [&str; 5] = [
    "00:00:00 midnight",
    "03:15:15",
    "06:30:30",
    "09:45:45",
    "23:59:59",
];

// Test markers (coordinates only — no city catalog).
const MARKERS: [MapMarker; 4] = [
    MapMarker {
        coord: GeoCoord {
            lon: -74.0,
            lat: 40.7,
        },
        hit_radius: 8,
    },
    MapMarker {
        coord: GeoCoord {
            lon: 2.35,
            lat: 48.85,
        },
        hit_radius: 8,
    },
    MapMarker {
        coord: GeoCoord {
            lon: 139.7,
            lat: 35.7,
        },
        hit_radius: 8,
    },
    MapMarker {
        coord: GeoCoord {
            lon: 151.2,
            lat: -33.9,
        },
        hit_radius: 8,
    },
];

struct NoAlloc;
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}
#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[WIDGET-GALLERY] panic\n");
    loop {
        process_yield();
    }
}

struct GalleryApp {
    time_idx: usize,
    show_date: bool,
    /// Last map click feedback (lon/lat text buffer).
    click_lon: i32,
    click_lat: i32,
    click_valid: bool,
    click_marker: Option<usize>,
    hover_lon: i32,
    hover_lat: i32,
    hover_valid: bool,
    selected: Option<GeoCoord>,
    /// Map size demo: 0 = large, 1 = medium, 2 = small.
    map_size_idx: usize,
    /// Dirty flag: only redraw when state changes (no continuous repaint).
    needs_redraw: bool,
    frames: u32,
}

impl GalleryApp {
    fn new() -> Self {
        Self {
            time_idx: 2,
            show_date: true,
            click_lon: 0,
            click_lat: 0,
            click_valid: false,
            click_marker: None,
            hover_lon: 0,
            hover_lat: 0,
            hover_valid: false,
            selected: None,
            map_size_idx: 0,
            needs_redraw: true,
            frames: 0,
        }
    }

    fn snapshot(&self) -> SolarClockSnapshot {
        let (h, m, s) = DEMO_TIMES[self.time_idx % DEMO_TIMES.len()];
        SolarClockSnapshot::new(h, m, s)
    }

    fn map_rect(&self) -> Rect {
        match self.map_size_idx % 3 {
            0 => Rect::new(40, 360, 520, 240),
            1 => Rect::new(40, 400, 360, 180),
            _ => Rect::new(40, 430, 240, 120),
        }
    }

    fn draw_header(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, 36), theme.panel_alt);
        F_MED.draw(
            canvas,
            "Widget Gallery — DigitalNumber · SolarClock · WorldMap",
            12,
            10,
            theme.accent,
        );
        F_SMALL.draw(
            canvas,
            "1-5 times  D date  M map size  click map  Esc quit  (dev only)",
            12,
            22,
            theme.text_dim,
        );
        canvas.hbar(0, 36, WIN_W, 1, theme.border);
    }

    fn draw_digital_section(&self, canvas: &mut Canvas, theme: &Theme) {
        F_SMALL.draw(canvas, "DigitalNumberWidget", 20, 48, theme.text_muted);

        // Row of sample values / sizes.
        let samples: [(&str, u32, u32, i32, i32); 5] = [
            ("04:05", 10, 18, 20, 68),
            ("23:59", 14, 24, 140, 64),
            ("-12.5", 12, 20, 280, 66),
            ("100", 16, 28, 420, 62),
            ("00:00", 20, 36, 540, 56),
        ];

        for (text, dw, dh, x, y) in samples {
            let mut w = DigitalNumberWidget::new(Rect::new(x, y, 110, dh + 8))
                .with_digit_size(dw, dh)
                .with_spacing((dw / 5).max(1))
                .with_max_chars(5)
                .with_align(DigitalAlign::Left)
                .with_colors(theme.accent, theme.panel_alt.lighten(16));
            let _ = w.set_value_str(text);
            canvas.fill_rect(Rect::new(x - 2, y - 2, 114, dh + 12), theme.panel);
            canvas.draw_rect(Rect::new(x - 2, y - 2, 114, dh + 12), theme.border);
            w.draw(canvas, theme);
            F_SMALL.draw(canvas, text, x, y + dh as i32 + 12, theme.text_dim);
        }

        // Leading zeros demo
        let mut z = DigitalNumberWidget::new(Rect::new(700, 64, 180, 40))
            .with_digit_size(14, 24)
            .with_max_chars(4)
            .with_leading_zeros(true)
            .with_align(DigitalAlign::Center);
        z.set_u32(42);
        canvas.fill_rect(Rect::new(698, 62, 184, 44), theme.panel);
        canvas.draw_rect(Rect::new(698, 62, 184, 44), theme.border);
        z.draw(canvas, theme);
        F_SMALL.draw(canvas, "leading zeros → 0042", 700, 110, theme.text_dim);
    }

    fn draw_clock_section(&self, canvas: &mut Canvas, theme: &Theme) {
        F_SMALL.draw(canvas, "SolarClockWidget", 20, 140, theme.text_muted);

        let snap = self.snapshot();
        let clock_rect = Rect::new(40, 158, 200, 200);
        let mut clock = SolarClockWidget::new(clock_rect, snap);
        if self.show_date {
            clock = clock.with_date("Mon 25 Jul");
        }
        clock.draw(canvas, theme);

        // Side legend / controls readout
        let lx = 260;
        let mut ly = 170;
        F_MED.draw(
            canvas,
            DEMO_LABELS[self.time_idx % DEMO_LABELS.len()],
            lx,
            ly,
            theme.text,
        );
        ly += 20;
        F_SMALL.draw(
            canvas,
            if self.show_date {
                "date: on (D toggles)"
            } else {
                "date: off (D toggles)"
            },
            lx,
            ly,
            theme.text_dim,
        );
        ly += 16;
        F_SMALL.draw(
            canvas,
            "no hands · 60 sun rays · dual tracks",
            lx,
            ly,
            theme.text_dim,
        );
        ly += 16;
        F_SMALL.draw(
            canvas,
            "second rays orange · minute thin · hour thick",
            lx,
            ly,
            theme.text_dim,
        );

        // Mini clocks at other demo times (compact)
        let minis = [(0usize, 260i32), (1, 360), (3, 460), (4, 560)];
        for (idx, x) in minis {
            let (h, m, s) = DEMO_TIMES[idx];
            SolarClockWidget::new(Rect::new(x, 250, 72, 72), SolarClockSnapshot::new(h, m, s))
                .draw(canvas, theme);
            F_SMALL.draw(canvas, DEMO_LABELS[idx], x, 324, theme.text_dim);
        }

        // Large clock without date for comparison
        SolarClockWidget::new(Rect::new(660, 158, 220, 220), snap).draw(canvas, theme);
        F_SMALL.draw(canvas, "no date text", 720, 380, theme.text_dim);
    }

    fn draw_map_section(&self, canvas: &mut Canvas, theme: &Theme) {
        F_SMALL.draw(
            canvas,
            "WorldMapWidget (equirectangular)",
            20,
            344,
            theme.text_muted,
        );

        let map_rect = self.map_rect();
        let mut map = WorldMapWidget::new(map_rect)
            .with_markers(&MARKERS)
            .with_grid(false);
        if let Some(sel) = self.selected {
            map = map.with_selected(sel);
        }
        if self.hover_valid {
            map = map.with_hover(GeoCoord::new(
                self.hover_lon as f32 / 100.0,
                self.hover_lat as f32 / 100.0,
            ));
        }
        map.draw(canvas, theme);

        // Coordinate feedback panel
        let fx = 580i32;
        let mut fy = 370i32;
        F_MED.draw(canvas, "Coordinates", fx, fy, theme.text);
        fy += 22;
        if self.click_valid {
            let mut buf = [0u8; 48];
            let n = format_coord_line(
                &mut buf,
                "click",
                self.click_lon,
                self.click_lat,
                self.click_marker,
            );
            if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                F_SMALL.draw(canvas, s, fx, fy, theme.accent);
            }
        } else {
            F_SMALL.draw(
                canvas,
                "click: (none — outside map ignored)",
                fx,
                fy,
                theme.text_dim,
            );
        }
        fy += 16;
        if self.hover_valid {
            let mut buf = [0u8; 48];
            let n = format_coord_line(&mut buf, "hover", self.hover_lon, self.hover_lat, None);
            if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                F_SMALL.draw(canvas, s, fx, fy, theme.text_muted);
            }
        } else {
            F_SMALL.draw(canvas, "hover: —", fx, fy, theme.text_dim);
        }
        fy += 20;
        F_SMALL.draw(
            canvas,
            "markers: NYC Paris Tokyo Sydney",
            fx,
            fy,
            theme.text_dim,
        );
        fy += 16;
        F_SMALL.draw(
            canvas,
            "M cycles map size (aspect preserved)",
            fx,
            fy,
            theme.text_dim,
        );
        fy += 16;
        F_SMALL.draw(
            canvas,
            "no timezone / city catalog in this widget",
            fx,
            fy,
            theme.text_dim,
        );

        // Frame counter to prove no continuous repaint (only advances on dirty).
        fy += 28;
        let mut fbuf = [0u8; 24];
        let flen = format_u32_label(&mut fbuf, b"redraws: ", self.frames);
        if let Ok(s) = core::str::from_utf8(&fbuf[..flen]) {
            F_SMALL.draw(canvas, s, fx, fy, theme.text_dim);
        }
    }
}

impl App for GalleryApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.frames = self.frames.saturating_add(1);
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        self.draw_header(canvas, theme);
        self.draw_digital_section(canvas, theme);
        self.draw_clock_section(canvas, theme);
        self.draw_map_section(canvas, theme);
        self.needs_redraw = false;
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Key(ch) => match ch {
                '1' => {
                    self.time_idx = 0;
                    true
                }
                '2' => {
                    self.time_idx = 1;
                    true
                }
                '3' => {
                    self.time_idx = 2;
                    true
                }
                '4' => {
                    self.time_idx = 3;
                    true
                }
                '5' => {
                    self.time_idx = 4;
                    true
                }
                'd' | 'D' => {
                    self.show_date = !self.show_date;
                    true
                }
                'm' | 'M' => {
                    self.map_size_idx = (self.map_size_idx + 1) % 3;
                    true
                }
                _ => false,
            },
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => {
                if keycode == KEY_ESC {
                    request_close();
                    return true;
                }
                // Left / right arrows (common XT codes 0x4B / 0x4D)
                if keycode == 0x4B {
                    self.time_idx = self.time_idx.checked_sub(1).unwrap_or(DEMO_TIMES.len() - 1);
                    return true;
                }
                if keycode == 0x4D {
                    self.time_idx = (self.time_idx + 1) % DEMO_TIMES.len();
                    return true;
                }
                false
            }
            Event::Click { x, y } | Event::MouseDown { x, y, button: 0 } => {
                let map = WorldMapWidget::new(self.map_rect()).with_markers(&MARKERS);
                match map.hit_test(Point::new(x, y)) {
                    MapHit::Outside => {
                        // Outside must not invent coordinates.
                        false
                    }
                    MapHit::Inside {
                        coord,
                        marker_index,
                    } => {
                        self.click_lon = (coord.lon * 100.0) as i32;
                        self.click_lat = (coord.lat * 100.0) as i32;
                        self.click_valid = true;
                        self.click_marker = marker_index;
                        self.selected = Some(coord);
                        true
                    }
                }
            }
            Event::MouseMove { x, y } => {
                let map = WorldMapWidget::new(self.map_rect());
                if let Some(c) = map.point_to_geo(Point::new(x, y)) {
                    let lon = (c.lon * 100.0) as i32;
                    let lat = (c.lat * 100.0) as i32;
                    if !self.hover_valid || lon != self.hover_lon || lat != self.hover_lat {
                        self.hover_lon = lon;
                        self.hover_lat = lat;
                        self.hover_valid = true;
                        return true;
                    }
                    false
                } else if self.hover_valid {
                    self.hover_valid = false;
                    true
                } else {
                    false
                }
            }
            // Tick must not force continuous redraws when state is unchanged.
            Event::Tick => false,
            _ => false,
        }
    }
}

fn format_u32_label(buf: &mut [u8], prefix: &[u8], value: u32) -> usize {
    let mut i = 0usize;
    for &b in prefix {
        if i < buf.len() {
            buf[i] = b;
            i += 1;
        }
    }
    let mut tmp = [0u8; 10];
    let mut n = value;
    let mut t = 0usize;
    if n == 0 {
        tmp[0] = b'0';
        t = 1;
    } else {
        while n > 0 && t < tmp.len() {
            tmp[t] = b'0' + (n % 10) as u8;
            n /= 10;
            t += 1;
        }
    }
    while t > 0 {
        t -= 1;
        if i < buf.len() {
            buf[i] = tmp[t];
            i += 1;
        }
    }
    i
}

fn format_coord_line(
    buf: &mut [u8],
    tag: &str,
    lon_c: i32,
    lat_c: i32,
    marker: Option<usize>,
) -> usize {
    // "click lon=-74.00 lat=40.70 m=0"
    let mut i = 0usize;
    for b in tag.bytes() {
        if i < buf.len() {
            buf[i] = b;
            i += 1;
        }
    }
    for &b in b" lon=" {
        if i < buf.len() {
            buf[i] = b;
            i += 1;
        }
    }
    i = write_fixed1(buf, i, lon_c);
    for &b in b" lat=" {
        if i < buf.len() {
            buf[i] = b;
            i += 1;
        }
    }
    i = write_fixed1(buf, i, lat_c);
    if let Some(m) = marker {
        for &b in b" m=" {
            if i < buf.len() {
                buf[i] = b;
                i += 1;
            }
        }
        if i < buf.len() {
            buf[i] = b'0' + (m as u8).min(9);
            i += 1;
        }
    }
    i
}

/// Write centi-degrees as `[-]ddd.dd`.
fn write_fixed1(buf: &mut [u8], mut i: usize, centi: i32) -> usize {
    let neg = centi < 0;
    let v = centi.unsigned_abs();
    if neg {
        if i < buf.len() {
            buf[i] = b'-';
            i += 1;
        }
    }
    let whole = v / 100;
    let frac = v % 100;
    let mut tmp = [0u8; 6];
    let mut t = 0usize;
    let mut n = whole;
    if n == 0 {
        tmp[0] = b'0';
        t = 1;
    } else {
        while n > 0 && t < tmp.len() {
            tmp[t] = b'0' + (n % 10) as u8;
            n /= 10;
            t += 1;
        }
    }
    while t > 0 {
        t -= 1;
        if i < buf.len() {
            buf[i] = tmp[t];
            i += 1;
        }
    }
    if i < buf.len() {
        buf[i] = b'.';
        i += 1;
    }
    if i < buf.len() {
        buf[i] = b'0' + (frac / 10) as u8;
        i += 1;
    }
    if i < buf.len() {
        buf[i] = b'0' + (frac % 10) as u8;
        i += 1;
    }
    i
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=widget-gallery",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let mut app = GalleryApp::new();

    let mut window = match Window::connect_with_material(
        WindowConfig {
            width: WIN_W,
            height: WIN_H,
            title: "Widget Gallery",
            decoration: WindowDecoration::CompactCloseMinimize,
        },
        WindowMaterial::Opaque,
    ) {
        Some(w) => w,
        None => {
            debug_log("[WIDGET-GALLERY] window connect failed\n");
            ProcessExit::exit(1);
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}
