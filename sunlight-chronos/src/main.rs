#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

#[cfg(not(test))]
use alloc::format;
use alloc::{collections::VecDeque, string::String, vec, vec::Vec};

#[cfg(not(test))]
use core::alloc::{GlobalAlloc, Layout};

use chronos_core::{
    display_char, translate_key_press, BiosKey, GuestState, GuestStateKind, GuestVideoMode,
    GuestWaitReason, HostKeyEvent, MouseViewport, Rgb8, Runtime, SliceStopReason,
    CHRONOS_INTERACTIVE_COM, VGA_FRAMEBUFFER_BYTES, VGA_HEIGHT, VGA_WIDTH,
};
#[cfg(not(test))]
use chronos_core::{DosDrive, DosEntry, LoaderError, MzError, UnsupportedExecutable};
use sun_font::{draw_text, measure_text, FontRole, TextStyle};
use sunlight_ipc::{debug_log, monotonic_millis};
#[cfg(not(test))]
use sunlight_ipc::{get_time_utc, process_yield, ProcessExit};
#[cfg(not(test))]
use sunlight_libc::{crt0, env, O_CREAT, O_TRUNC, O_WRONLY};
use sunlight_ui::{
    set_client_cursor, widgets::Panel, App, Canvas, Color, CursorShape, Event, EventPollCounters,
    Rect, Theme,
};
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
const INSTRUCTIONS_PER_TICK: usize = 2048;
const STARTUP_INSTRUCTION_BUDGET: usize = 32 * 1024;
const RUNNING_PRESENT_INTERVAL_MS: u64 = 16;
const BUDGET_CONTINUATION_INTERVAL_MS: u64 = 0;
const INT28_TIMER_INTERVAL_MS: u64 = 16;
const WAKE_EVIDENCE_CAPACITY: usize = 128;
const TEXT_PRESENT_INSTRUCTION_QUANTUM: usize = 32 * 1024;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeWakeSource {
    Startup,
    BudgetContinuation,
    Int28Timer,
    Keyboard,
    MouseMotion,
    MouseButton,
    FocusChanged,
    PointerOwnership,
    NativeEvent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WakeEvidence {
    sequence: u64,
    source: RuntimeWakeSource,
    guest_state: GuestStateKind,
    wait_reason: GuestWaitReason,
    cs: u16,
    ip: u16,
    instructions_retired: u64,
    next_wake_deadline: Option<u64>,
    framebuffer_generation: u64,
    palette_generation: u64,
    mouse_generation: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MouseInputCounters {
    event_route: EventPollCounters,
    pointer_moves_received: u64,
    pointer_buttons_received: u64,
    rejected_outside_content: u64,
    mapped_guest_coordinates: u64,
    mouse_state_updates: u64,
    mouse_generation_changes: u64,
    button_edge_updates: u64,
    duplicate_button_edges: u64,
    last_mapped_x: u16,
    last_mapped_y: u16,
}

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
    graphics_cache: Vec<Rgb8>,
    converted_framebuffer_generation: u64,
    converted_palette_generation: u64,
    converted_video_mode_generation: u64,
    framebuffer_conversions: u64,
    last_text_present_ms: u64,
    text_present_pending: bool,
    text_instructions_since_present: usize,
    last_graphics_present_ms: u64,
    graphics_present_pending: bool,
    graphics_viewport: MouseViewport,
    focused: bool,
    old_guest_cursor_rect: Option<Rect>,
    new_guest_cursor_rect: Option<Rect>,
    next_wake_deadline: Option<u64>,
    wake_evidence: VecDeque<WakeEvidence>,
    wake_sequence: u64,
    mouse_input_counters: MouseInputCounters,
    mouse_log_generation: u64,
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
            graphics_cache: vec![Rgb8::default(); VGA_FRAMEBUFFER_BYTES],
            converted_framebuffer_generation: u64::MAX,
            converted_palette_generation: u64::MAX,
            converted_video_mode_generation: u64::MAX,
            framebuffer_conversions: 0,
            last_text_present_ms: 0,
            text_present_pending: false,
            text_instructions_since_present: 0,
            last_graphics_present_ms: 0,
            graphics_present_pending: false,
            graphics_viewport: default_graphics_viewport(),
            focused: true,
            old_guest_cursor_rect: None,
            new_guest_cursor_rect: None,
            next_wake_deadline: None,
            wake_evidence: VecDeque::with_capacity(WAKE_EVIDENCE_CAPACITY),
            wake_sequence: 0,
            mouse_input_counters: MouseInputCounters::default(),
            mouse_log_generation: 0,
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
            graphics_cache: vec![Rgb8::default(); VGA_FRAMEBUFFER_BYTES],
            converted_framebuffer_generation: u64::MAX,
            converted_palette_generation: u64::MAX,
            converted_video_mode_generation: u64::MAX,
            framebuffer_conversions: 0,
            last_text_present_ms: 0,
            text_present_pending: false,
            text_instructions_since_present: 0,
            last_graphics_present_ms: 0,
            graphics_present_pending: false,
            graphics_viewport: default_graphics_viewport(),
            focused: true,
            old_guest_cursor_rect: None,
            new_guest_cursor_rect: None,
            next_wake_deadline: None,
            wake_evidence: VecDeque::with_capacity(WAKE_EVIDENCE_CAPACITY),
            wake_sequence: 0,
            mouse_input_counters: MouseInputCounters::default(),
            mouse_log_generation: 0,
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
            GuestState::YieldedUntilTimer => self.set_status(b"Yielded until timer"),
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

    fn draw_text_surface(&mut self, canvas: &mut Canvas, rect: Rect, theme: &Theme) {
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.draw_rect(rect, theme.border);
        let surface = rect.inset(SURFACE_INSET);
        canvas.fill_rect(surface, DOS_SURFACE);
        canvas.draw_rect(surface, dos_color(8));

        let x0 = surface.x + ((surface.w as i32 - 80 * DOS_CELL_W) / 2).max(0);
        let y0 = surface.y + ((surface.h as i32 - 25 * DOS_CELL_H) / 2).max(0);
        self.graphics_viewport =
            MouseViewport::new(x0, y0, (80 * DOS_CELL_W) as u32, (25 * DOS_CELL_H) as u32);
        draw_surface_cells(canvas, &self.runtime, x0, y0);
        if self.cursor_visible
            && self.runtime.text_cursor_visible()
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

    fn refresh_graphics_cache(&mut self) -> bool {
        let generation = self.runtime.framebuffer_generation();
        let palette_generation = self.runtime.palette_generation();
        let mode_generation = self.runtime.video_mode_generation();
        if generation == self.converted_framebuffer_generation
            && palette_generation == self.converted_palette_generation
            && mode_generation == self.converted_video_mode_generation
        {
            return false;
        }
        let mut dirty_rows = self.runtime.take_graphics_dirty_rows();
        if palette_generation != self.converted_palette_generation
            || mode_generation != self.converted_video_mode_generation
        {
            dirty_rows = [true; VGA_HEIGHT];
        }
        if !self
            .runtime
            .convert_graphics_rows(&dirty_rows, &mut self.graphics_cache)
        {
            return false;
        }
        self.converted_framebuffer_generation = generation;
        self.converted_palette_generation = palette_generation;
        self.converted_video_mode_generation = mode_generation;
        self.framebuffer_conversions = self.framebuffer_conversions.wrapping_add(1);
        true
    }

    fn draw_graphics_surface(&mut self, canvas: &mut Canvas, rect: Rect, theme: &Theme) {
        self.refresh_graphics_cache();
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.draw_rect(rect, theme.border);
        let surface = rect.inset(SURFACE_INSET);
        canvas.fill_rect(surface, DOS_SURFACE);
        canvas.draw_rect(surface, dos_color(8));
        let (viewport, scale) = graphics_viewport(surface);
        self.graphics_viewport = MouseViewport::new(viewport.x, viewport.y, viewport.w, viewport.h);
        draw_scaled_graphics(canvas, viewport, scale, &self.graphics_cache);
        if self.focused && self.runtime.mouse().cursor_visible() {
            draw_guest_mouse_cursor(
                canvas,
                viewport,
                scale,
                self.runtime.mouse().framebuffer_position(),
            );
        }
    }

    fn guest_cursor_rect(&self) -> Option<Rect> {
        if !self.focused
            || self.runtime.video_mode() != GuestVideoMode::Vga320x200x256
            || !self.runtime.mouse().cursor_visible()
            || self.graphics_viewport.width == 0
            || self.graphics_viewport.height == 0
        {
            return None;
        }
        let scale = (self.graphics_viewport.width / VGA_WIDTH as u32)
            .min(self.graphics_viewport.height / VGA_HEIGHT as u32)
            .max(1);
        let (x, y) = self.runtime.mouse().framebuffer_position();
        Some(Rect::new(
            self.graphics_viewport.x + i32::from(x) * scale as i32,
            self.graphics_viewport.y + i32::from(y) * scale as i32,
            16 * scale,
            16 * scale,
        ))
    }

    fn record_cursor_damage(&mut self, old: Option<Rect>) -> bool {
        let new = self.guest_cursor_rect();
        let changed = old != new;
        if changed {
            self.old_guest_cursor_rect = old;
            self.new_guest_cursor_rect = new;
        }
        changed
    }

    fn handle_mouse_motion(&mut self, x: i32, y: i32) -> bool {
        self.mouse_input_counters.pointer_moves_received = self
            .mouse_input_counters
            .pointer_moves_received
            .wrapping_add(1);
        let old = self.guest_cursor_rect();
        let inside = self.graphics_viewport.contains(x, y);
        let captured = self.runtime.mouse().captured();
        let generation_before = self.runtime.mouse().generation();
        self.runtime
            .inject_mouse_motion(self.graphics_viewport, x, y);
        if inside || captured {
            let (guest_x, guest_y) = self.runtime.mouse().position();
            self.mouse_input_counters.mapped_guest_coordinates = self
                .mouse_input_counters
                .mapped_guest_coordinates
                .wrapping_add(1);
            self.mouse_input_counters.last_mapped_x = guest_x;
            self.mouse_input_counters.last_mapped_y = guest_y;
        } else {
            self.mouse_input_counters.rejected_outside_content = self
                .mouse_input_counters
                .rejected_outside_content
                .wrapping_add(1);
        }
        self.record_mouse_generation_change(generation_before);
        if !self.runtime.mouse().pointer_inside() {
            // Sunlight's current compositor has no region-scoped hidden
            // cursor, so Chronos deliberately keeps the safe host pointer.
            set_client_cursor(CursorShape::Pointer);
        }
        let changed = self.record_cursor_damage(old);
        self.maybe_log_mouse_input(true);
        changed
    }

    fn handle_mouse_button(&mut self, x: i32, y: i32, button: u8, pressed: bool) -> bool {
        self.mouse_input_counters.pointer_buttons_received = self
            .mouse_input_counters
            .pointer_buttons_received
            .wrapping_add(1);
        let old = self.guest_cursor_rect();
        let inside = self.graphics_viewport.contains(x, y);
        let captured = self.runtime.mouse().captured();
        let generation_before = self.runtime.mouse().generation();
        let buttons_before = self.runtime.mouse().buttons().bits();
        self.runtime
            .inject_mouse_button(self.graphics_viewport, x, y, button, pressed);
        if inside || captured {
            let (guest_x, guest_y) = self.runtime.mouse().position();
            self.mouse_input_counters.mapped_guest_coordinates = self
                .mouse_input_counters
                .mapped_guest_coordinates
                .wrapping_add(1);
            self.mouse_input_counters.last_mapped_x = guest_x;
            self.mouse_input_counters.last_mapped_y = guest_y;
        } else {
            self.mouse_input_counters.rejected_outside_content = self
                .mouse_input_counters
                .rejected_outside_content
                .wrapping_add(1);
        }
        let buttons_after = self.runtime.mouse().buttons().bits();
        if buttons_after != buttons_before {
            self.mouse_input_counters.button_edge_updates = self
                .mouse_input_counters
                .button_edge_updates
                .wrapping_add(1);
        } else if inside || captured {
            self.mouse_input_counters.duplicate_button_edges = self
                .mouse_input_counters
                .duplicate_button_edges
                .wrapping_add(1);
        }
        self.record_mouse_generation_change(generation_before);
        let changed = self.record_cursor_damage(old);
        self.maybe_log_mouse_input(true);
        changed
    }

    fn record_mouse_generation_change(&mut self, generation_before: u64) {
        let generation_after = self.runtime.mouse().generation();
        if generation_after != generation_before {
            self.mouse_input_counters.mouse_state_updates = self
                .mouse_input_counters
                .mouse_state_updates
                .wrapping_add(1);
            self.mouse_input_counters.mouse_generation_changes = self
                .mouse_input_counters
                .mouse_generation_changes
                .wrapping_add(generation_after.wrapping_sub(generation_before));
        }
    }

    fn maybe_log_mouse_input(&mut self, pointer_activity: bool) {
        let polls = self.mouse_input_counters.event_route.display_polls;
        let activity = self
            .mouse_input_counters
            .pointer_moves_received
            .wrapping_add(self.mouse_input_counters.pointer_buttons_received);
        let marker = polls.rotate_left(17) ^ activity;
        let should_log = pointer_activity || polls <= 8 || (polls != 0 && polls % 64 == 0);
        if !should_log || marker == self.mouse_log_generation {
            return;
        }
        self.mouse_log_generation = marker;
        #[cfg(not(test))]
        log_mouse_input_counters(&self.runtime, self.mouse_input_counters);
    }

    fn draw_status_bar(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = Rect::new(
            0,
            canvas.height.saturating_sub(FOOTER_H) as i32,
            canvas.width,
            FOOTER_H.min(canvas.height),
        );
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

        let runtime_hint = match self.runtime.video_mode() {
            GuestVideoMode::Text80x25Color => "INT 16h / B8000",
            GuestVideoMode::Vga320x200x256 => "A0000 / Indexed8",
        };
        let hint_size = measure_text(runtime_hint, small);
        draw_text(
            canvas,
            runtime_hint,
            rect.right() - hint_size.w as i32 - 10,
            text_y,
            &TextStyle::new(small, theme.text_dim),
        );
    }

    fn schedule_after_slice(&mut self, now_ms: u64) {
        self.next_wake_deadline = match self.runtime.last_slice_stop_reason() {
            SliceStopReason::BudgetExhausted
                if matches!(self.runtime.state(), GuestState::Running) =>
            {
                Some(now_ms.saturating_add(BUDGET_CONTINUATION_INTERVAL_MS))
            }
            SliceStopReason::YieldedUntilTimer
                if matches!(self.runtime.state(), GuestState::YieldedUntilTimer) =>
            {
                Some(now_ms.saturating_add(INT28_TIMER_INTERVAL_MS))
            }
            _ => None,
        };
    }

    fn record_wake_evidence(&mut self, source: RuntimeWakeSource) {
        self.wake_sequence = self.wake_sequence.wrapping_add(1);
        let evidence = WakeEvidence {
            sequence: self.wake_sequence,
            source,
            guest_state: self.runtime.state_kind(),
            wait_reason: self.runtime.wait_reason(),
            cs: self.runtime.cpu.cs,
            ip: self.runtime.cpu.ip,
            instructions_retired: self.runtime.instructions_retired(),
            next_wake_deadline: self.next_wake_deadline,
            framebuffer_generation: self.runtime.framebuffer_generation(),
            palette_generation: self.runtime.palette_generation(),
            mouse_generation: self.runtime.mouse().generation(),
        };
        if self.wake_evidence.len() == WAKE_EVIDENCE_CAPACITY {
            self.wake_evidence.pop_front();
        }
        self.wake_evidence.push_back(evidence);
        #[cfg(not(test))]
        if evidence.sequence <= WAKE_EVIDENCE_CAPACITY as u64 {
            log_wake_evidence(evidence);
        }
    }

    fn run_scheduled_slice(
        &mut self,
        now_ms: u64,
        budget: usize,
        source: RuntimeWakeSource,
    ) -> Option<bool> {
        if matches!(self.runtime.state(), GuestState::YieldedUntilTimer) {
            let deadline_reached = self
                .next_wake_deadline
                .is_some_and(|deadline| now_ms >= deadline);
            if source != RuntimeWakeSource::Int28Timer || !deadline_reached {
                return None;
            }
            if !self.runtime.wake_from_timer() {
                return None;
            }
        }
        if !matches!(
            self.runtime.state(),
            GuestState::Ready | GuestState::Running
        ) {
            return None;
        }

        let changed = self.runtime.run_slice(budget);
        self.schedule_after_slice(now_ms);
        self.record_wake_evidence(source);
        Some(changed)
    }

    fn due_wake_source(&self, now_ms: u64) -> Option<RuntimeWakeSource> {
        match self.runtime.state() {
            GuestState::Ready => Some(RuntimeWakeSource::Startup),
            GuestState::Running
                if self
                    .next_wake_deadline
                    .is_none_or(|deadline| now_ms >= deadline) =>
            {
                Some(RuntimeWakeSource::BudgetContinuation)
            }
            GuestState::YieldedUntilTimer
                if self
                    .next_wake_deadline
                    .is_some_and(|deadline| now_ms >= deadline) =>
            {
                Some(RuntimeWakeSource::Int28Timer)
            }
            _ => None,
        }
    }

    fn poll_timeout_at(&self, now_ms: u64) -> u64 {
        if matches!(
            self.runtime.state(),
            GuestState::Ready | GuestState::Running
        ) {
            // Do not put a runnable guest behind display EVENT_POLL. The UI
            // framework interprets zero as a local Tick, so budget continuations
            // remain independent of display, mouse, and keyboard traffic.
            return 0;
        }
        if self
            .next_wake_deadline
            .is_some_and(|deadline| now_ms >= deadline)
        {
            return 0;
        }
        self.next_wake_deadline.map_or_else(
            || 200,
            |deadline| deadline.saturating_sub(now_ms).clamp(1, 200),
        )
    }

    /// Native pointer traffic can be continuous, especially with high-rate
    /// devices. Advance one ordinary bounded slice after every delivered
    /// event so motion cannot starve shell startup, keyboard handling, or a
    /// polling graphics guest waiting to observe the new mouse state.
    fn advance_after_native_event(
        &mut self,
        event_changed: bool,
        source: RuntimeWakeSource,
    ) -> bool {
        let now_ms = monotonic_millis();
        let guest_changed = self
            .run_scheduled_slice(now_ms, INSTRUCTIONS_PER_TICK, source)
            .unwrap_or(false);
        event_changed | guest_changed
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
        let width = canvas.width;
        let height = canvas.height;
        canvas.fill_rect(Rect::new(0, 0, width, height), theme.bg);
        Panel::new(Rect::new(0, 0, width, HEADER_H.min(height))).draw(canvas, theme);
        draw_text(
            canvas,
            self.title.as_str(),
            PAD,
            7,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text(
            canvas,
            match self.runtime.video_mode() {
                GuestVideoMode::Text80x25Color => "16-bit real-mode guest",
                GuestVideoMode::Vga320x200x256 => "VGA 13h 320x200 - guest framebuffer",
            },
            PAD,
            25,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        let content_height = height
            .saturating_sub(HEADER_H)
            .saturating_sub(FOOTER_H)
            .saturating_sub(PAD as u32 * 2);
        let surface_rect = Rect::new(
            PAD,
            HEADER_H as i32 + PAD,
            width.saturating_sub(PAD as u32 * 2),
            content_height,
        );
        match self.runtime.video_mode() {
            GuestVideoMode::Text80x25Color => self.draw_text_surface(canvas, surface_rect, theme),
            GuestVideoMode::Vga320x200x256 => {
                self.draw_graphics_surface(canvas, surface_rect, theme)
            }
        }
        self.draw_status_bar(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                let old_guest_cursor = self.guest_cursor_rect();
                let cursor_active = matches!(
                    self.runtime.state(),
                    GuestState::WaitingForInput
                        | GuestState::Running
                        | GuestState::YieldedUntilTimer
                ) && self.runtime.text_cursor_visible();
                let cursor_visible = cursor_active && (monotonic_millis() / 500) % 2 == 0;
                let cursor_changed = self.cursor_visible != cursor_visible;
                self.cursor_visible = cursor_visible;
                let now = monotonic_millis();
                if let Some(wake_source) = self.due_wake_source(now) {
                    let mode_before = self.runtime.video_mode();
                    let text_or_state_changed = self
                        .run_scheduled_slice(now, INSTRUCTIONS_PER_TICK, wake_source)
                        .unwrap_or(false);
                    #[cfg(debug_assertions)]
                    if self.runtime.dac_entries_committed_last_slice() != 0 {
                        debug_log("[CHRONOS] DAC entries changed this slice: ");
                        debug_log_u32(self.runtime.dac_entries_committed_last_slice());
                        debug_log("; palette generation ");
                        debug_log_u32(self.runtime.palette_generation().min(u32::MAX as u64) as u32);
                        debug_log("\n");
                    }
                    if self.runtime.video_mode() != mode_before {
                        log_video_state(&self.runtime);
                    }
                    let child_failed = if let Some(trap) = self.runtime.take_recovered_child_trap()
                    {
                        debug_log("[CHRONOS] child failed; text mode restored: ");
                        debug_log(trap.summary());
                        debug_log("\n");
                        true
                    } else {
                        false
                    };
                    let status_changed = if child_failed {
                        self.set_status(b"Child failed; shell resumed")
                    } else {
                        self.update_status_from_runtime()
                    };
                    if text_or_state_changed {
                        match self.runtime.video_mode() {
                            GuestVideoMode::Text80x25Color => self.text_present_pending = true,
                            GuestVideoMode::Vga320x200x256 => self.graphics_present_pending = true,
                        }
                    }
                    if self.runtime.video_mode() == GuestVideoMode::Text80x25Color
                        && self.text_present_pending
                    {
                        self.text_instructions_since_present = self
                            .text_instructions_since_present
                            .saturating_add(INSTRUCTIONS_PER_TICK);
                    }
                    let mode_changed = mode_before != self.runtime.video_mode();
                    let guest_running = matches!(
                        self.runtime.state(),
                        GuestState::Running | GuestState::YieldedUntilTimer
                    );
                    let presentation_changed = match self.runtime.video_mode() {
                        GuestVideoMode::Text80x25Color => {
                            self.graphics_present_pending = false;
                            if self.text_present_pending {
                                let due = text_presentation_due(
                                    &mut self.last_text_present_ms,
                                    now,
                                    mode_changed,
                                    guest_running,
                                    self.text_instructions_since_present,
                                );
                                if due {
                                    self.text_present_pending = false;
                                    self.text_instructions_since_present = 0;
                                }
                                due
                            } else {
                                false
                            }
                        }
                        GuestVideoMode::Vga320x200x256 => {
                            self.text_present_pending = false;
                            self.text_instructions_since_present = 0;
                            if self.graphics_present_pending {
                                let due = running_presentation_due(
                                    &mut self.last_graphics_present_ms,
                                    now,
                                    mode_changed,
                                    guest_running,
                                );
                                if due {
                                    self.graphics_present_pending = false;
                                }
                                due
                            } else {
                                false
                            }
                        }
                    };
                    if matches!(self.runtime.state(), GuestState::Exited { .. }) && !self.persisted
                    {
                        self.persisted = true;
                        #[cfg(not(test))]
                        if let Some(storage) = &self.storage {
                            persist_drives(&self.runtime, storage);
                        }
                    }
                    self.record_cursor_damage(old_guest_cursor)
                        || cursor_changed
                        || presentation_changed
                        || status_changed
                } else {
                    self.record_cursor_damage(old_guest_cursor) || cursor_changed
                }
            }
            Event::MouseMove { x, y } => {
                let changed = self.handle_mouse_motion(x, y);
                self.advance_after_native_event(changed, RuntimeWakeSource::MouseMotion)
            }
            Event::MouseDown { x, y, button } => {
                let changed = self.handle_mouse_button(x, y, button, true);
                self.advance_after_native_event(changed, RuntimeWakeSource::MouseButton)
            }
            Event::MouseUp { x, y, button } => {
                let changed = self.handle_mouse_button(x, y, button, false);
                self.advance_after_native_event(changed, RuntimeWakeSource::MouseButton)
            }
            Event::Click { x, y } => {
                let changed = self.handle_mouse_button(x, y, 0, false);
                self.advance_after_native_event(changed, RuntimeWakeSource::MouseButton)
            }
            Event::FocusChanged { focused } => {
                let old = self.guest_cursor_rect();
                self.focused = focused;
                self.runtime.mouse_focus_changed(focused);
                set_client_cursor(CursorShape::Pointer);
                let changed = self.record_cursor_damage(old);
                self.advance_after_native_event(changed, RuntimeWakeSource::FocusChanged)
            }
            Event::PointerOwnership { owned, .. } => {
                if !owned {
                    if self.runtime.mouse().captured() {
                        self.runtime.mouse_pointer_delivery_lost();
                    } else {
                        self.runtime.mouse_pointer_left();
                    }
                    set_client_cursor(CursorShape::Pointer);
                }
                self.advance_after_native_event(false, RuntimeWakeSource::PointerOwnership)
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
                let input_changed = self.runtime.inject_key(BiosKey {
                    ascii,
                    scan_code: 0,
                });
                let changed = input_changed | self.update_status_from_runtime();
                self.advance_after_native_event(changed, RuntimeWakeSource::Keyboard)
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
                    let input_changed = self.runtime.inject_key(key);
                    let changed = input_changed | self.update_status_from_runtime();
                    self.advance_after_native_event(changed, RuntimeWakeSource::Keyboard)
                } else {
                    self.advance_after_native_event(false, RuntimeWakeSource::Keyboard)
                }
            }
            _ => self.advance_after_native_event(false, RuntimeWakeSource::NativeEvent),
        }
    }

    fn on_ready(&mut self) -> bool {
        if !matches!(
            self.runtime.state(),
            GuestState::Ready | GuestState::Running
        ) {
            return false;
        }

        // The window framework has already committed its first native frame.
        // Give the guest one bounded startup burst before entering the normal
        // event-poll loop so a text shell can reach its first BIOS/DOS input
        // wait without exposing or becoming stranded on an intermediate
        // banner frame. Blocking input and INT 28h both stop run_slice early.
        let old_guest_cursor = self.guest_cursor_rect();
        let mode_before = self.runtime.video_mode();
        let now = monotonic_millis();
        let runtime_changed = self
            .run_scheduled_slice(now, STARTUP_INSTRUCTION_BUDGET, RuntimeWakeSource::Startup)
            .unwrap_or(false);
        if self.runtime.video_mode() != mode_before {
            log_video_state(&self.runtime);
        }
        let child_failed = if let Some(trap) = self.runtime.take_recovered_child_trap() {
            debug_log("[CHRONOS] startup child failed; text mode restored: ");
            debug_log(trap.summary());
            debug_log("\n");
            true
        } else {
            false
        };
        let status_changed = if child_failed {
            self.set_status(b"Child failed; shell resumed")
        } else {
            self.update_status_from_runtime()
        };
        if self.runtime.video_mode() == GuestVideoMode::Vga320x200x256 {
            self.graphics_present_pending = false;
        } else {
            self.text_present_pending = false;
            self.text_instructions_since_present = 0;
        }
        if matches!(self.runtime.state(), GuestState::Exited { .. }) && !self.persisted {
            self.persisted = true;
            #[cfg(not(test))]
            if let Some(storage) = &self.storage {
                persist_drives(&self.runtime, storage);
            }
        }
        let cursor_changed = self.record_cursor_damage(old_guest_cursor);
        runtime_changed || status_changed || cursor_changed
    }

    fn poll_timeout_ms(&self) -> u64 {
        self.poll_timeout_at(monotonic_millis())
    }

    fn event_poll_counters(&mut self, counters: EventPollCounters) {
        self.mouse_input_counters.event_route = counters;
        self.maybe_log_mouse_input(false);
    }
}

fn graphics_surface_rect(width: u32, height: u32) -> Rect {
    let content_height = height
        .saturating_sub(HEADER_H)
        .saturating_sub(FOOTER_H)
        .saturating_sub(PAD as u32 * 2);
    Rect::new(
        PAD,
        HEADER_H as i32 + PAD,
        width.saturating_sub(PAD as u32 * 2),
        content_height,
    )
}

fn default_graphics_viewport() -> MouseViewport {
    let surface = graphics_surface_rect(WIN_W, WIN_H).inset(SURFACE_INSET);
    let (viewport, _) = graphics_viewport(surface);
    MouseViewport::new(viewport.x, viewport.y, viewport.w, viewport.h)
}

#[cfg(not(test))]
fn log_wake_evidence(evidence: WakeEvidence) {
    let mut line = [0u8; 256];
    let mut length = copy_bytes(b"[CHRONOS-SCHED] seq=", &mut line);
    length += write_decimal_u64(evidence.sequence, &mut line[length..]);
    length += copy_bytes(b" wake=", &mut line[length..]);
    length += copy_bytes(wake_source_name(evidence.source), &mut line[length..]);
    length += copy_bytes(b" state=", &mut line[length..]);
    length += copy_bytes(state_kind_name(evidence.guest_state), &mut line[length..]);
    length += copy_bytes(b" wait=", &mut line[length..]);
    length += copy_bytes(wait_reason_name(evidence.wait_reason), &mut line[length..]);
    length += copy_bytes(b" cs:ip=", &mut line[length..]);
    length += write_hex_u16(evidence.cs, &mut line[length..]);
    line[length] = b':';
    length += 1;
    length += write_hex_u16(evidence.ip, &mut line[length..]);
    length += copy_bytes(b" retired=", &mut line[length..]);
    length += write_decimal_u64(evidence.instructions_retired, &mut line[length..]);
    length += copy_bytes(b" deadline=", &mut line[length..]);
    if let Some(deadline) = evidence.next_wake_deadline {
        length += write_decimal_u64(deadline, &mut line[length..]);
    } else {
        length += copy_bytes(b"none", &mut line[length..]);
    }
    length += copy_bytes(b" fb=", &mut line[length..]);
    length += write_decimal_u64(evidence.framebuffer_generation, &mut line[length..]);
    length += copy_bytes(b" palette=", &mut line[length..]);
    length += write_decimal_u64(evidence.palette_generation, &mut line[length..]);
    length += copy_bytes(b" mouse=", &mut line[length..]);
    length += write_decimal_u64(evidence.mouse_generation, &mut line[length..]);
    line[length] = b'\n';
    length += 1;
    if let Ok(text) = core::str::from_utf8(&line[..length]) {
        debug_log(text);
    }
}

#[cfg(not(test))]
fn log_mouse_input_counters(runtime: &Runtime, counters: MouseInputCounters) {
    let mut line = [0u8; 512];
    let mut length = copy_bytes(b"[CHRONOS-MOUSE] win=", &mut line);
    length += write_decimal_u64(counters.event_route.window_id, &mut line[length..]);
    length += copy_bytes(b" polls=", &mut line[length..]);
    length += write_decimal_u64(counters.event_route.display_polls, &mut line[length..]);
    length += copy_bytes(b" available=", &mut line[length..]);
    length += write_decimal_u64(counters.event_route.events_available, &mut line[length..]);
    length += copy_bytes(b" dequeued=", &mut line[length..]);
    length += write_decimal_u64(counters.event_route.events_dequeued, &mut line[length..]);
    length += copy_bytes(b" wrong_window=", &mut line[length..]);
    length += write_decimal_u64(
        counters.event_route.wrong_window_replies,
        &mut line[length..],
    );
    length += copy_bytes(b" local_ticks=", &mut line[length..]);
    length += write_decimal_u64(counters.event_route.local_ticks, &mut line[length..]);
    length += copy_bytes(b" interleaved_polls=", &mut line[length..]);
    length += write_decimal_u64(counters.event_route.interleaved_polls, &mut line[length..]);
    length += copy_bytes(b" moves=", &mut line[length..]);
    length += write_decimal_u64(counters.pointer_moves_received, &mut line[length..]);
    length += copy_bytes(b" buttons=", &mut line[length..]);
    length += write_decimal_u64(counters.pointer_buttons_received, &mut line[length..]);
    length += copy_bytes(b" outside=", &mut line[length..]);
    length += write_decimal_u64(counters.rejected_outside_content, &mut line[length..]);
    length += copy_bytes(b" mapped=", &mut line[length..]);
    length += write_decimal_u64(counters.mapped_guest_coordinates, &mut line[length..]);
    length += copy_bytes(b" guest=", &mut line[length..]);
    length += write_decimal_u64(counters.last_mapped_x as u64, &mut line[length..]);
    line[length] = b',';
    length += 1;
    length += write_decimal_u64(counters.last_mapped_y as u64, &mut line[length..]);
    length += copy_bytes(b" updates=", &mut line[length..]);
    length += write_decimal_u64(counters.mouse_state_updates, &mut line[length..]);
    length += copy_bytes(b" gen_changes=", &mut line[length..]);
    length += write_decimal_u64(counters.mouse_generation_changes, &mut line[length..]);
    length += copy_bytes(b" mouse_gen=", &mut line[length..]);
    length += write_decimal_u64(runtime.mouse().generation(), &mut line[length..]);
    length += copy_bytes(b" edges=", &mut line[length..]);
    length += write_decimal_u64(counters.button_edge_updates, &mut line[length..]);
    length += copy_bytes(b" duplicate_edges=", &mut line[length..]);
    length += write_decimal_u64(counters.duplicate_button_edges, &mut line[length..]);
    length += copy_bytes(b" int33_0003=", &mut line[length..]);
    length += write_decimal_u64(
        runtime.mouse().int33_state_query_count(),
        &mut line[length..],
    );
    let (buttons, x, y) = runtime.mouse().last_state_query();
    length += copy_bytes(b" bx_cx_dx=", &mut line[length..]);
    length += write_decimal_u64(buttons as u64, &mut line[length..]);
    line[length] = b',';
    length += 1;
    length += write_decimal_u64(x as u64, &mut line[length..]);
    line[length] = b',';
    length += 1;
    length += write_decimal_u64(y as u64, &mut line[length..]);
    length += copy_bytes(b" state=", &mut line[length..]);
    length += copy_bytes(state_kind_name(runtime.state_kind()), &mut line[length..]);
    line[length] = b'\n';
    length += 1;
    if let Ok(text) = core::str::from_utf8(&line[..length]) {
        debug_log(text);
    }
}

#[cfg(not(test))]
const fn wake_source_name(source: RuntimeWakeSource) -> &'static [u8] {
    match source {
        RuntimeWakeSource::Startup => b"startup",
        RuntimeWakeSource::BudgetContinuation => b"budget-continuation",
        RuntimeWakeSource::Int28Timer => b"int28-timer",
        RuntimeWakeSource::Keyboard => b"keyboard",
        RuntimeWakeSource::MouseMotion => b"mouse-motion",
        RuntimeWakeSource::MouseButton => b"mouse-button",
        RuntimeWakeSource::FocusChanged => b"focus",
        RuntimeWakeSource::PointerOwnership => b"pointer-ownership",
        RuntimeWakeSource::NativeEvent => b"native-event",
    }
}

#[cfg(not(test))]
const fn state_kind_name(state: GuestStateKind) -> &'static [u8] {
    match state {
        GuestStateKind::Ready => b"Ready",
        GuestStateKind::Running => b"Running",
        GuestStateKind::WaitingForInput => b"WaitingForInput",
        GuestStateKind::YieldedUntilTimer => b"YieldedUntilTimer",
        GuestStateKind::Exited => b"Exited",
        GuestStateKind::Halted => b"Halted",
        GuestStateKind::Trapped => b"Trapped",
    }
}

#[cfg(not(test))]
const fn wait_reason_name(reason: GuestWaitReason) -> &'static [u8] {
    match reason {
        GuestWaitReason::None => b"none",
        GuestWaitReason::KeyboardInput => b"keyboard",
        GuestWaitReason::CooperativeTimer => b"timer",
        GuestWaitReason::Stopped => b"stopped",
    }
}

#[cfg(not(test))]
fn write_decimal_u64(mut value: u64, output: &mut [u8]) -> usize {
    let mut reversed = [0u8; 20];
    let mut count = 0;
    loop {
        reversed[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for index in 0..count {
        output[index] = reversed[count - index - 1];
    }
    count
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
    if let chronos_core::Trap::UnsupportedIoPort {
        operation,
        port,
        width,
        value,
        cs,
        ip,
        ..
    } = trap
    {
        length += copy_bytes(
            match operation {
                chronos_core::IoOperation::Read => b" in port=".as_slice(),
                chronos_core::IoOperation::Write => b" out port=".as_slice(),
            },
            &mut line[length..],
        );
        length += write_hex_u16(*port, &mut line[length..]);
        length += copy_bytes(
            match width {
                chronos_core::IoWidth::Byte => b" width=8 cs:ip=".as_slice(),
                chronos_core::IoWidth::Word => b" width=16 cs:ip=".as_slice(),
            },
            &mut line[length..],
        );
        length += write_hex_u16(*cs, &mut line[length..]);
        line[length] = b':';
        length += 1;
        length += write_hex_u16(*ip, &mut line[length..]);
        if let Some(value) = value {
            length += copy_bytes(b" value=", &mut line[length..]);
            length += write_hex_u16(*value, &mut line[length..]);
        }
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

fn log_video_state(runtime: &Runtime) {
    match runtime.video_mode() {
        GuestVideoMode::Text80x25Color => {
            debug_log("[CHRONOS] Video mode: 03h; 80x25; framebuffer B8000\n");
        }
        GuestVideoMode::Vga320x200x256 => {
            debug_log(
                "[CHRONOS] Video mode: 13h; 320x200; framebuffer A000:0000; palette default VGA; dirty rows ",
            );
            let dirty = runtime.memory.graphics_dirty_rows();
            let first = dirty.iter().position(|row| *row).unwrap_or(0);
            let last = dirty.iter().rposition(|row| *row).unwrap_or(0);
            debug_log_u32(first as u32);
            debug_log("-");
            debug_log_u32(last as u32);
            debug_log("; generation ");
            debug_log_u32(runtime.framebuffer_generation().min(u32::MAX as u64) as u32);
            debug_log("\n");
        }
    }
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

fn graphics_viewport(surface: Rect) -> (Rect, u32) {
    let horizontal_scale = surface.w / VGA_WIDTH as u32;
    let vertical_scale = surface.h / VGA_HEIGHT as u32;
    let scale = horizontal_scale.min(vertical_scale).max(1);
    let width = VGA_WIDTH as u32 * scale;
    let height = VGA_HEIGHT as u32 * scale;
    let x = surface.x + (surface.w as i32 - width as i32) / 2;
    let y = surface.y + (surface.h as i32 - height as i32) / 2;
    (Rect::new(x, y, width, height), scale)
}

fn running_presentation_due(
    last_present_ms: &mut u64,
    now_ms: u64,
    mode_changed: bool,
    guest_running: bool,
) -> bool {
    let due = mode_changed
        || !guest_running
        || now_ms.saturating_sub(*last_present_ms) >= RUNNING_PRESENT_INTERVAL_MS;
    if due {
        *last_present_ms = now_ms;
    }
    due
}

fn text_presentation_due(
    last_present_ms: &mut u64,
    now_ms: u64,
    mode_changed: bool,
    guest_running: bool,
    instructions_since_present: usize,
) -> bool {
    let enough_work = instructions_since_present >= TEXT_PRESENT_INSTRUCTION_QUANTUM;
    let enough_time = now_ms.saturating_sub(*last_present_ms) >= RUNNING_PRESENT_INTERVAL_MS;
    let due = mode_changed || !guest_running || (enough_work && enough_time);
    if due {
        *last_present_ms = now_ms;
    }
    due
}

fn draw_scaled_graphics(canvas: &mut Canvas, viewport: Rect, scale: u32, pixels: &[Rgb8]) {
    if pixels.len() < VGA_FRAMEBUFFER_BYTES || scale == 0 {
        return;
    }
    for y in 0..VGA_HEIGHT {
        for x in 0..VGA_WIDTH {
            let rgb = pixels[y * VGA_WIDTH + x];
            canvas.fill_rect(
                Rect::new(
                    viewport.x + x as i32 * scale as i32,
                    viewport.y + y as i32 * scale as i32,
                    scale,
                    scale,
                ),
                Color::rgb(rgb.r, rgb.g, rgb.b),
            );
        }
    }
}

/// A fixed-hotspot 16x16 classic DOS arrow. It is composited after indexed
/// conversion and therefore never enters guest memory or either guest
/// generation counter. Bit 15 is the leftmost pixel.
const DOS_MOUSE_CURSOR_OUTLINE: [u16; 16] = [
    0x8000, 0xc000, 0xe000, 0xf000, 0xf800, 0xfc00, 0xfe00, 0xff00, 0xff80, 0xfc00, 0xdc00, 0x8e00,
    0x0600, 0x0700, 0x0300, 0x0000,
];
const DOS_MOUSE_CURSOR_FILL: [u16; 16] = [
    0x0000, 0x4000, 0x6000, 0x7000, 0x7800, 0x7c00, 0x7e00, 0x7f00, 0x7c00, 0x5800, 0x0c00, 0x0400,
    0x0200, 0x0200, 0x0000, 0x0000,
];

fn draw_guest_mouse_cursor(
    canvas: &mut Canvas,
    viewport: Rect,
    scale: u32,
    framebuffer_position: (u16, u16),
) {
    if scale == 0 {
        return;
    }
    let origin_x = viewport.x + i32::from(framebuffer_position.0) * scale as i32;
    let origin_y = viewport.y + i32::from(framebuffer_position.1) * scale as i32;
    for (mask, color) in [
        (&DOS_MOUSE_CURSOR_OUTLINE, Color::rgb(0, 0, 0)),
        (&DOS_MOUSE_CURSOR_FILL, Color::rgb(255, 255, 255)),
    ] {
        for (row, bits) in mask.iter().copied().enumerate() {
            for column in 0..16 {
                if bits & (0x8000 >> column) == 0 {
                    continue;
                }
                let pixel = Rect::new(
                    origin_x + column * scale as i32,
                    origin_y + row as i32 * scale as i32,
                    scale,
                    scale,
                );
                if let Some(clipped) = pixel.intersect(viewport) {
                    canvas.fill_rect(clipped, color);
                }
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
    // Use the bundle-provided title (e.g. "Sunlight Mines") for ordinary
    // graphical application bundles. The generic terminal title is only for
    // the default interactive shell when launched without bundle metadata.
    // WindowConfig requires &'static str. Leak the title string for bundle launches
    // so that "Sunlight Mines" (or other bundle name) appears as the OS window title.
    let window_title: &'static str = if app.title == "Chronos - Sunlight DOS Terminal" {
        "Chronos - Sunlight DOS Terminal"
    } else {
        let s = app.title.clone();
        &*alloc::boxed::Box::leak(s.into_boxed_str())
    };
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: window_title,
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
    use super::{
        dos_color, draw_scaled_graphics, graphics_viewport, running_presentation_due, ChronosApp,
        RuntimeWakeSource, DOS_CELL_H, DOS_CELL_W, DOS_SURFACE, WIN_H, WIN_W,
    };
    use alloc::{format, vec};
    use chronos_core::{
        BiosKey, DosDrive, GuestState, GuestStateKind, GuestVideoMode, GuestWaitReason,
        GuestWakeSource, Rgb8, Runtime, SliceStopReason,
    };
    use sun_font::{measure_text, FontRole};
    use sunlight_ui::{App, Canvas, Color, Event, Rect, Theme};

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
    fn native_tick_loop_reaches_and_renders_the_bundled_shell_prompt() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let autoexec = include_bytes!("../../ChronosDosShell.sunapp/Program/AUTOEXEC.BAT");
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_program(shell, b"").unwrap();
        app.runtime
            .set_application_id(b"org.sunlight.chronos.shell");
        app.runtime.set_executable_path(b"C:\\SUNSH.EXE");
        app.runtime
            .drives_mut()
            .add_base_file(DosDrive::C, "AUTOEXEC.BAT", autoexec.to_vec())
            .unwrap();

        assert!(app.on_ready());

        assert_eq!(
            app.runtime.state(),
            &GuestState::WaitingForInput,
            "cpu={:?}, cursor=({}, {})",
            app.runtime.cpu,
            app.runtime.cursor_column(),
            app.runtime.cursor_row()
        );
        let text: [u8; 2000] =
            core::array::from_fn(|index| app.runtime.cell(index % 80, index / 80).character);
        assert!(text
            .windows(b"CMD C:\\>".len())
            .any(|window| window == b"CMD C:\\>"));

        let mut framebuffer = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut framebuffer, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());
        let surface_color = DOS_SURFACE.0;
        assert!(framebuffer.iter().any(|pixel| *pixel != surface_color));
    }

    #[test]
    fn native_startup_burst_stops_at_the_first_cooperative_idle_hint() {
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_com(&[
            0xcd, 0x28, // DOS cooperative idle hint
            0xeb, 0xfc, // repeat forever, one yield per native slice
        ])
        .unwrap();

        assert!(app.on_ready());
        assert_eq!(app.runtime.state(), &GuestState::YieldedUntilTimer);
        assert!(app.runtime.cooperative_yielded_last_slice());
        assert_eq!(app.runtime.cooperative_yield_count(), 1);
    }

    fn standalone_mines_app() -> ChronosApp {
        let image = include_bytes!("../../SunlightMines.sunapp/Program/SUNMINE.EXE");
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_program(image, b"").unwrap();
        app.runtime
            .set_application_id(b"org.sunlight.chronos.mines");
        app.runtime.set_executable_path(b"C:\\SUNMINE.EXE");
        app
    }

    fn run_mines_until_timer_yield(app: &mut ChronosApp, mut now_ms: u64, budget: usize) {
        let mut slices = 0usize;
        while matches!(app.runtime.state(), GuestState::Ready | GuestState::Running) {
            let source = if matches!(app.runtime.state(), GuestState::Ready) {
                RuntimeWakeSource::Startup
            } else {
                RuntimeWakeSource::BudgetContinuation
            };
            app.run_scheduled_slice(now_ms, budget, source)
                .expect("scheduled Mines slice");
            slices += 1;
            now_ms = app.next_wake_deadline.unwrap_or(now_ms.saturating_add(1));
            assert!(
                slices < 10_000,
                "Mines did not reach its timer yield: cpu={:?} retired={}",
                app.runtime.cpu,
                app.runtime.instructions_retired()
            );
        }
        assert_eq!(app.runtime.state(), &GuestState::YieldedUntilTimer);
    }

    fn deliver_mines_event(app: &mut ChronosApp, event: Event, now_ms: u64) {
        app.update(event);
        let continuation = app.next_wake_deadline.unwrap_or(now_ms);
        run_mines_until_timer_yield(app, continuation, 2_048);
    }

    #[test]
    fn no_input_budget_continuations_advance_cs_ip_and_retirement() {
        let mut app = standalone_mines_app();
        let mouse_generation = app.runtime.mouse().generation();

        assert!(app
            .run_scheduled_slice(1_000, 1, RuntimeWakeSource::Startup)
            .is_some());
        let first = *app.wake_evidence.back().unwrap();
        assert_eq!(first.source, RuntimeWakeSource::Startup);
        assert_eq!(first.guest_state, GuestStateKind::Running);
        assert_eq!(first.wait_reason, GuestWaitReason::None);
        assert_eq!(first.instructions_retired, 1);
        assert_eq!(first.mouse_generation, mouse_generation);
        assert_eq!(first.next_wake_deadline, Some(1_000));

        assert!(app
            .run_scheduled_slice(1_000, 1, RuntimeWakeSource::BudgetContinuation)
            .is_some());
        let second = *app.wake_evidence.back().unwrap();
        assert_eq!(second.source, RuntimeWakeSource::BudgetContinuation);
        assert_eq!(second.instructions_retired, 2);
        assert_ne!((second.cs, second.ip), (first.cs, first.ip));
        assert_eq!(second.mouse_generation, mouse_generation);
        assert_eq!(
            app.runtime.last_slice_stop_reason(),
            SliceStopReason::BudgetExhausted
        );
    }

    #[test]
    fn standalone_mines_completes_initial_frame_without_native_input() {
        let mut app = standalone_mines_app();
        let mut now_ms = 10_000;
        let mut slices = 0usize;

        loop {
            let source = if slices == 0 {
                RuntimeWakeSource::Startup
            } else {
                RuntimeWakeSource::BudgetContinuation
            };
            assert!(app.run_scheduled_slice(now_ms, 2_048, source).is_some());
            slices += 1;
            if app.runtime.state() == &GuestState::YieldedUntilTimer {
                break;
            }
            assert_eq!(app.runtime.state(), &GuestState::Running);
            now_ms = app.next_wake_deadline.expect("running guest continuation");
            assert!(
                slices < 10_000,
                "standalone Mines startup did not reach INT 28h: cpu={:?} retired={}",
                app.runtime.cpu,
                app.runtime.instructions_retired()
            );
        }

        assert!(slices > 1, "test must cover a frame split across slices");
        assert_eq!(app.runtime.wait_reason(), GuestWaitReason::CooperativeTimer);
        assert!(app.wake_evidence.iter().all(|entry| matches!(
            entry.source,
            RuntimeWakeSource::Startup | RuntimeWakeSource::BudgetContinuation
        )));
        assert!(app.runtime.instructions_retired() > 2_048);
        assert_eq!(app.runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert!(!app.runtime.text_cursor_visible());
        let title: [u8; 14] =
            core::array::from_fn(|column| app.runtime.cell(29 + column, 0).character);
        assert_eq!(&title, b"SUNLIGHT MINES");

        let deadline = app.next_wake_deadline.expect("INT 28h timer deadline");
        assert_eq!(
            app.poll_timeout_at(deadline - 1),
            1,
            "YieldedUntilTimer must remain in bounded display polling before its wake"
        );
        assert_eq!(
            app.poll_timeout_at(deadline),
            0,
            "the due timer wake becomes an app-local continuation"
        );
        let retired_at_yield = app.runtime.instructions_retired();
        assert_eq!(
            app.run_scheduled_slice(deadline - 1, 2_048, RuntimeWakeSource::Int28Timer,),
            None
        );
        assert_eq!(app.runtime.instructions_retired(), retired_at_yield);
        assert!(app
            .run_scheduled_slice(deadline, 2_048, RuntimeWakeSource::Int28Timer)
            .is_some());
        assert!(app.runtime.instructions_retired() > retired_at_yield);
        assert_eq!(app.runtime.last_wake_source(), Some(GuestWakeSource::Timer));
        assert_eq!(
            app.wake_evidence.back().unwrap().source,
            RuntimeWakeSource::Int28Timer
        );
        let mut post_timer_slices = 0usize;
        while app.runtime.state() == &GuestState::Running {
            let continuation = app.next_wake_deadline.expect("post-timer continuation");
            app.run_scheduled_slice(continuation, 2_048, RuntimeWakeSource::BudgetContinuation)
                .expect("post-timer no-input slice");
            post_timer_slices += 1;
            assert!(
                post_timer_slices < 10_000,
                "timer wake did not reach the next idle hint"
            );
        }
        assert_eq!(app.runtime.state(), &GuestState::YieldedUntilTimer);
    }

    #[test]
    fn small_budget_mines_frame_continues_until_complete_without_input() {
        const SMALL_BUDGET: usize = 64;
        let mut app = standalone_mines_app();
        let mut now_ms = 20_000;
        let mut slices = 0usize;
        let mut running_continuations = 0usize;
        let initial_title = app.runtime.cell(29, 0);

        loop {
            let source = if slices == 0 {
                RuntimeWakeSource::Startup
            } else {
                RuntimeWakeSource::BudgetContinuation
            };
            app.run_scheduled_slice(now_ms, SMALL_BUDGET, source)
                .expect("scheduled no-input continuation");
            slices += 1;
            if app.runtime.state() == &GuestState::YieldedUntilTimer {
                break;
            }
            assert_eq!(
                app.runtime.last_slice_stop_reason(),
                SliceStopReason::BudgetExhausted
            );
            running_continuations += 1;
            now_ms = app
                .next_wake_deadline
                .expect("small-budget continuation deadline");
            assert!(
                slices < 10_000,
                "small-budget Mines startup did not converge"
            );
        }

        assert!(running_continuations > 1);
        assert_ne!(app.runtime.cell(29, 0), initial_title);
        assert_eq!(app.runtime.wait_reason(), GuestWaitReason::CooperativeTimer);
        assert!(app.runtime.instructions_retired() > SMALL_BUDGET as u64);
    }

    #[test]
    fn standalone_mines_center_corners_and_button_edges_reach_int33_once() {
        let mut app = standalone_mines_app();
        run_mines_until_timer_yield(&mut app, 30_000, 2_048);
        assert!(app.runtime.mouse().cursor_visible());
        assert_eq!(app.runtime.mouse().ranges(), (0, 79, 0, 24));

        let mut framebuffer = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut framebuffer, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());
        let viewport = app.graphics_viewport;
        let center_x = viewport.x + (31 + 9) * DOS_CELL_W + DOS_CELL_W / 2;
        let center_y = viewport.y + (3 + 4) * DOS_CELL_H + DOS_CELL_H / 2;
        deliver_mines_event(&mut app, Event::mouse_move(center_x, center_y), 30_001);
        let (center_guest_x, center_guest_y) = app.runtime.mouse().position();
        assert_eq!((center_guest_x, center_guest_y), (40, 7));
        assert_eq!(
            app.runtime.mouse().last_state_query(),
            (0, center_guest_x, center_guest_y)
        );

        let corners = [
            (viewport.x, viewport.y, 0, 0),
            (viewport.x + viewport.width as i32 - 1, viewport.y, 79, 0),
            (viewport.x, viewport.y + viewport.height as i32 - 1, 0, 24),
            (
                viewport.x + viewport.width as i32 - 1,
                viewport.y + viewport.height as i32 - 1,
                79,
                24,
            ),
        ];
        for (index, (native_x, native_y, guest_x, guest_y)) in corners.into_iter().enumerate() {
            deliver_mines_event(
                &mut app,
                Event::mouse_move(native_x, native_y),
                30_010 + index as u64,
            );
            assert_eq!(app.runtime.mouse().position(), (guest_x, guest_y));
            assert_eq!(
                app.runtime.mouse().last_state_query(),
                (0, guest_x, guest_y)
            );
        }
        assert_eq!(
            app.runtime.last_wake_source(),
            Some(GuestWakeSource::MouseMotion)
        );

        let cell_before_click = (app.runtime.cell(39, 7), app.runtime.cell(40, 7));
        let query_count_before_click = app.runtime.mouse().int33_state_query_count();
        deliver_mines_event(&mut app, Event::mouse_down(center_x, center_y, 0), 30_020);
        assert_eq!(app.runtime.mouse().buttons().bits(), 1);
        assert_eq!(app.runtime.mouse().last_state_query().0, 1);
        assert_eq!(
            app.runtime.last_wake_source(),
            Some(GuestWakeSource::MouseButton)
        );
        assert_ne!(
            (app.runtime.cell(39, 7), app.runtime.cell(40, 7)),
            cell_before_click
        );
        assert_ne!(app.runtime.cell(39, 7).character, b'*');

        deliver_mines_event(&mut app, Event::click(center_x, center_y), 30_021);
        assert_eq!(app.runtime.mouse().buttons().bits(), 0);
        assert_eq!(app.runtime.mouse().last_state_query().0, 0);
        assert!(app.runtime.mouse().int33_state_query_count() >= query_count_before_click + 2);

        assert_eq!(app.mouse_input_counters.pointer_moves_received, 5);
        assert_eq!(app.mouse_input_counters.pointer_buttons_received, 2);
        assert_eq!(app.mouse_input_counters.rejected_outside_content, 0);
        assert_eq!(app.mouse_input_counters.mapped_guest_coordinates, 7);
        assert_eq!(app.mouse_input_counters.button_edge_updates, 2);
        assert_eq!(app.mouse_input_counters.duplicate_button_edges, 0);
        assert!(app.mouse_input_counters.mouse_state_updates >= 7);
        assert!(app.mouse_input_counters.mouse_generation_changes >= 7);
    }

    #[test]
    fn standalone_mines_right_click_and_restart_update_the_board() {
        let mut app = standalone_mines_app();
        run_mines_until_timer_yield(&mut app, 35_000, 2_048);

        let mut framebuffer = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut framebuffer, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());
        let viewport = app.graphics_viewport;
        let cell_x = viewport.x + 31 * DOS_CELL_W + DOS_CELL_W / 2;
        let cell_y = viewport.y + 3 * DOS_CELL_H + DOS_CELL_H / 2;

        deliver_mines_event(&mut app, Event::mouse_down(cell_x, cell_y, 1), 35_001);
        assert_eq!(app.runtime.mouse().position(), (31, 3));
        assert_eq!(app.runtime.mouse().buttons().bits(), 2);
        assert_eq!(app.runtime.mouse().last_state_query(), (2, 31, 3));
        assert_eq!(app.runtime.cell(31, 3).character, b'F');
        deliver_mines_event(&mut app, Event::mouse_up(cell_x, cell_y, 1), 35_002);
        assert_eq!(app.runtime.cell(31, 3).character, b'F');
        assert_eq!(app.runtime.cell(10, 1).character, b'0');
        assert_eq!(app.runtime.cell(11, 1).character, b'9');

        app.update(Event::Key('r'));
        run_mines_until_timer_yield(&mut app, 35_003, 2_048);
        assert_eq!(app.runtime.cell(31, 3).character, 0xdb);
        assert_eq!(app.runtime.cell(32, 3).character, 0xdb);
        assert_eq!(app.runtime.cell(10, 1).character, b'1');
        assert_eq!(app.runtime.cell(11, 1).character, b'0');
    }

    #[test]
    fn standalone_mines_escape_exits_cleanly() {
        let mut app = standalone_mines_app();
        run_mines_until_timer_yield(&mut app, 40_000, 2_048);

        app.runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        let deadline = app.next_wake_deadline.expect("Mines timer deadline");
        app.run_scheduled_slice(deadline, 2_048, RuntimeWakeSource::Int28Timer)
            .expect("Mines escape wake");
        while app.runtime.state() == &GuestState::Running {
            let continuation = app.next_wake_deadline.expect("escape continuation");
            app.run_scheduled_slice(continuation, 2_048, RuntimeWakeSource::BudgetContinuation)
                .expect("Mines escape continuation");
        }

        assert_eq!(app.runtime.state(), &GuestState::Exited { code: 0 });
        assert!(!app.runtime.mouse().cursor_visible());
    }

    #[test]
    fn native_dir_output_is_coalesced_and_reaches_the_next_prompt() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_program(shell, b"").unwrap();
        for index in 0..80 {
            let name = format!("F{index:07}.TXT");
            app.runtime
                .drives_mut()
                .add_base_file(DosDrive::C, &name, vec![index as u8; 32])
                .unwrap();
        }
        assert!(app.on_ready());
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        for ch in ['d', 'i', 'r'] {
            app.update(Event::Key(ch));
        }
        let mut redraws = usize::from(app.update(Event::Key('\n')));
        let mut slices = 0usize;
        while app.runtime.state() != &GuestState::WaitingForInput && slices < 10_000 {
            redraws += usize::from(app.update(Event::Tick));
            slices += 1;
        }

        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert!(slices < 1_000, "DIR needed {slices} native slices");
        assert!(
            redraws <= 4,
            "DIR requested {redraws} full client frames instead of coalescing"
        );
        let text: [u8; 2000] =
            core::array::from_fn(|index| app.runtime.cell(index % 80, index / 80).character);
        assert!(text
            .windows(b"CMD C:\\>".len())
            .any(|window| window == b"CMD C:\\>"));
    }

    #[test]
    fn text_mode_mouse_state_does_not_request_invisible_frames() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_program(shell, b"").unwrap();
        assert!(app.on_ready());

        assert!(!app.update(Event::MouseMove { x: 210, y: 210 }));
        assert!(!app.update(Event::MouseDown {
            x: 210,
            y: 210,
            button: 0,
        }));
        assert_eq!(app.runtime.mouse().buttons().bits(), 0);
        assert!(!app.update(Event::FocusChanged { focused: false }));
        assert_eq!(app.runtime.mouse().buttons().bits(), 0);
    }

    #[test]
    fn continuous_text_mode_pointer_events_cannot_starve_shell_or_keyboard() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_program(shell, b"").unwrap();

        for sample in 0..10_000i32 {
            app.update(Event::MouseMove {
                x: sample.rem_euclid(WIN_W as i32),
                y: (sample * 7).rem_euclid(WIN_H as i32),
            });
            if app.runtime.state() == &GuestState::WaitingForInput {
                break;
            }
        }
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);

        app.update(Event::Key('h'));
        app.update(Event::Key('e'));
        for sample in 0..1_000i32 {
            app.update(Event::MouseMove {
                x: sample.rem_euclid(WIN_W as i32),
                y: (sample * 7).rem_euclid(WIN_H as i32),
            });
            if app.runtime.state() == &GuestState::WaitingForInput
                && app.runtime.cursor_column() >= b"CMD C:\\>he".len()
            {
                break;
            }
        }
        let text: [u8; 2000] =
            core::array::from_fn(|index| app.runtime.cell(index % 80, index / 80).character);
        assert!(text
            .windows(b"CMD C:\\>he".len())
            .any(|window| window == b"CMD C:\\>he"));
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
        assert_eq!(app.poll_timeout_ms(), 0);
        app.runtime.run_slice(1024);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(app.poll_timeout_ms(), 200);

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

    fn mode13_app() -> ChronosApp {
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_com(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, 0xb8, 0x00, 0xa0, 0x8e, 0xc0, 0x31, 0xff, 0xb0, 0x04,
            0xaa, 0xb4, 0x00, 0xcd, 0x16,
        ])
        .unwrap();
        app.runtime.run_slice(32);
        assert_eq!(app.runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        app
    }

    #[test]
    fn graphics_cache_converts_only_when_mode_framebuffer_or_palette_generation_changes() {
        let mut app = mode13_app();
        assert!(app.refresh_graphics_cache());
        assert_eq!(app.framebuffer_conversions, 1);
        assert_eq!(app.graphics_cache[0], Rgb8::new(170, 0, 0));
        assert!(!app.refresh_graphics_cache());
        assert_eq!(app.framebuffer_conversions, 1);

        app.runtime.memory.write_u8(0xa000, 320, 10);
        assert!(app.refresh_graphics_cache());
        assert_eq!(app.framebuffer_conversions, 2);
        assert_eq!(app.graphics_cache[320], Rgb8::new(85, 255, 85));
        app.runtime.memory.write_u8(0xa000, 320, 10);
        assert!(!app.refresh_graphics_cache());
        assert_eq!(app.framebuffer_conversions, 2);
    }

    #[test]
    fn palette_only_batch_reconverts_once_without_touching_a0000_generation() {
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_com(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, 0xb8, 0x00, 0xa0, 0x8e, 0xc0, 0x31, 0xff, 0xb0, 32, 0xaa,
            0xba, 0xc8, 0x03, 0xb0, 32, 0xee, 0x42, 0xb0, 63, 0xee, 0x30, 0xc0, 0xee, 0xee, 0xf4,
        ])
        .unwrap();
        app.runtime.run_slice(7);
        assert!(app.refresh_graphics_cache());
        assert_eq!(app.framebuffer_conversions, 1);
        let framebuffer_generation = app.runtime.framebuffer_generation();
        let index = app.runtime.framebuffer_index(0, 0);

        app.runtime.run_slice(64);
        assert_eq!(app.runtime.dac_entries_committed_last_slice(), 1);
        assert_eq!(app.runtime.framebuffer_generation(), framebuffer_generation);
        assert_eq!(app.runtime.framebuffer_index(0, 0), index);
        assert!(app.refresh_graphics_cache());
        assert_eq!(app.framebuffer_conversions, 2);
        assert_eq!(app.graphics_cache[0], Rgb8::new(255, 0, 0));
        assert!(!app.refresh_graphics_cache());
        assert_eq!(app.framebuffer_conversions, 2);
    }

    #[test]
    fn running_graphics_presentation_is_batched_and_static_waiting_is_not_polled() {
        let mut last = 0;
        let redraws = (1..=1000)
            .filter(|now| running_presentation_due(&mut last, *now, *now == 1, true))
            .count();
        assert_eq!(redraws, 63);

        let app = mode13_app();
        assert_eq!(
            app.poll_timeout_ms(),
            200,
            "waiting graphics guests stay idle"
        );
    }

    #[test]
    fn viewport_uses_largest_integer_scale_and_centers_without_guest_mutation() {
        let surface = Rect::new(24, 72, 792, 472);
        let (viewport, scale) = graphics_viewport(surface);
        assert_eq!(scale, 2);
        assert_eq!(viewport, Rect::new(100, 108, 640, 400));

        let app = mode13_app();
        let checksum = app.runtime.framebuffer_checksum();
        let (resized, resized_scale) = graphics_viewport(Rect::new(10, 10, 1000, 650));
        assert_eq!(resized_scale, 3);
        assert_eq!(resized.w, 960);
        assert_eq!(resized.h, 600);
        assert_eq!(app.runtime.framebuffer_checksum(), checksum);
    }

    #[test]
    fn nearest_neighbor_renderer_preserves_logical_pixel_blocks() {
        let mut framebuffer = vec![0u32; 8 * 8];
        let mut canvas = Canvas::new(&mut framebuffer, 8, 8, 8);
        let mut pixels = vec![Rgb8::default(); 320 * 200];
        pixels[0] = Rgb8::new(255, 85, 85);
        pixels[1] = Rgb8::new(85, 255, 85);
        draw_scaled_graphics(&mut canvas, Rect::new(0, 0, 640, 400), 2, &pixels);
        assert_eq!(framebuffer[0], Color::rgb(255, 85, 85).0);
        assert_eq!(framebuffer[1], Color::rgb(255, 85, 85).0);
        assert_eq!(framebuffer[8], Color::rgb(255, 85, 85).0);
        assert_eq!(framebuffer[2], Color::rgb(85, 255, 85).0);
        assert_eq!(framebuffer[3], Color::rgb(85, 255, 85).0);
    }

    #[test]
    fn graphics_view_selects_a0000_and_never_draws_the_text_cursor() {
        let mut app = mode13_app();
        app.cursor_visible = true;
        let mut framebuffer = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut framebuffer, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());

        // Logical (0,0) is palette index 4 and occupies an exact 2x2 block at
        // the centered graphics viewport origin. No text/cursor pass follows.
        let red = Color::rgb(170, 0, 0).0;
        for (x, y) in [(100, 108), (101, 108), (100, 109), (101, 109)] {
            assert_eq!(framebuffer[y * WIN_W as usize + x], red);
        }
    }

    #[test]
    fn native_shell_path_renders_vgalab_guest_pixels_then_returns_to_text() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let vgalab = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM");
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_program(shell, b"").unwrap();
        app.runtime
            .drives_mut()
            .add_base_directory(DosDrive::C, "TESTS")
            .unwrap();
        app.runtime
            .drives_mut()
            .add_base_file(DosDrive::C, "TESTS/VGALAB.COM", vgalab.to_vec())
            .unwrap();
        app.runtime.run_slice(10_000_000);
        for ascii in [b'V', b'G', b'A', b'L', b'A', b'B', b'\r'] {
            app.runtime.inject_ascii(ascii);
            app.runtime.run_slice(10_000_000);
        }
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(app.runtime.video_mode(), GuestVideoMode::Vga320x200x256);

        let mut framebuffer = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut framebuffer, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());
        assert_eq!(app.framebuffer_conversions, 1);
        // Viewport origin is (100,108), scale 2. These host pixels correspond
        // exactly to guest checker (24,64) and sun (140,84) indices.
        assert_eq!(
            canvas.pixels[236 * WIN_W as usize + 148],
            Color::rgb(85, 255, 255).0
        );
        assert_eq!(
            canvas.pixels[276 * WIN_W as usize + 380],
            Color::rgb(170, 85, 0).0
        );
        app.view(&mut canvas, &Theme::sunlight_dark());
        assert_eq!(app.framebuffer_conversions, 1);

        app.runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        app.runtime.run_slice(10_000_000);
        assert_eq!(app.runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(app.runtime.state(), &GuestState::WaitingForInput);
        let text: [u8; 2000] =
            core::array::from_fn(|index| app.runtime.cell(index % 80, index / 80).character);
        assert_eq!(
            text.windows(b"CMD C:\\>".len())
                .filter(|window| *window == b"CMD C:\\>")
                .count(),
            1
        );
    }

    #[test]
    fn native_surface_changes_from_guest_dac_outs_while_a0000_stays_static() {
        let palcycle = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM");
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_com(palcycle).unwrap();
        let mut instructions = 0usize;
        while app.runtime.palette_generation() < 33 {
            app.runtime.step();
            instructions += 1;
            assert!(instructions < 1_000_000);
        }
        let checksum = app.runtime.framebuffer_checksum();
        let generation = app.runtime.framebuffer_generation();

        let mut framebuffer = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut framebuffer, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());
        let host_offset = 148 * WIN_W as usize + 100;
        let first_color = canvas.pixels[host_offset];
        assert_eq!(app.framebuffer_conversions, 1);

        while app.runtime.palette_generation() < 65 {
            app.runtime.step();
            instructions += 1;
            assert!(instructions < 1_000_000);
        }
        app.view(&mut canvas, &Theme::sunlight_dark());
        assert_ne!(canvas.pixels[host_offset], first_color);
        assert_eq!(app.framebuffer_conversions, 2);
        assert_eq!(app.runtime.framebuffer_checksum(), checksum);
        assert_eq!(app.runtime.framebuffer_generation(), generation);
        assert_eq!(app.runtime.framebuffer_index(0, 20), Some(32));
    }

    fn mouse_overlay_app() -> ChronosApp {
        let mut app = ChronosApp::new();
        app.runtime = Runtime::from_com(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, // mode 13h
            0xb8, 0x07, 0x00, 0x31, 0xc9, 0xba, 0x3f, 0x01, 0xcd, 0x33, 0xb8, 0x08, 0x00, 0x31,
            0xc9, 0xba, 0xc7, 0x00, 0xcd, 0x33, 0xb8, 0x01, 0x00, 0xcd, 0x33, 0xf4,
        ])
        .unwrap();
        app.runtime.run_slice(64);
        app
    }

    #[test]
    fn guest_cursor_overlay_changes_native_pixels_not_guest_evidence() {
        let mut app = mouse_overlay_app();
        let framebuffer_checksum = app.runtime.framebuffer_checksum();
        let framebuffer_generation = app.runtime.framebuffer_generation();
        let palette_checksum = app.runtime.palette_checksum();
        let palette_generation = app.runtime.palette_generation();

        let mut first = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut first_canvas = Canvas::new(&mut first, WIN_W, WIN_W, WIN_H);
        app.view(&mut first_canvas, &Theme::sunlight_dark());
        let conversions = app.framebuffer_conversions;

        assert!(app.update(Event::MouseMove { x: 210, y: 210 }));
        let mut second = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut second_canvas = Canvas::new(&mut second, WIN_W, WIN_W, WIN_H);
        app.view(&mut second_canvas, &Theme::sunlight_dark());

        assert_ne!(first, second);
        assert_eq!(app.framebuffer_conversions, conversions);
        assert_eq!(app.runtime.framebuffer_checksum(), framebuffer_checksum);
        assert_eq!(app.runtime.framebuffer_generation(), framebuffer_generation);
        assert_eq!(app.runtime.palette_checksum(), palette_checksum);
        assert_eq!(app.runtime.palette_generation(), palette_generation);
        assert!(app.old_guest_cursor_rect.is_some());
        assert!(app.new_guest_cursor_rect.is_some());
        assert_ne!(app.old_guest_cursor_rect, app.new_guest_cursor_rect);
    }

    #[test]
    fn cursor_clips_at_edges_and_focus_loss_clears_guest_drag() {
        let mut app = mouse_overlay_app();
        let mut pixels = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut pixels, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());
        let viewport = app.graphics_viewport;
        let right = viewport.x + viewport.width as i32 - 1;
        let bottom = viewport.y + viewport.height as i32 - 1;
        assert!(app.update(Event::MouseDown {
            x: right,
            y: bottom,
            button: 0,
        }));
        assert!(app.runtime.mouse().captured());
        assert_eq!(app.runtime.mouse().buttons().bits(), 1);
        app.view(&mut canvas, &Theme::sunlight_dark());

        assert!(app.update(Event::FocusChanged { focused: false }));
        assert_eq!(app.runtime.mouse().buttons().bits(), 0);
        assert!(!app.runtime.mouse().captured());
        assert!(app.guest_cursor_rect().is_none());
        app.update(Event::FocusChanged { focused: true });
        assert_eq!(app.runtime.mouse().buttons().bits(), 0);
    }

    #[test]
    fn title_header_status_and_letterbox_events_do_not_reach_guest_mouse() {
        let mut app = mouse_overlay_app();
        let mut pixels = vec![0u32; WIN_W as usize * WIN_H as usize];
        let mut canvas = Canvas::new(&mut pixels, WIN_W, WIN_W, WIN_H);
        app.view(&mut canvas, &Theme::sunlight_dark());
        let initial = app.runtime.mouse().position();
        assert!(!app.update(Event::MouseMove { x: 5, y: 5 }));
        assert_eq!(app.runtime.mouse().position(), initial);
        assert!(!app.update(Event::MouseDown {
            x: 20,
            y: WIN_H as i32 - 2,
            button: 0,
        }));
        assert_eq!(app.runtime.mouse().buttons().bits(), 0);
        let viewport = app.graphics_viewport;
        assert!(!app.update(Event::MouseMove {
            x: viewport.x - 1,
            y: viewport.y,
        }));
        assert_eq!(app.runtime.mouse().position(), initial);
        assert_eq!(app.mouse_input_counters.rejected_outside_content, 3);
        assert_eq!(app.mouse_input_counters.mapped_guest_coordinates, 0);
        assert_eq!(app.mouse_input_counters.mouse_state_updates, 0);
    }
}
