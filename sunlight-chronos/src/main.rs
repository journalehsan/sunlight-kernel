#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

use alloc::string::String;
#[cfg(not(test))]
use alloc::{format, vec::Vec};

#[cfg(not(test))]
use core::alloc::{GlobalAlloc, Layout};

use chronos_core::{
    display_char, translate_key_press, BiosKey, GuestState, HostKeyEvent, LoaderError, MzError,
    Runtime, UnsupportedExecutable, CHRONOS_INTERACTIVE_COM,
};
#[cfg(not(test))]
use chronos_core::{DosDrive, DosEntry};
use sun_font::{draw_text, measure_text, FontRole, TextStyle};
use sunlight_ipc::{debug_log, get_time_utc, monotonic_millis};
#[cfg(not(test))]
use sunlight_ipc::{process_yield, ProcessExit};
#[cfg(not(test))]
use sunlight_libc::{crt0, env, O_CREAT, O_TRUNC, O_WRONLY};
use sunlight_ui::{widgets::Panel, App, Canvas, Color, Event, Rect, Theme};
#[cfg(not(test))]
use sunlight_ui::{Window, WindowConfig, WindowDecoration};

const WIN_W: u32 = 840;
const WIN_H: u32 = 592;
const HEADER_H: u32 = 48;
const FOOTER_H: u32 = 24;
const PAD: i32 = 18;
const SURFACE_INSET: i32 = 6;
const DOS_CELL_W: i32 = 9;
const DOS_CELL_H: i32 = 16;
const INSTRUCTIONS_PER_TICK: usize = 128;
const DOS_SURFACE: Color = Color::rgb(12, 20, 37);
const DOS_CURSOR: Color = Color::rgb(255, 181, 71);
#[cfg(not(test))]
const MAX_HOST_FILE: usize = 64 * 1024;
#[cfg(not(test))]
const MAX_IMPORT_DEPTH: usize = 8;
#[cfg(not(test))]
const HEAP_SIZE: usize = 4 * 1024 * 1024;

#[cfg(not(test))]
struct BumpAllocator;

#[cfg(not(test))]
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
        static mut NEXT: usize = 0;

        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned.saturating_add(layout.size());
        if end > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        NEXT = end;
        core::ptr::addr_of_mut!(HEAP).cast::<u8>().add(aligned)
    }

    unsafe fn dealloc(&self, _pointer: *mut u8, _layout: Layout) {}
}

#[cfg(not(test))]
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    debug_log("[CHRONOS] panic");
    if let Some(location) = info.location() {
        debug_log(" at ");
        debug_log(location.file());
        debug_log(":");
        debug_log_u32(location.line());
        debug_log(":");
        debug_log_u32(location.column());
    }
    debug_log("\n");
    loop {
        process_yield();
    }
}

struct ChronosApp {
    runtime: Runtime,
    title: String,
    #[cfg(not(test))]
    storage: Option<ChronosStorage>,
    persisted: bool,
    status: [u8; 32],
    status_len: usize,
    trap_logged: bool,
    cursor_visible: bool,
}

impl ChronosApp {
    fn new() -> Self {
        let runtime = Runtime::from_com(CHRONOS_INTERACTIVE_COM)
            .expect("bundled Chronos COM program must fit the guest process");
        let mut app = Self {
            runtime,
            title: String::from("Chronos - Sunlight DOS Terminal"),
            #[cfg(not(test))]
            storage: None,
            persisted: false,
            status: [0; 32],
            status_len: 0,
            trap_logged: false,
            cursor_visible: true,
        };
        app.set_status(b"Ready");
        app
    }

    #[cfg(not(test))]
    fn launch(config: ChronosLaunch) -> Self {
        let image = read_file(&config.entry).unwrap_or_else(|| CHRONOS_INTERACTIVE_COM.to_vec());
        debug_log("[CHRONOS] guest image loaded\n");
        let (mut runtime, load_error) = match Runtime::from_program(&image, &config.command_tail) {
            Ok(runtime) => (runtime, None),
            Err(error) => {
                log_loader_error(&error);
                (
                    Runtime::from_com(&[0xf4]).expect("minimal halted guest must load"),
                    Some(error),
                )
            }
        };
        debug_log("[CHRONOS] guest runtime initialized\n");
        runtime.set_application_id(config.app_id.as_bytes());
        runtime.set_executable_path(config.entry_name().as_bytes());
        runtime.set_guest_unix_time(get_time_utc());
        let storage = config.storage();
        seed_drives(&mut runtime, &storage);
        debug_log("[CHRONOS] DOS drives seeded\n");
        let mut app = Self {
            runtime,
            title: config.title,
            storage: Some(storage),
            persisted: false,
            status: [0; 32],
            status_len: 0,
            trap_logged: false,
            cursor_visible: true,
        };
        app.set_status(load_error.as_ref().map_or(b"Ready", loader_status));
        app
    }

    fn set_status(&mut self, value: &[u8]) -> bool {
        let length = value.len().min(self.status.len());
        if self.status_len == length && self.status[..length] == value[..length] {
            return false;
        }
        self.status_len = length;
        self.status[..length].copy_from_slice(&value[..length]);
        true
    }

    fn status_text(&self) -> &str {
        core::str::from_utf8(&self.status[..self.status_len]).unwrap_or("Guest Trapped")
    }

    fn update_status_from_runtime(&mut self) -> bool {
        match self.runtime.state().clone() {
            GuestState::Ready => self.set_status(b"Ready"),
            GuestState::Running => self.set_status(b"Running"),
            GuestState::WaitingForInput => self.set_status(b"Waiting for Input"),
            GuestState::Exited { code } => {
                let mut status = [0; 32];
                let mut length = copy_bytes(b"Exited with code ", &mut status);
                length += write_decimal_u8(code, &mut status[length..]);
                self.set_status(&status[..length])
            }
            GuestState::Halted => self.set_status(b"Guest Halted"),
            GuestState::Trapped(trap) => {
                let changed = self.set_status(b"Guest Trapped");
                if !self.trap_logged {
                    log_trap(&trap);
                    self.trap_logged = true;
                }
                changed
            }
        }
    }

    fn draw_text_surface(&self, canvas: &mut Canvas, rect: Rect, theme: &Theme) {
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.draw_rect(rect, theme.border);
        let surface = rect.inset(SURFACE_INSET);
        canvas.fill_rect(surface, DOS_SURFACE);
        canvas.draw_rect(surface, dos_color(8));

        let x0 = surface.x + ((surface.w as i32 - 80 * DOS_CELL_W) / 2).max(0);
        let y0 = surface.y + ((surface.h as i32 - 25 * DOS_CELL_H) / 2).max(0);
        draw_surface_cells(canvas, &self.runtime, x0, y0);
        if self.cursor_visible
            && matches!(
                self.runtime.state(),
                GuestState::WaitingForInput | GuestState::Running
            )
        {
            let cursor_x = x0 + self.runtime.cursor_column() as i32 * DOS_CELL_W;
            let cursor_y = y0 + self.runtime.cursor_row() as i32 * DOS_CELL_H + DOS_CELL_H - 2;
            canvas.fill_rect(
                Rect::new(cursor_x, cursor_y, DOS_CELL_W as u32, 2),
                DOS_CURSOR,
            );
        }
    }

    fn draw_status_bar(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = Rect::new(0, WIN_H as i32 - FOOTER_H as i32, WIN_W, FOOTER_H);
        let small = FontRole::UiSmall;
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.hbar(rect.x, rect.y, rect.w, 1, theme.border);

        let text_y = rect.y + (rect.h as i32 - measure_text("Ag", small).h as i32) / 2;
        draw_text(
            canvas,
            "DOS .COM / MZ",
            rect.x + 10,
            text_y,
            &TextStyle::new(small, theme.text_dim),
        );

        let status_size = measure_text(self.status_text(), small);
        draw_text(
            canvas,
            self.status_text(),
            rect.x + (rect.w as i32 - status_size.w as i32) / 2,
            text_y,
            &TextStyle::new(small, theme.text),
        );

        let runtime_hint = "INT 16h / B8000";
        let hint_size = measure_text(runtime_hint, small);
        draw_text(
            canvas,
            runtime_hint,
            rect.right() - hint_size.w as i32 - 10,
            text_y,
            &TextStyle::new(small, theme.text_dim),
        );
    }
}

#[cfg(not(test))]
fn loader_status(error: &LoaderError) -> &'static [u8] {
    match error {
        LoaderError::Mz(MzError::UnsupportedExtendedFormat(UnsupportedExecutable::Pe)) => {
            b"Unsupported executable: PE"
        }
        LoaderError::Mz(MzError::UnsupportedExtendedFormat(UnsupportedExecutable::Ne)) => {
            b"Unsupported executable: NE"
        }
        LoaderError::Mz(MzError::UnsupportedExtendedFormat(UnsupportedExecutable::Le)) => {
            b"Unsupported executable: LE"
        }
        LoaderError::Mz(MzError::UnsupportedExtendedFormat(UnsupportedExecutable::Lx)) => {
            b"Unsupported executable: LX"
        }
        LoaderError::Mz(_) => b"Invalid MZ header",
        LoaderError::InsufficientMemory { .. } => b"Insufficient DOS memory",
        _ => b"Guest program load failed",
    }
}

#[cfg(not(test))]
fn log_loader_error(error: &LoaderError) {
    debug_log("[CHRONOS] loader rejected guest: ");
    debug_log(core::str::from_utf8(loader_status(error)).unwrap_or("load failed"));
    debug_log("\n");
}

impl App for ChronosApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        Panel::new(Rect::new(0, 0, WIN_W, HEADER_H)).draw(canvas, theme);
        draw_text(
            canvas,
            self.title.as_str(),
            PAD,
            7,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text(
            canvas,
            "16-bit real-mode guest",
            PAD,
            25,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        let text_rect = Rect::new(
            PAD,
            HEADER_H as i32 + PAD,
            WIN_W - (PAD as u32 * 2),
            WIN_H - HEADER_H - FOOTER_H - PAD as u32 * 2,
        );
        self.draw_text_surface(canvas, text_rect, theme);
        self.draw_status_bar(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                let cursor_active = matches!(
                    self.runtime.state(),
                    GuestState::WaitingForInput | GuestState::Running
                );
                let cursor_visible = cursor_active && (monotonic_millis() / 500) % 2 == 0;
                let cursor_changed = self.cursor_visible != cursor_visible;
                self.cursor_visible = cursor_visible;
                if matches!(
                    self.runtime.state(),
                    GuestState::Ready | GuestState::Running
                ) {
                    let text_or_state_changed = self.runtime.run_slice(INSTRUCTIONS_PER_TICK);
                    let status_changed = self.update_status_from_runtime();
                    if matches!(self.runtime.state(), GuestState::Exited { .. }) && !self.persisted
                    {
                        self.persisted = true;
                        #[cfg(not(test))]
                        if let Some(storage) = &self.storage {
                            persist_drives(&self.runtime, storage);
                        }
                    }
                    cursor_changed || text_or_state_changed || status_changed
                } else {
                    cursor_changed
                }
            }
            Event::Key(ch)
                if ch.is_ascii_graphic() || matches!(ch, ' ' | '\n' | '\r' | '\u{8}') =>
            {
                let ascii = match ch {
                    '\n' => b'\r',
                    '\r' => b'\r',
                    '\u{8}' => 0x08,
                    _ => ch as u8,
                };
                self.runtime.inject_key(BiosKey {
                    ascii,
                    scan_code: 0,
                });
                self.update_status_from_runtime()
            }
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ctrl,
                alt,
                ..
            } => {
                self.runtime.update_modifiers(shift, ctrl, alt);
                let key = translate_key_press(HostKeyEvent {
                    keycode,
                    pressed,
                    shift,
                    ctrl,
                    alt,
                });
                if let Some(key) = key.filter(|key| key.ascii == 0 || key.ascii < 0x20) {
                    self.runtime.inject_key(key);
                    self.update_status_from_runtime()
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

fn log_trap(trap: &chronos_core::Trap) {
    let mut line = [0u8; 160];
    let mut length = copy_bytes(b"[CHRONOS] guest trap: ", &mut line);
    length += copy_bytes(trap.summary().as_bytes(), &mut line[length..]);

    if let chronos_core::Trap::UnsupportedOpcode { cs, ip, bytes, cpu } = trap {
        length += copy_bytes(b" cs:ip=", &mut line[length..]);
        length += write_hex_u16(*cs, &mut line[length..]);
        line[length] = b':';
        length += 1;
        length += write_hex_u16(*ip, &mut line[length..]);
        length += copy_bytes(b" opcode=", &mut line[length..]);
        length += write_hex_u8(bytes[0], &mut line[length..]);
        length += copy_bytes(b" ax=", &mut line[length..]);
        length += write_hex_u16(cpu.ax, &mut line[length..]);
        length += copy_bytes(b" bx=", &mut line[length..]);
        length += write_hex_u16(cpu.bx, &mut line[length..]);
        length += copy_bytes(b" cx=", &mut line[length..]);
        length += write_hex_u16(cpu.cx, &mut line[length..]);
        length += copy_bytes(b" dx=", &mut line[length..]);
        length += write_hex_u16(cpu.dx, &mut line[length..]);
        length += copy_bytes(b" ds=", &mut line[length..]);
        length += write_hex_u16(cpu.ds, &mut line[length..]);
        length += copy_bytes(b" es=", &mut line[length..]);
        length += write_hex_u16(cpu.es, &mut line[length..]);
        length += copy_bytes(b" ss=", &mut line[length..]);
        length += write_hex_u16(cpu.ss, &mut line[length..]);
    }
    if let chronos_core::Trap::UnsupportedInterrupt {
        interrupt,
        function,
    } = trap
    {
        length += copy_bytes(b" int=", &mut line[length..]);
        length += write_hex_u8(*interrupt, &mut line[length..]);
        length += copy_bytes(b" ah=", &mut line[length..]);
        length += write_hex_u8(*function, &mut line[length..]);
    }
    if let chronos_core::Trap::UnterminatedDosString {
        segment, offset, ..
    } = trap
    {
        length += copy_bytes(b" ds:dx=", &mut line[length..]);
        length += write_hex_u16(*segment, &mut line[length..]);
        line[length] = b':';
        length += 1;
        length += write_hex_u16(*offset, &mut line[length..]);
    }
    line[length] = b'\n';
    length += 1;

    if let Ok(text) = core::str::from_utf8(&line[..length]) {
        debug_log(text);
    }
}

fn copy_bytes(source: &[u8], destination: &mut [u8]) -> usize {
    let length = source.len().min(destination.len());
    destination[..length].copy_from_slice(&source[..length]);
    length
}

#[cfg(not(test))]
fn debug_log_u32(mut value: u32) {
    let mut buffer = [0u8; 10];
    let mut index = buffer.len();
    if value == 0 {
        debug_log("0");
        return;
    }
    while value != 0 {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    debug_log(core::str::from_utf8(&buffer[index..]).unwrap_or("0"));
}

fn write_hex_u16(value: u16, output: &mut [u8]) -> usize {
    for (index, shift) in [12, 8, 4, 0].iter().enumerate() {
        output[index] = hex_digit((value >> shift) as u8);
    }
    4
}

fn write_hex_u8(value: u8, output: &mut [u8]) -> usize {
    output[0] = hex_digit(value >> 4);
    output[1] = hex_digit(value);
    2
}

fn write_decimal_u8(value: u8, output: &mut [u8]) -> usize {
    if value >= 100 {
        output[0] = b'0' + value / 100;
        output[1] = b'0' + (value / 10) % 10;
        output[2] = b'0' + value % 10;
        3
    } else if value >= 10 {
        output[0] = b'0' + value / 10;
        output[1] = b'0' + value % 10;
        2
    } else {
        output[0] = b'0' + value;
        1
    }
}

fn hex_digit(value: u8) -> u8 {
    match value & 0x0f {
        0..=9 => b'0' + (value & 0x0f),
        digit => b'A' + (digit - 10),
    }
}

fn draw_surface_cells(canvas: &mut Canvas, runtime: &Runtime, x0: i32, y0: i32) {
    for row in 0..25 {
        for column in 0..80 {
            let cell = runtime.cell(column, row);
            let background = dos_color((cell.attribute >> 4) & 0x07);
            let foreground = dos_color(cell.attribute & 0x0f);
            canvas.fill_rect(
                Rect::new(
                    x0 + column as i32 * DOS_CELL_W,
                    y0 + row as i32 * DOS_CELL_H,
                    DOS_CELL_W as u32,
                    DOS_CELL_H as u32,
                ),
                background,
            );
            if cell.character != b' ' {
                let mut encoded = [0; 4];
                let text = display_char(cell.character).encode_utf8(&mut encoded);
                draw_text(
                    canvas,
                    text,
                    x0 + column as i32 * DOS_CELL_W,
                    y0 + row as i32 * DOS_CELL_H,
                    &TextStyle::new(FontRole::MonoRegular, foreground),
                );
            }
        }
    }
}

fn dos_color(index: u8) -> sunlight_ui::Color {
    const COLORS: [(u8, u8, u8); 16] = [
        (12, 20, 37),
        (71, 108, 174),
        (70, 151, 122),
        (72, 158, 181),
        (188, 91, 108),
        (175, 105, 168),
        (193, 137, 74),
        (203, 214, 229),
        (76, 95, 121),
        (111, 162, 230),
        (107, 205, 155),
        (106, 198, 222),
        (242, 125, 140),
        (212, 136, 207),
        (248, 190, 102),
        (244, 248, 255),
    ];
    let (red, green, blue) = COLORS[index as usize & 0x0f];
    sunlight_ui::Color::rgb(red, green, blue)
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::env::init(_envp);
    let config = ChronosLaunch::from_argv(_argc, _argv);
    let mut app = config
        .map(ChronosApp::launch)
        .unwrap_or_else(ChronosApp::new);
    debug_log("[CHRONOS] connecting display window\n");
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Chronos - Sunlight DOS Terminal",
        decoration: WindowDecoration::Normal,
    }) {
        Some(window) => window,
        None => loop {
            process_yield();
        },
    };
    debug_log("[CHRONOS] window connected\n");
    window.run(&mut app);
    ProcessExit::exit(0);
}

#[cfg(not(test))]
#[derive(Clone)]
struct ChronosLaunch {
    entry: String,
    title: String,
    app_id: String,
    bundle_root: Option<String>,
    documents_root: Option<String>,
    direct: bool,
    command_tail: Vec<u8>,
}

#[cfg(not(test))]
#[derive(Clone)]
struct ChronosStorage {
    c_base: String,
    c_dependencies: Option<String>,
    c_overlay: String,
    documents_root: Option<String>,
}

#[cfg(not(test))]
impl ChronosLaunch {
    fn from_argv(argc: u64, argv: *const *const u8) -> Option<Self> {
        let mut pointers = [core::ptr::null(); 15];
        let count = unsafe { crt0::collect_raw_args(argc, argv, &mut pointers) };
        let mut values: Vec<&[u8]> = Vec::new();
        for pointer in pointers.iter().take(count) {
            let length = unsafe { crt0::cstr_len(*pointer, 255) };
            values.push(unsafe { core::slice::from_raw_parts(*pointer, length) });
        }
        let bundled = values.iter().any(|value| *value == b"--chronos-bundle");
        let direct = values.iter().any(|value| *value == b"--chronos-direct");
        if !bundled && !direct {
            return None;
        }
        let entry = argument(&values, b"--chronos-entry")
            .or_else(|| argument(&values, b"--chronos-direct"))?;
        let app_id = argument(&values, b"--chronos-app-id")?;
        let title = argument(&values, b"--chronos-title")?;
        let bundle_root = argument(&values, b"--chronos-bundle");
        let documents_root = argument(&values, b"--chronos-documents");
        if entry.len() >= 255 || app_id.len() > 80 || title.len() > 100 {
            return None;
        }
        let mut command_tail = Vec::new();
        let skip = [
            b"--chronos-bundle".as_slice(),
            b"--chronos-entry".as_slice(),
            b"--chronos-direct".as_slice(),
            b"--chronos-app-id".as_slice(),
            b"--chronos-title".as_slice(),
            b"--chronos-documents".as_slice(),
        ];
        let mut index = 1usize;
        while index < values.len() {
            if skip.iter().any(|key| values[index] == *key) {
                index += 2;
            } else if !values[index].starts_with(b"--sunlight-launch=") {
                if !command_tail.is_empty() {
                    command_tail.push(b' ');
                }
                command_tail.extend_from_slice(values[index]);
                index += 1;
            } else {
                index += 1;
            }
        }
        command_tail.truncate(126);
        Some(Self {
            entry: String::from_utf8(entry.to_vec()).ok()?,
            title: String::from_utf8(title.to_vec()).ok()?,
            app_id: String::from_utf8(app_id.to_vec()).ok()?,
            bundle_root: bundle_root.and_then(|value| String::from_utf8(value.to_vec()).ok()),
            documents_root: documents_root.and_then(|value| String::from_utf8(value.to_vec()).ok()),
            direct,
            command_tail,
        })
    }

    fn storage(&self) -> ChronosStorage {
        let home = env::getenv(b"HOME").unwrap_or("/root");
        let c_base = if self.direct {
            parent_path(&self.entry)
        } else {
            format!("{}/Program", self.bundle_root.as_deref().unwrap_or(""))
        };
        let c_overlay = if self.direct {
            String::from("/tmp/chronos-direct-overlay")
        } else {
            format!("{}/.config/sunlight/chronos/{}/overlay", home, self.app_id)
        };
        ChronosStorage {
            c_base,
            c_dependencies: self
                .bundle_root
                .as_ref()
                .map(|root| format!("{}/Dependencies", root)),
            c_overlay,
            documents_root: self.documents_root.clone(),
        }
    }

    fn entry_name(&self) -> String {
        if self.direct {
            let name = self.entry.rsplit('/').next().unwrap_or("PROGRAM.EXE");
            format!("C:\\{}", name.to_ascii_uppercase())
        } else {
            let name = self.entry.rsplit('/').next().unwrap_or("PROGRAM.EXE");
            format!("C:\\{}", name.to_ascii_uppercase())
        }
    }
}

#[cfg(not(test))]
fn argument<'a>(values: &[&'a [u8]], key: &[u8]) -> Option<&'a [u8]> {
    values
        .iter()
        .position(|value| *value == key)
        .and_then(|index| values.get(index + 1).copied())
}

#[cfg(not(test))]
fn seed_drives(runtime: &mut Runtime, storage: &ChronosStorage) {
    import_directory(runtime, DosDrive::C, &storage.c_base, "", false, 0);
    if let Some(dependencies) = &storage.c_dependencies {
        import_directory(runtime, DosDrive::C, dependencies, "", false, 0);
    }
    import_directory(runtime, DosDrive::C, &storage.c_overlay, "", true, 0);
    if let Some(documents) = &storage.documents_root {
        import_directory(runtime, DosDrive::D, documents, "", false, 0);
    } else {
        runtime
            .drives_mut()
            .set_access(DosDrive::D, chronos_core::DriveAccess::ReadOnly);
    }
}

#[cfg(not(test))]
fn import_directory(
    runtime: &mut Runtime,
    drive: DosDrive,
    host_root: &str,
    guest_prefix: &str,
    overlay: bool,
    depth: usize,
) {
    if depth >= MAX_IMPORT_DEPTH {
        return;
    }
    let mut entries = [sunlight_libc::DirEntry::zeroed(); 64];
    let Ok(count) = sunlight_libc::read_dir(host_root.as_bytes(), &mut entries) else {
        return;
    };
    for entry in entries.iter().take(count) {
        let Ok(name) = core::str::from_utf8(entry.name_bytes()) else {
            continue;
        };
        if name.is_empty() || name.starts_with('.') || name.contains('/') || name.contains('\\') {
            continue;
        }
        let guest = if guest_prefix.is_empty() {
            name.to_ascii_uppercase()
        } else {
            format!("{}/{}", guest_prefix, name.to_ascii_uppercase())
        };
        let host = format!("{}/{}", host_root, name);
        if entry.file_type == sunlight_libc::FT_DIR {
            if overlay {
                let _ =
                    runtime
                        .drives_mut()
                        .import_overlay_entry(drive, &guest, DosEntry::directory());
            } else {
                let _ = runtime.drives_mut().add_base_directory(drive, &guest);
            }
            import_directory(runtime, drive, &host, &guest, overlay, depth + 1);
        } else if entry.file_type == sunlight_libc::FT_FILE {
            let Some(data) = read_file(&host) else {
                continue;
            };
            if overlay {
                let _ = runtime.drives_mut().import_overlay_entry(
                    drive,
                    &guest,
                    DosEntry::file(data, 0),
                );
            } else {
                let _ = runtime.drives_mut().add_base_file(drive, &guest, data);
            }
        }
    }
}

#[cfg(not(test))]
fn persist_drives(runtime: &Runtime, storage: &ChronosStorage) {
    if !storage.c_overlay.starts_with("/tmp/") {
        persist_drive(runtime, DosDrive::C, &storage.c_overlay);
    }
    if let Some(documents) = &storage.documents_root {
        persist_drive(runtime, DosDrive::D, documents);
    }
}

#[cfg(not(test))]
fn persist_drive(runtime: &Runtime, drive: DosDrive, root: &str) {
    let Ok(entries) = runtime.drives().overlay_entries(drive) else {
        return;
    };
    for (guest_path, entry) in entries {
        let target = format!("{}/{}", root, guest_path);
        if entry.is_directory {
            let _ = sunlight_libc::mkdir_recursive(target.as_bytes());
            continue;
        }
        let parent = parent_path(&target);
        let _ = sunlight_libc::mkdir_recursive(parent.as_bytes());
        let Ok(fd) =
            sunlight_libc::open_with_flags(target.as_bytes(), O_WRONLY | O_CREAT | O_TRUNC)
        else {
            continue;
        };
        let _ = sunlight_libc::write_all(fd, &entry.data);
        let _ = sunlight_libc::close(fd);
    }
}

#[cfg(not(test))]
fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = sunlight_libc::open(path.as_bytes()).ok()?;
    let mut data = Vec::new();
    let mut buffer = [0u8; 1024];
    while data.len() < MAX_HOST_FILE {
        let read = sunlight_libc::read(fd, &mut buffer).ok()?;
        if read == 0 {
            break;
        }
        let remaining = MAX_HOST_FILE - data.len();
        data.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    let _ = sunlight_libc::close(fd);
    Some(data)
}

#[cfg(not(test))]
fn parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.into())
        .unwrap_or_else(|| String::from("/"))
}

#[cfg(test)]
mod tests {
    use super::{dos_color, ChronosApp, DOS_CELL_W, DOS_SURFACE};
    use chronos_core::{BiosKey, GuestState};
    use sun_font::{measure_text, FontRole};
    use sunlight_ui::{App, Event};

    #[test]
    fn status_text_uses_the_precise_waiting_state() {
        let mut app = ChronosApp::new();
        assert_eq!(app.status_text(), "Ready");

        app.runtime.run_slice(1024);
        assert!(app.update_status_from_runtime());
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(app.status_text(), "Waiting for Input");
    }

    #[test]
    fn decoded_enter_reaches_the_guest_console() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        assert!(app.update(Event::Key('\n')));
        app.update(Event::Tick);
        assert_eq!(
            (app.runtime.cursor_column(), app.runtime.cursor_row()),
            (0, 7)
        );
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
    }

    #[test]
    fn decoded_enter_and_backspace_reach_the_guest_console() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);

        assert!(app.update(Event::Key('A')));
        app.update(Event::Tick);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert!(app.update(Event::Key('\u{8}')));
        app.update(Event::Tick);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert!(app.update(Event::Key('\n')));
        app.update(Event::Tick);

        assert_eq!(app.runtime.cell(0, 6).character, b' ');
        assert_eq!(
            (app.runtime.cursor_column(), app.runtime.cursor_row()),
            (0, 7)
        );
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
    }

    #[test]
    fn raw_printable_key_events_do_not_duplicate_text_input() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);

        assert!(!app.update(Event::KeyPress {
            keycode: 0x39,
            pressed: true,
            shift: false,
            ctrl: false,
            alt: false,
            super_key: false,
        }));
        assert!(app.update(Event::Key('A')));
        app.update(Event::Tick);

        assert_eq!(app.runtime.cell(0, 6).character, b'A');
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(
            (app.runtime.cursor_column(), app.runtime.cursor_row()),
            (1, 6)
        );
    }

    #[test]
    fn dos_grid_uses_fixed_fira_code_cell_widths() {
        assert_eq!(
            measure_text("MM", FontRole::MonoRegular).w,
            (DOS_CELL_W * 2) as u32
        );
    }

    #[test]
    fn dos_palette_keeps_a_navy_surface_and_distinct_dos_colors() {
        assert_eq!(dos_color(0), DOS_SURFACE);
        assert_ne!(dos_color(0), dos_color(1));
        assert_ne!(dos_color(7), dos_color(15));
        assert!(dos_color(15).r() > dos_color(0).r());
    }

    #[test]
    fn idle_cursor_ticks_do_not_wake_a_blocked_guest() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        app.update(Event::Tick);
        app.update(Event::Tick);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
    }

    #[test]
    fn bundled_interactive_guest_waits_and_exits_from_guest_keyboard_input() {
        let mut app = ChronosApp::new();
        app.runtime.run_slice(1024);
        assert!(app.update_status_from_runtime());
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        app.runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        app.runtime.run_slice(64);
        assert!(app.update_status_from_runtime());
        assert_eq!(app.runtime.state(), &GuestState::Exited { code: 0 });
        assert_eq!(app.status_text(), "Exited with code 0");
    }
}
