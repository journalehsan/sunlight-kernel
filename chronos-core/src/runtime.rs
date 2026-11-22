use alloc::{collections::VecDeque, vec::Vec};

use crate::{
    dos, load_com_with_command_tail, load_program, ArenaError, CpuState, DosHandleTable,
    DosMemoryArena, DosPath, DriveTable, ExecutableFormat, GuestMemory, GuestVideoMode,
    IoOperation, IoTrap, IoWidth, LoadedProgram, LoaderError, Rgb8, TextAttribute,
    TextAttributeMode, TextCell, TextModeSurface, VgaDacEntry, DEFAULT_ATTRIBUTE, TEXT_COLUMNS,
    TEXT_ROWS, VGA_FRAMEBUFFER_BYTES, VGA_HEIGHT, VIDEO_SEGMENT,
};

const BDA_SEGMENT: u16 = 0x0040;
const BDA_KEYBOARD_FLAGS: u16 = 0x0017;
const BDA_MODELED_KEYBOARD_FLAGS: u8 = 0x2f;
const BDA_VIDEO_MODE: u16 = 0x0049;
const BDA_COLUMNS: u16 = 0x004a;
const BDA_PAGE_SIZE: u16 = 0x004c;
const BDA_CURSOR_POSITIONS: u16 = 0x0050;
const BDA_CURSOR_SHAPE: u16 = 0x0060;
const BDA_ACTIVE_PAGE: u16 = 0x0062;
const BDA_VIDEO_STATE_OFFSET: u16 = 0x0049;
const BDA_VIDEO_STATE_BYTES: usize = 0x60;
const ENVIRONMENT_MAX_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trap {
    UnsupportedOpcode {
        cs: u16,
        ip: u16,
        bytes: [u8; 4],
        cpu: CpuState,
    },
    UnsupportedInterrupt {
        interrupt: u8,
        function: u8,
    },
    UnterminatedDosString {
        segment: u16,
        offset: u16,
        maximum: usize,
    },
    InvalidSegmentRegister {
        encoding: u8,
    },
    SegmentLimit {
        segment: u16,
        offset: u32,
        width: u8,
    },
    MalformedPrefix {
        cs: u16,
        ip: u16,
    },
    InvalidVideoRectangle,
    UnsupportedVideoMode {
        mode: u8,
    },
    DivideError {
        cs: u16,
        ip: u16,
    },
    CpuProfileViolation {
        cs: u16,
        ip: u16,
        opcode: u8,
    },
    UnsupportedIoPort {
        operation: IoOperation,
        port: u16,
        width: IoWidth,
        value: Option<u16>,
        cs: u16,
        ip: u16,
        active_executable: Vec<u8>,
        application_id: Vec<u8>,
    },
    ChildLoadFailed {
        error: LoaderError,
    },
}

impl Trap {
    pub const fn summary(&self) -> &'static str {
        match self {
            Self::UnsupportedOpcode { .. } => "Unsupported guest instruction",
            Self::UnsupportedInterrupt { .. } => "Unsupported DOS or BIOS interrupt",
            Self::UnterminatedDosString { .. } => "Unterminated DOS string",
            Self::InvalidSegmentRegister { .. } => "Invalid segment register",
            Self::SegmentLimit { .. } => "Guest operand exceeds real-mode segment limit",
            Self::MalformedPrefix { .. } => "Malformed instruction prefix",
            Self::InvalidVideoRectangle => "Invalid video rectangle",
            Self::UnsupportedVideoMode { .. } => "Unsupported BIOS video mode",
            Self::DivideError { .. } => "Guest divide error",
            Self::CpuProfileViolation { .. } => "Instruction unavailable on guest CPU",
            Self::UnsupportedIoPort { .. } => "Unsupported guest I/O port access",
            Self::ChildLoadFailed { .. } => "Child executable could not be loaded",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestState {
    Ready,
    Running,
    WaitingForInput,
    YieldedUntilTimer,
    Exited { code: u8 },
    Halted,
    Trapped(Trap),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestStateKind {
    Ready,
    Running,
    WaitingForInput,
    YieldedUntilTimer,
    Exited,
    Halted,
    Trapped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestWaitReason {
    None,
    KeyboardInput,
    CooperativeTimer,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceStopReason {
    NotRun,
    BudgetExhausted,
    WaitingForInput,
    YieldedUntilTimer,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestWakeSource {
    Startup,
    Timer,
    Keyboard,
    MouseMotion,
    MouseButton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BiosKey {
    pub ascii: u8,
    pub scan_code: u8,
}

impl BiosKey {
    pub(crate) fn bios_word(self, enhanced: bool) -> u16 {
        let ascii = if !enhanced || self.ascii == 0xf0 {
            self.console_byte()
        } else {
            self.ascii
        };
        let scan = match (enhanced, self.scan_code, ascii) {
            (false, 0xe0, 0x0d | 0x0a) => 0x1c,
            (false, 0xe0, b'/') => 0x35,
            _ => self.scan_code,
        };
        u16::from_le_bytes([ascii, scan])
    }

    // BIOS markers are not characters for DOS's byte-oriented console.
    pub(crate) fn console_byte(self) -> u8 {
        if self.scan_code != 0 && matches!(self.ascii, 0xe0 | 0xf0) {
            0
        } else {
            self.ascii
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostKeyEvent {
    pub keycode: u8,
    pub extended: bool,
    pub pressed: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Translate SunlightOS PS/2-set-1 key events into BIOS key pairs. Printable
/// text should be injected through `Event::Key`; this path is for controls and
/// extended keys, preventing duplicate printable events.
pub fn translate_key_press(event: HostKeyEvent) -> Option<BiosKey> {
    translate_key(event, false)
}

fn translate_key(event: HostKeyEvent, num_lock: bool) -> Option<BiosKey> {
    if !event.pressed {
        return None;
    }
    let scan_code = event.keycode;
    let modified = |scan_code, ascii| Some(BiosKey { ascii, scan_code });
    if scan_code == 0x1c {
        return if event.alt {
            modified(if event.extended { 0xa6 } else { 0x1c }, 0)
        } else {
            modified(
                if event.extended { 0xe0 } else { 0x1c },
                if event.ctrl { b'\n' } else { b'\r' },
            )
        };
    }
    // PS/2 set 1 keypad words and E0 navigation, as in SeaBIOS 1.16.2.
    if let Some((digit, ctrl_scan)) = match scan_code {
        0x47 => Some((b'7', 0x77)),
        0x48 => Some((b'8', 0x8d)),
        0x49 => Some((b'9', 0x84)),
        0x4b => Some((b'4', 0x73)),
        0x4c if !event.extended => Some((b'5', 0x8f)),
        0x4d => Some((b'6', 0x74)),
        0x4f => Some((b'1', 0x75)),
        0x50 => Some((b'2', 0x91)),
        0x51 => Some((b'3', 0x76)),
        0x52 => Some((b'0', 0x92)),
        0x53 => Some((b'.', 0x93)),
        _ => None,
    } {
        return if event.alt {
            if event.extended {
                modified(scan_code + 0x50, 0)
            } else {
                // No immediate BIOS word for Alt plus a keypad digit or decimal.
                None
            }
        } else if event.extended {
            modified(if event.ctrl { ctrl_scan } else { scan_code }, 0xe0)
        } else if event.ctrl {
            modified(ctrl_scan, 0)
        } else {
            modified(scan_code, if num_lock ^ event.shift { digit } else { 0 })
        };
    }
    if let Some((plain_scan, ascii, ctrl_scan, alt_scan, alt_ascii)) =
        match (event.extended, scan_code) {
            (false, 0x37) => Some((0x37, b'*', 0x96, 0x37, 0xf0)),
            (false, 0x4a) => Some((0x4a, b'-', 0x8e, 0x4a, 0xf0)),
            (false, 0x4e) => Some((0x4e, b'+', 0x90, 0x4e, 0xf0)),
            (true, 0x35) => Some((0xe0, b'/', 0x95, 0xa4, 0)),
            _ => None,
        }
    {
        return if event.alt {
            modified(alt_scan, alt_ascii)
        } else if event.ctrl {
            modified(ctrl_scan, 0)
        } else {
            modified(plain_scan, ascii)
        };
    }
    if !event.extended && (0x3b..=0x44).contains(&scan_code) {
        let base = if event.alt {
            0x68
        } else if event.ctrl {
            0x5e
        } else if event.shift {
            0x54
        } else {
            0x3b
        };
        return modified(base + scan_code - 0x3b, 0);
    }
    if !event.extended && event.alt && matches!(scan_code, 0x10..=0x19 | 0x1e..=0x26 | 0x2c..=0x32)
    {
        return modified(scan_code, 0);
    }
    let ascii = match scan_code {
        0x01 => 0x1b,
        0x0e => 0x08,
        0x0f => b'\t',
        0x39 => b' ',
        _ => return None,
    };
    Some(BiosKey { ascii, scan_code })
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum InputMode {
    Bios {
        enhanced: bool,
    },
    Dos {
        echo: bool,
    },
    ConsoleRead {
        segment: u16,
        offset: u16,
    },
    Line {
        segment: u16,
        offset: u16,
        maximum: u8,
        length: u8,
    },
}

#[derive(Clone, Copy, Debug)]
struct ModRm {
    register: u8,
    rm: u8,
    memory: Option<(u16, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuProfile {
    I8086,
    I80186,
    I80286,
    /// Partial integer-only 386 real-mode implementation; unsupported forms trap.
    I80386RealMode,
}

impl CpuProfile {
    pub const fn supports_80186(self) -> bool {
        !matches!(self, Self::I8086)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminationType {
    Normal = 0,
    CtrlBreak = 1,
    RuntimeTrap = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChildResult {
    pub code: u8,
    pub termination: TerminationType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub hundredths: u8,
}

#[derive(Clone, Debug)]
pub struct DosProcess {
    pub(crate) searches: Vec<crate::DirectoryEntry>,
    pub(crate) search_index: usize,
    vectors_on_entry: [(u16, u16); 256],
    pub psp_segment: u16,
    pub parent_psp: Option<u16>,
    pub environment_segment: u16,
    pub cpu: CpuState,
    pub dta_segment: u16,
    pub dta_offset: u16,
    pub handles: DosHandleTable,
    pub current_drive: crate::DosDrive,
    pub format: ExecutableFormat,
    pub allocation_segment: u16,
    pub allocation_paragraphs: u16,
    pub environment_owned: bool,
    pub child_result: Option<ChildResult>,
    pub executable_path: Vec<u8>,
}

/// Complete safe real-mode guest. Text output is always backed by guest
/// `0xB8000` memory; the surface field remains a zero-sized compatibility view.
pub struct Runtime {
    pub cpu: CpuState,
    pub memory: GuestMemory,
    pub text: TextModeSurface,
    state: GuestState,
    keyboard: VecDeque<BiosKey>,
    keyboard_trace: Option<fn(u8, u16, u16, usize)>,
    keyboard_trace_remaining: u16,
    pending_input: Option<InputMode>,
    dos_scan_tail: Option<u8>,
    pub(crate) cursor_column: usize,
    pub(crate) cursor_row: usize,
    pub(crate) cursor_shape: u16,
    video_mode: GuestVideoMode,
    video_mode_generation: u64,
    text_attribute_mode: TextAttributeMode,
    text_attribute_generation: u64,
    io: crate::io::GuestIoDispatcher,
    pub(crate) mouse: crate::DosMouse,
    cooperative_yield_requested: bool,
    cooperative_yielded_last_slice: bool,
    cooperative_yield_count: u64,
    instructions_retired: u64,
    last_slice_stop_reason: SliceStopReason,
    last_wake_source: Option<GuestWakeSource>,
    wake_generation: u64,
    num_lock: bool,
    num_lock_down: bool,
    modifier_sides: u8,
    ctrl: bool,
    alt: bool,
    pub(crate) drives: DriveTable,
    pub(crate) handles: DosHandleTable,
    pub(crate) dta_segment: u16,
    pub(crate) dta_offset: u16,
    pub(crate) arena: DosMemoryArena,
    pub(crate) active_process: DosProcess,
    pub(crate) parent_process: Option<DosProcess>,
    recovered_child_trap: Option<Trap>,
    pub(crate) last_delivered_child_result: Option<ChildResult>,
    pub(crate) cpu_profile: CpuProfile,
    guest_unix_time: u64,
    environment_app_id: Vec<u8>,
    executable_path: Vec<u8>,
    pub(crate) break_check_enabled: bool,
}

impl Runtime {
    pub fn from_com(image: &[u8]) -> Result<Self, LoaderError> {
        Self::from_com_with_command_tail(image, &[])
    }

    pub fn from_com_with_command_tail(
        image: &[u8],
        command_tail: &[u8],
    ) -> Result<Self, LoaderError> {
        let mut memory = GuestMemory::new();
        let cpu = load_com_with_command_tail(&mut memory, image, command_tail)?;
        let active_process = DosProcess {
            searches: Vec::new(),
            search_index: 0,
            vectors_on_entry: [(0, 0); 256],
            psp_segment: crate::PSP_SEGMENT,
            parent_psp: None,
            environment_segment: 0,
            cpu,
            dta_segment: crate::PSP_SEGMENT,
            dta_offset: 0x0080,
            handles: DosHandleTable::new(),
            current_drive: crate::DosDrive::C,
            format: ExecutableFormat::Com,
            allocation_segment: crate::PSP_SEGMENT,
            allocation_paragraphs: 0x1000,
            environment_owned: false,
            child_result: None,
            executable_path: b"C:\\PROGRAM.COM".to_vec(),
        };
        let mut runtime = Self {
            cpu,
            memory,
            text: TextModeSurface::new(),
            state: GuestState::Ready,
            keyboard: VecDeque::new(),
            keyboard_trace: None,
            keyboard_trace_remaining: 0,
            pending_input: None,
            dos_scan_tail: None,
            cursor_column: 0,
            cursor_row: 0,
            cursor_shape: 0x0607,
            video_mode: GuestVideoMode::Text80x25Color,
            video_mode_generation: 0,
            text_attribute_mode: TextAttributeMode::Blink,
            text_attribute_generation: 0,
            io: crate::io::GuestIoDispatcher::default(),
            mouse: crate::DosMouse::default(),
            cooperative_yield_requested: false,
            cooperative_yielded_last_slice: false,
            cooperative_yield_count: 0,
            instructions_retired: 0,
            last_slice_stop_reason: SliceStopReason::NotRun,
            last_wake_source: None,
            wake_generation: 0,
            num_lock: false,
            num_lock_down: false,
            modifier_sides: 0,
            ctrl: false,
            alt: false,
            drives: DriveTable::new(),
            handles: DosHandleTable::new(),
            dta_segment: crate::PSP_SEGMENT,
            dta_offset: 0x0080,
            arena: DosMemoryArena::new(),
            active_process,
            parent_process: None,
            recovered_child_trap: None,
            last_delivered_child_result: None,
            cpu_profile: CpuProfile::I8086,
            guest_unix_time: 0,
            environment_app_id: b"org.sunlight.chronos".to_vec(),
            executable_path: b"C:\\PROGRAM.COM".to_vec(),
            break_check_enabled: false,
        };
        let _ = runtime.arena.allocate(0x1000, crate::PSP_SEGMENT);
        runtime.initialize_environment();
        runtime.initialize_interrupts();
        runtime.active_process.vectors_on_entry = runtime.vector_snapshot();
        runtime.memory.write_u16(0x0040, 0x0010, 0x0020);
        runtime.reset_video();
        Ok(runtime)
    }

    pub fn from_program(image: &[u8], command_tail: &[u8]) -> Result<Self, LoaderError> {
        let mut runtime = Self::from_com_with_command_tail(&[], &[])?;
        runtime.memory.clear();
        runtime.initialize_interrupts();
        runtime.memory.write_u16(0x0040, 0x0010, 0x0020);
        runtime.arena = DosMemoryArena::new();
        runtime.drives = DriveTable::new();
        runtime.handles = DosHandleTable::new();
        runtime.keyboard.clear();
        runtime.pending_input = None;
        runtime.parent_process = None;
        runtime.recovered_child_trap = None;
        runtime.last_delivered_child_result = None;
        runtime.mouse.reset();
        runtime.cooperative_yield_requested = false;
        runtime.cooperative_yielded_last_slice = false;
        runtime.cooperative_yield_count = 0;
        runtime.instructions_retired = 0;
        runtime.last_slice_stop_reason = SliceStopReason::NotRun;
        runtime.last_wake_source = None;
        runtime.wake_generation = 0;
        runtime.executable_path = b"C:\\PROGRAM.EXE".to_vec();
        let environment_entries =
            default_environment(&runtime.environment_app_id, &runtime.executable_path);
        let environment_segment = runtime
            .reserve_initial_environment(&environment_entries, &runtime.executable_path.clone())?;
        let program = load_program(
            &mut runtime.memory,
            &mut runtime.arena,
            image,
            command_tail,
            None,
            environment_segment,
        )?;
        runtime
            .arena
            .reassign_owner(environment_segment, program.psp_segment)
            .map_err(|_| LoaderError::InsufficientMemory {
                requested: 0,
                largest: runtime.arena.largest_available(),
            })?;
        runtime.arena.sync_mcbs(&mut runtime.memory);
        runtime.install_initial_program(program);
        runtime.active_process.environment_segment = environment_segment;
        runtime.active_process.environment_owned = true;
        runtime.reset_video();
        Ok(runtime)
    }

    fn install_initial_program(&mut self, program: LoadedProgram) {
        self.cpu = program.cpu;
        self.active_process = DosProcess {
            searches: Vec::new(),
            search_index: 0,
            vectors_on_entry: self.vector_snapshot(),
            psp_segment: program.psp_segment,
            parent_psp: None,
            environment_segment: 0,
            cpu: program.cpu,
            dta_segment: program.psp_segment,
            dta_offset: 0x0080,
            handles: DosHandleTable::new(),
            current_drive: self.drives.current_drive,
            format: program.format,
            allocation_segment: program.psp_segment,
            allocation_paragraphs: program.paragraphs,
            environment_owned: false,
            child_result: None,
            executable_path: self.executable_path.clone(),
        };
        self.dta_segment = program.psp_segment;
        self.dta_offset = 0x0080;
    }

    pub fn set_cpu_profile(&mut self, profile: CpuProfile) {
        self.cpu_profile = profile;
    }

    /// Sets the guest-visible UTC clock from the Chronos host boundary.
    /// DOS guests can read this clock but cannot modify it.
    pub fn set_guest_unix_time(&mut self, unix_time: u64) {
        self.guest_unix_time = unix_time;
    }

    pub fn guest_date(&self) -> GuestDate {
        let days = (self.guest_unix_time / 86_400).min(i64::MAX as u64) as i64;
        let (year, month, day) = civil_from_days(days);
        GuestDate {
            year: year.clamp(0, u16::MAX as i32) as u16,
            month,
            day,
            weekday: ((days + 4).rem_euclid(7)) as u8,
        }
    }

    pub fn guest_time(&self) -> GuestTime {
        let seconds = self.guest_unix_time % 86_400;
        GuestTime {
            hour: (seconds / 3_600) as u8,
            minute: ((seconds / 60) % 60) as u8,
            second: (seconds % 60) as u8,
            hundredths: 0,
        }
    }

    fn vector_snapshot(&self) -> [(u16, u16); 256] {
        core::array::from_fn(|n| self.interrupt_vector(n as u8))
    }

    fn release_process_vectors(&mut self, process: &DosProcess) {
        // Restore only vectors whose backing allocation is about to be freed.
        // A suspended parent's handler remains valid and must survive child exit.
        for n in 0..256 {
            let vector = self.interrupt_vector(n as u8);
            let address = GuestMemory::physical_address(vector.0, vector.1);
            if self.arena.blocks().iter().any(|block| {
                let start = usize::from(block.segment) * 16;
                block.owner_psp == Some(process.psp_segment)
                    && (start..start + usize::from(block.paragraphs) * 16).contains(&address)
            }) {
                let (segment, offset) = process.vectors_on_entry[n];
                self.set_interrupt_vector(n as u8, segment, offset);
            }
        }
    }

    fn initialize_interrupts(&mut self) {
        self.memory.install_firmware();
        self.set_interrupt_vector(
            0x15,
            crate::memory::INT15_SEGMENT,
            crate::memory::INT15_OFFSET,
        );
        self.set_interrupt_vector(0x1f, crate::font::ROM_SEGMENT, 0x400);
        self.set_interrupt_vector(0x43, crate::font::ROM_SEGMENT, 0x800);
        // DOS multiplex has an end-of-chain handler even with no TSR installed.
        // Ordinary guest IRET supports both tail jumps and PUSHF/CALL FAR chains.
        // DOS-owned low memory, separate from SFT and country case mapping.
        self.memory.write_u8(0x0800, 0x0400, 0xcf);
        self.set_interrupt_vector(0x2f, 0x0800, 0x0400);
    }

    pub(crate) fn interrupt_vector(&self, interrupt: u8) -> (u16, u16) {
        let address = u16::from(interrupt) * 4;
        (
            self.memory.read_u16(0, address + 2),
            self.memory.read_u16(0, address),
        )
    }

    pub(crate) fn enter_guest_interrupt(&mut self, segment: u16, offset: u16) {
        self.push_u16(self.cpu.flags | 2);
        self.push_u16(self.cpu.cs);
        self.push_u16(self.cpu.ip);
        self.cpu.flags &= !(CpuState::FLAG_IF | CpuState::FLAG_TF);
        self.cpu.cs = segment;
        self.cpu.ip = offset;
    }

    pub(crate) fn set_interrupt_vector(&mut self, interrupt: u8, segment: u16, offset: u16) {
        self.memory.write_u16(0, u16::from(interrupt) * 4, offset);
        self.memory
            .write_u16(0, u16::from(interrupt) * 4 + 2, segment);
    }

    pub fn set_application_id(&mut self, app_id: &[u8]) {
        self.environment_app_id = app_id[..app_id.len().min(80)].to_vec();
        self.reset_environment();
    }

    /// Sets the DOS-visible entry path used by COMSPEC and SHELL. This path is
    /// guest-owned metadata such as `C:\SUNSH.EXE`, never a host filesystem
    /// path.
    pub fn set_executable_path(&mut self, executable_path: &[u8]) {
        if executable_path.is_empty() || executable_path.len() > 240 {
            return;
        }
        self.executable_path = executable_path.to_vec();
        self.active_process.executable_path = self.executable_path.clone();
        self.reset_environment();
    }

    fn reset_environment(&mut self) {
        if self.active_process.environment_owned && self.active_process.environment_segment != 0 {
            let _ = self.arena.free(
                self.active_process.environment_segment,
                self.active_process.psp_segment,
            );
        }
        let entries = default_environment(&self.environment_app_id, &self.executable_path);
        let executable_path = self.executable_path.clone();
        if let Ok(segment) =
            self.write_environment(self.active_process.psp_segment, &entries, &executable_path)
        {
            self.active_process.environment_segment = segment;
            self.active_process.environment_owned = true;
            self.memory
                .write_u16(self.active_process.psp_segment, 0x002c, segment);
        }
    }

    pub fn current_psp(&self) -> u16 {
        self.active_process.psp_segment
    }

    pub(crate) fn allocate_memory(&mut self, paragraphs: u16) -> Result<u16, ArenaError> {
        let result = self
            .arena
            .allocate(paragraphs, self.active_process.psp_segment);
        self.arena.sync_mcbs(&mut self.memory);
        result
    }

    fn release_vectors_in_range(&mut self, start: usize, end: usize) {
        for n in 0..256 {
            let (segment, offset) = self.interrupt_vector(n as u8);
            if (start..end).contains(&GuestMemory::physical_address(segment, offset)) {
                let previous = self.active_process.vectors_on_entry[n];
                // The entry snapshot may itself refer to this block (e.g. a
                // parent block explicitly freed later). Never restore freed code.
                let address = GuestMemory::physical_address(previous.0, previous.1);
                let (segment, offset) = if (start..end).contains(&address) {
                    if n == 0x2f {
                        (0x0800, 0x0400)
                    } else {
                        (0, 0)
                    }
                } else {
                    previous
                };
                self.set_interrupt_vector(n as u8, segment, offset);
            }
        }
    }

    pub(crate) fn free_memory(&mut self, segment: u16) -> Result<(), ArenaError> {
        let paragraphs = self
            .arena
            .blocks()
            .iter()
            .find(|block| block.segment == segment)
            .map(|block| block.paragraphs);
        let result = self.arena.free(segment, self.active_process.psp_segment);
        if result.is_ok() {
            let start = usize::from(segment) * 16;
            self.release_vectors_in_range(start, start + usize::from(paragraphs.unwrap()) * 16);
        }
        self.arena.sync_mcbs(&mut self.memory);
        result
    }

    pub(crate) fn resize_memory(
        &mut self,
        segment: u16,
        paragraphs: u16,
    ) -> Result<(), ArenaError> {
        let old_paragraphs = self
            .arena
            .blocks()
            .iter()
            .find(|block| block.segment == segment)
            .map(|block| block.paragraphs);
        let result = self
            .arena
            .resize(segment, paragraphs, self.active_process.psp_segment);
        if result.is_ok() && old_paragraphs.is_some_and(|old| paragraphs < old) {
            let start = usize::from(segment) * 16;
            self.release_vectors_in_range(
                start + usize::from(paragraphs) * 16,
                start + usize::from(old_paragraphs.unwrap()) * 16,
            );
        }
        if result.is_ok() && segment == self.active_process.psp_segment {
            self.active_process.allocation_paragraphs = paragraphs;
            self.memory.write_u16(segment, 2, segment + paragraphs);
        }
        self.arena.sync_mcbs(&mut self.memory);
        result
    }

    pub(crate) fn largest_available_memory(&self) -> u16 {
        self.arena.largest_available()
    }

    pub(crate) fn set_current_psp(&mut self, psp: u16) -> bool {
        psp == self.active_process.psp_segment
    }

    pub(crate) fn exec(
        &mut self,
        path: DosPath,
        command_tail: &[u8],
        requested_environment: u16,
    ) -> Result<(), LoaderError> {
        if self.parent_process.is_some() {
            return Err(LoaderError::UnsupportedExecutableFormat);
        }
        let image = self
            .drives
            .read_file(&path)
            .map_err(|_| LoaderError::UnsupportedExecutableFormat)?
            .to_vec();
        let parent_psp = self.active_process.psp_segment;
        let inherited_environment = self.environment_bytes(requested_environment)?;
        // Reserve the environment before the executable can request the largest block.
        let saved_arena = self.arena.clone();
        let environment_segment = self.write_environment(
            parent_psp,
            &inherited_environment,
            path.display().as_bytes(),
        )?;
        let program = match load_program(
            &mut self.memory,
            &mut self.arena,
            &image,
            command_tail,
            Some(parent_psp),
            environment_segment,
        ) {
            Ok(program) => program,
            Err(error) => {
                self.arena = saved_arena;
                self.arena.sync_mcbs(&mut self.memory);
                return Err(error);
            }
        };
        self.arena
            .reassign_owner(environment_segment, program.psp_segment)
            .expect("reserved environment exists");
        self.arena.sync_mcbs(&mut self.memory);
        self.memory
            .write_u16(program.psp_segment, 0x002c, environment_segment);
        self.active_process.cpu = self.cpu;
        self.active_process.cpu.flags &= !CpuState::FLAG_CF;
        self.active_process.dta_segment = self.dta_segment;
        self.active_process.dta_offset = self.dta_offset;
        self.active_process.handles = self.handles.clone();
        self.active_process.current_drive = self.drives.current_drive;
        let mut child = DosProcess {
            searches: Vec::new(),
            search_index: 0,
            vectors_on_entry: self.vector_snapshot(),
            psp_segment: program.psp_segment,
            parent_psp: Some(parent_psp),
            environment_segment,
            cpu: program.cpu,
            dta_segment: program.psp_segment,
            dta_offset: 0x0080,
            handles: self.handles.inherit_for_child(),
            current_drive: self.drives.current_drive,
            format: program.format,
            allocation_segment: program.psp_segment,
            allocation_paragraphs: program.paragraphs,
            environment_owned: true,
            child_result: None,
            executable_path: path.display().as_bytes().to_vec(),
        };
        core::mem::swap(&mut child, &mut self.active_process);
        self.parent_process = Some(child);
        self.cpu = self.active_process.cpu;
        self.dta_segment = self.active_process.dta_segment;
        self.dta_offset = self.active_process.dta_offset;
        self.handles = self.active_process.handles.clone();
        self.drives.current_drive = self.active_process.current_drive;
        self.pending_input = None;
        self.dos_scan_tail = None;
        Ok(())
    }

    fn initialize_environment(&mut self) {
        let contents = default_environment(&self.environment_app_id, &self.executable_path);
        let executable_path = self.executable_path.clone();
        if let Ok(segment) =
            self.write_environment(self.active_process.psp_segment, &contents, &executable_path)
        {
            self.active_process.environment_segment = segment;
            self.active_process.environment_owned = true;
            self.memory
                .write_u16(self.active_process.psp_segment, 0x002c, segment);
        }
    }

    fn environment_bytes(&self, requested: u16) -> Result<Vec<u8>, LoaderError> {
        let segment = if requested == 0 {
            self.active_process.environment_segment
        } else {
            requested
        };
        if segment == 0 {
            return Ok(default_environment(
                &self.environment_app_id,
                &self.executable_path,
            ));
        }
        let owner = if requested == 0 {
            self.active_process.psp_segment
        } else {
            self.active_process.psp_segment
        };
        if !self.arena.owns_range(owner, segment, 0, 1) {
            return Err(LoaderError::UnsupportedExecutableFormat);
        }
        let mut bytes = Vec::new();
        for offset in 0..ENVIRONMENT_MAX_BYTES {
            let byte = self.memory.read_u8(segment, offset as u16);
            bytes.push(byte);
            if bytes.len() >= 2 && bytes[bytes.len() - 2..] == [0, 0] {
                return Ok(bytes);
            }
        }
        Err(LoaderError::UnsupportedExecutableFormat)
    }

    fn write_environment(
        &mut self,
        owner_psp: u16,
        entries: &[u8],
        executable_path: &[u8],
    ) -> Result<u16, LoaderError> {
        let contents = environment_contents(entries, executable_path)?;
        let paragraphs = environment_paragraphs()?;
        let segment = self
            .arena
            .allocate(paragraphs.max(1), owner_psp)
            .map_err(|_| LoaderError::InsufficientMemory {
                requested: paragraphs,
                largest: self.arena.largest_available(),
            })?;
        self.memory.write_slice(segment, 0, &contents)?;
        self.arena.sync_mcbs(&mut self.memory);
        Ok(segment)
    }

    fn reserve_initial_environment(
        &mut self,
        entries: &[u8],
        executable_path: &[u8],
    ) -> Result<u16, LoaderError> {
        let contents = environment_contents(entries, executable_path)?;
        let paragraphs = environment_paragraphs()?;
        let segment = self.arena.allocate(paragraphs.max(1), 0).map_err(|_| {
            LoaderError::InsufficientMemory {
                requested: paragraphs,
                largest: self.arena.largest_available(),
            }
        })?;
        self.memory.write_slice(segment, 0, &contents)?;
        self.arena.sync_mcbs(&mut self.memory);
        Ok(segment)
    }

    fn restore_parent(&mut self, result: ChildResult) {
        // An interactive text parent must never inherit an unusable graphics
        // surface from a child that exited or trapped before restoring mode 03h.
        // This policy is based solely on the parent/child execution context.
        if self.video_mode != GuestVideoMode::Text80x25Color {
            self.set_video_mode(GuestVideoMode::Text80x25Color);
        }
        let mut child = core::mem::replace(
            &mut self.active_process,
            self.parent_process
                .take()
                .expect("restore_parent requires suspended parent"),
        );
        child.handles.close_nonstandard();
        self.release_process_vectors(&child);
        self.arena.free_owner(child.psp_segment);
        self.arena.sync_mcbs(&mut self.memory);
        self.active_process.child_result = Some(result);
        self.cpu = self.active_process.cpu;
        self.dta_segment = self.active_process.dta_segment;
        self.dta_offset = self.active_process.dta_offset;
        self.handles = self.active_process.handles.clone();
        self.drives.current_drive = self.active_process.current_drive;
        self.pending_input = None;
        self.dos_scan_tail = None;
        self.state = GuestState::Running;
    }

    pub fn drives(&self) -> &DriveTable {
        &self.drives
    }

    pub fn drives_mut(&mut self) -> &mut DriveTable {
        &mut self.drives
    }

    pub fn state(&self) -> &GuestState {
        &self.state
    }

    pub const fn state_kind(&self) -> GuestStateKind {
        match self.state {
            GuestState::Ready => GuestStateKind::Ready,
            GuestState::Running => GuestStateKind::Running,
            GuestState::WaitingForInput => GuestStateKind::WaitingForInput,
            GuestState::YieldedUntilTimer => GuestStateKind::YieldedUntilTimer,
            GuestState::Exited { .. } => GuestStateKind::Exited,
            GuestState::Halted => GuestStateKind::Halted,
            GuestState::Trapped(_) => GuestStateKind::Trapped,
        }
    }

    pub const fn wait_reason(&self) -> GuestWaitReason {
        match self.state {
            GuestState::WaitingForInput => GuestWaitReason::KeyboardInput,
            GuestState::YieldedUntilTimer => GuestWaitReason::CooperativeTimer,
            GuestState::Ready | GuestState::Running => GuestWaitReason::None,
            GuestState::Exited { .. } | GuestState::Halted | GuestState::Trapped(_) => {
                GuestWaitReason::Stopped
            }
        }
    }

    pub const fn instructions_retired(&self) -> u64 {
        self.instructions_retired
    }

    pub const fn last_slice_stop_reason(&self) -> SliceStopReason {
        self.last_slice_stop_reason
    }

    pub const fn last_wake_source(&self) -> Option<GuestWakeSource> {
        self.last_wake_source
    }

    pub const fn wake_generation(&self) -> u64 {
        self.wake_generation
    }

    pub const fn mouse(&self) -> &crate::DosMouse {
        &self.mouse
    }

    pub fn set_mouse_enabled(&mut self, enabled: bool) {
        self.mouse = crate::DosMouse::new(enabled);
    }

    pub fn mouse_focus_changed(&mut self, focused: bool) {
        self.mouse.focus_changed(focused);
    }

    pub fn mouse_pointer_left(&mut self) {
        self.mouse.pointer_left();
    }

    pub fn mouse_pointer_delivery_lost(&mut self) {
        self.mouse.pointer_delivery_lost();
    }

    pub fn inject_mouse_motion(&mut self, viewport: crate::MouseViewport, x: i32, y: i32) -> bool {
        if self.video_mode == GuestVideoMode::Text80x25Color && !self.mouse.cursor_visible() {
            return false;
        }
        let changed = self.mouse.native_motion(viewport, x, y);
        if changed {
            self.wake_yielded(GuestWakeSource::MouseMotion);
        }
        changed
    }

    pub fn inject_mouse_button(
        &mut self,
        viewport: crate::MouseViewport,
        x: i32,
        y: i32,
        button: u8,
        pressed: bool,
    ) -> bool {
        if self.video_mode == GuestVideoMode::Text80x25Color && !self.mouse.cursor_visible() {
            return false;
        }
        let changed = self.mouse.native_button(viewport, x, y, button, pressed);
        if changed {
            self.wake_yielded(GuestWakeSource::MouseButton);
        }
        changed
    }

    pub const fn cooperative_yielded_last_slice(&self) -> bool {
        self.cooperative_yielded_last_slice
    }

    pub const fn cooperative_yield_count(&self) -> u64 {
        self.cooperative_yield_count
    }

    pub(crate) fn cooperative_yield(&mut self) {
        self.cooperative_yield_requested = true;
        self.cooperative_yield_count = self.cooperative_yield_count.wrapping_add(1);
        self.state = GuestState::YieldedUntilTimer;
    }

    pub fn wake_from_timer(&mut self) -> bool {
        self.wake_yielded(GuestWakeSource::Timer)
    }

    fn wake_yielded(&mut self, source: GuestWakeSource) -> bool {
        if !matches!(self.state, GuestState::YieldedUntilTimer) {
            return false;
        }
        self.state = GuestState::Running;
        self.cooperative_yielded_last_slice = false;
        self.record_wake(source);
        true
    }

    fn record_wake(&mut self, source: GuestWakeSource) {
        self.last_wake_source = Some(source);
        self.wake_generation = self.wake_generation.wrapping_add(1);
    }

    pub fn cell(&self, column: usize, row: usize) -> TextCell {
        TextModeSurface::cell(&self.memory, column, row)
    }

    pub fn cursor_column(&self) -> usize {
        self.cursor_column
    }
    pub fn cursor_row(&self) -> usize {
        self.cursor_row
    }
    pub fn cursor_shape(&self) -> u16 {
        self.cursor_shape
    }

    pub const fn video_mode(&self) -> GuestVideoMode {
        self.video_mode
    }

    pub const fn video_mode_generation(&self) -> u64 {
        self.video_mode_generation
    }

    pub const fn text_attribute_mode(&self) -> TextAttributeMode {
        self.text_attribute_mode
    }

    /// Advances whenever text attribute bit-7 semantics change, without
    /// rewriting guest `0xB8000` bytes.
    pub const fn text_attribute_generation(&self) -> u64 {
        self.text_attribute_generation
    }

    pub fn text_attribute(&self, attribute: u8) -> TextAttribute {
        TextAttribute::decode(attribute, self.text_attribute_mode)
    }

    /// True when the visible text page contains at least one attribute whose
    /// bit 7 is a blink marker under the current mode.
    pub fn has_blink_attributes(&self) -> bool {
        if self.video_mode != GuestVideoMode::Text80x25Color
            || self.text_attribute_mode != TextAttributeMode::Blink
        {
            return false;
        }
        (0..TEXT_COLUMNS * TEXT_ROWS).any(|index| {
            self.memory
                .read_display_byte(VIDEO_SEGMENT, (index * 2 + 1) as u16)
                & 0x80
                != 0
        })
    }

    pub const fn framebuffer_generation(&self) -> u64 {
        self.memory.framebuffer_generation()
    }

    pub const fn palette(&self) -> &[Rgb8; 256] {
        self.io.vga_dac().palette()
    }

    pub const fn palette_entries(&self) -> &[VgaDacEntry; 256] {
        self.io.vga_dac().entries()
    }

    pub const fn palette_generation(&self) -> u64 {
        self.io.vga_dac().palette_generation()
    }

    pub fn palette_checksum(&self) -> u64 {
        self.io.vga_dac().palette_checksum()
    }

    pub const fn dac_entries_committed_last_slice(&self) -> u32 {
        self.io.entries_committed_slice()
    }

    pub const fn unsupported_io_attempts(&self) -> u64 {
        self.io.unsupported_attempts()
    }

    pub fn framebuffer_index(&self, x: usize, y: usize) -> Option<u8> {
        if x >= crate::VGA_WIDTH || y >= VGA_HEIGHT {
            return None;
        }
        Some(self.memory.read_display_byte(
            crate::VGA_FRAMEBUFFER_SEGMENT,
            (y * crate::VGA_WIDTH + x) as u16,
        ))
    }

    /// Stable FNV-1a signature of the 64,000 authoritative visible Mode 13h
    /// bytes. This is intended for deterministic guest regression tests.
    pub fn framebuffer_checksum(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for offset in 0..VGA_FRAMEBUFFER_BYTES {
            hash ^= self
                .memory
                .read_display_byte(crate::VGA_FRAMEBUFFER_SEGMENT, offset as u16)
                as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    pub fn take_graphics_dirty_rows(&mut self) -> [bool; VGA_HEIGHT] {
        self.memory.take_graphics_dirty()
    }

    pub fn convert_graphics_rows(
        &self,
        dirty_rows: &[bool; VGA_HEIGHT],
        destination: &mut [Rgb8],
    ) -> bool {
        crate::convert_indexed_rows(&self.memory, self.palette(), dirty_rows, destination)
    }

    pub fn take_recovered_child_trap(&mut self) -> Option<Trap> {
        self.recovered_child_trap.take()
    }

    pub const fn last_delivered_child_result(&self) -> Option<ChildResult> {
        self.last_delivered_child_result
    }

    pub const fn text_cursor_visible(&self) -> bool {
        matches!(self.video_mode, GuestVideoMode::Text80x25Color) && self.cursor_shape & 0x2000 == 0
    }

    /// Observe successful BIOS key status and read returns for a bounded replay.
    /// Empty polling is deliberately omitted.
    pub fn trace_keyboard(&mut self, max_returns: u16, observer: fn(u8, u16, u16, usize)) {
        self.keyboard_trace = Some(observer);
        self.keyboard_trace_remaining = max_returns;
    }

    pub(crate) fn trace_keyboard_return(&mut self, function: u8) {
        if self.keyboard_trace_remaining != 0 {
            if let Some(observer) = self.keyboard_trace {
                self.keyboard_trace_remaining -= 1;
                observer(function, self.cpu.ax, self.cpu.flags, self.keyboard.len());
            }
        }
    }

    pub fn inject_key(&mut self, key: BiosKey) -> bool {
        self.keyboard.push_back(key);
        let woke = self.wake_yielded(GuestWakeSource::Keyboard);
        if matches!(self.state, GuestState::WaitingForInput) {
            self.complete_pending_input();
            if matches!(self.state, GuestState::Running) {
                self.record_wake(GuestWakeSource::Keyboard);
            }
            return true;
        }
        woke
    }

    /// Number of untranslated BIOS key events pending for the guest.
    ///
    /// This is diagnostic state only; BIOS keyboard calls remain the sole
    /// mechanism by which a guest consumes events.
    pub fn queued_key_count(&self) -> usize {
        self.keyboard.len()
    }

    pub fn inject_ascii(&mut self, ascii: u8) -> bool {
        self.inject_key(BiosKey {
            ascii,
            scan_code: 0,
        })
    }

    pub fn update_modifiers(&mut self, shift: bool, ctrl: bool, alt: bool) {
        // Snapshots can clear stale edges but cannot identify an unknown side.
        self.modifier_sides &= (if shift { 0x03 } else { 0 })
            | (if ctrl { 0x14 } else { 0 })
            | (if alt { 0x28 } else { 0 });
        self.ctrl = ctrl;
        self.alt = alt;
        // Publish the same modeled bits that INT 16h reports. Leave flags
        // outside the current keyboard model in guest memory untouched.
        let old = self.memory.read_u8(BDA_SEGMENT, BDA_KEYBOARD_FLAGS);
        self.memory.write_u8(
            BDA_SEGMENT,
            BDA_KEYBOARD_FLAGS,
            (old & !BDA_MODELED_KEYBOARD_FLAGS) | self.shift_flags(),
        );
    }

    pub fn release_host_keys(&mut self) {
        self.update_modifiers(false, false, false);
        self.num_lock_down = false;
    }

    /// The guest owns its lock state; transient modifiers come from each host snapshot.
    pub fn translate_host_key(&mut self, event: HostKeyEvent) -> Option<BiosKey> {
        // Low two bits use the BIOS Shift ordering; the next four use the
        // enhanced BIOS Ctrl/Alt ordering shifted left by two.
        let side = match (event.extended, event.keycode) {
            (false, 0x36) => 0x01,
            (false, 0x2a) => 0x02,
            (false, 0x1d) => 0x04,
            (false, 0x38) => 0x08,
            (true, 0x1d) => 0x10,
            (true, 0x38) => 0x20,
            _ => 0,
        };
        if event.pressed {
            self.modifier_sides |= side;
        } else {
            self.modifier_sides &= !side;
        }
        let num_lock_key = !event.extended && event.keycode == 0x45;
        if num_lock_key {
            if event.pressed && !self.num_lock_down {
                self.num_lock = !self.num_lock;
            }
            self.num_lock_down = event.pressed;
        }
        self.update_modifiers(event.shift, event.ctrl, event.alt);
        if num_lock_key {
            return None;
        }
        translate_key(event, self.num_lock)
    }

    pub fn run_slice(&mut self, budget: usize) -> bool {
        if matches!(self.state, GuestState::Ready) {
            self.state = GuestState::Running;
            self.record_wake(GuestWakeSource::Startup);
        }
        if !matches!(self.state, GuestState::Running) {
            self.last_slice_stop_reason = match self.state {
                GuestState::WaitingForInput => SliceStopReason::WaitingForInput,
                GuestState::YieldedUntilTimer => SliceStopReason::YieldedUntilTimer,
                _ => SliceStopReason::Stopped,
            };
            return false;
        }
        let state_before = self.state.clone();
        let mode_generation_before = self.video_mode_generation;
        let text_attribute_generation_before = self.text_attribute_generation;
        let framebuffer_generation_before = self.memory.framebuffer_generation();
        let palette_generation_before = self.palette_generation();
        let mouse_generation_before = self.mouse.generation();
        let overlay_generation_before = self.mouse.overlay_generation();
        self.cooperative_yield_requested = false;
        self.cooperative_yielded_last_slice = false;
        self.io.begin_slice();
        for _ in 0..budget {
            if !matches!(self.state, GuestState::Running) {
                break;
            }
            self.step();
            if self.cooperative_yield_requested {
                self.cooperative_yielded_last_slice = true;
                break;
            }
        }
        self.last_slice_stop_reason = match self.state {
            GuestState::Running => SliceStopReason::BudgetExhausted,
            GuestState::WaitingForInput => SliceStopReason::WaitingForInput,
            GuestState::YieldedUntilTimer => SliceStopReason::YieldedUntilTimer,
            _ => SliceStopReason::Stopped,
        };
        self.memory.take_video_dirty().iter().any(|dirty| *dirty)
            || self.memory.framebuffer_generation() != framebuffer_generation_before
            || self.palette_generation() != palette_generation_before
            || self.video_mode_generation != mode_generation_before
            || self.text_attribute_generation != text_attribute_generation_before
            || self.mouse.generation() != mouse_generation_before
            || self.mouse.overlay_generation() != overlay_generation_before
            || self.state != state_before
    }

    pub fn step(&mut self) {
        if matches!(self.state, GuestState::Ready) {
            self.state = GuestState::Running;
            self.record_wake(GuestWakeSource::Startup);
        }
        if !matches!(self.state, GuestState::Running) {
            return;
        }
        let instruction_cs = self.cpu.cs;
        let instruction_ip = self.cpu.ip;
        let mut segment_override = None;
        let mut repeat = None;
        let mut operand32 = false;
        let mut address32 = false;
        let opcode = loop {
            let opcode = self.fetch_u8();
            match opcode {
                0x66 if self.cpu_profile == CpuProfile::I80386RealMode => operand32 = true,
                0x67 if self.cpu_profile == CpuProfile::I80386RealMode => address32 = true,
                0x26 => segment_override = Some(self.cpu.es),
                0x2e => segment_override = Some(self.cpu.cs),
                0x36 => segment_override = Some(self.cpu.ss),
                0x3e => segment_override = Some(self.cpu.ds),
                0xf2 => repeat = Some(false),
                0xf3 => repeat = Some(true),
                _ => break opcode,
            }
            if self.cpu.ip.wrapping_sub(instruction_ip) >= 15 {
                self.state = GuestState::Trapped(Trap::MalformedPrefix {
                    cs: instruction_cs,
                    ip: instruction_ip,
                });
                return;
            }
        };
        let result = if address32 && opcode == 0xd7 {
            self.execute_xlat(segment_override, true)
        } else if address32 {
            // Staged explicitly: never decode a 32-bit EA as a 16-bit address.
            Err(self.bad_opcode(instruction_ip))
        } else if operand32 {
            self.execute32(opcode, segment_override, repeat, instruction_ip)
        } else {
            self.execute(opcode, segment_override, repeat, instruction_ip)
        };
        if let Err(trap) = result {
            if self.parent_process.is_some() {
                self.recovered_child_trap = Some(trap);
                self.restore_parent(ChildResult {
                    code: 1,
                    termination: TerminationType::RuntimeTrap,
                });
            } else {
                self.state = GuestState::Trapped(trap);
            }
        } else {
            self.instructions_retired = self.instructions_retired.wrapping_add(1);
        }
    }

    fn fetch_u32(&mut self) -> u32 {
        let lo = self.fetch_u16() as u32;
        lo | ((self.fetch_u16() as u32) << 16)
    }

    // Validate the complete operand before writing any byte. A20 stays disabled.
    fn validate_rm32(&self, operand: ModRm) -> Result<(), Trap> {
        if let Some((segment, offset)) = operand.memory {
            if offset > 0xfffc {
                return Err(Trap::SegmentLimit {
                    segment,
                    offset: u32::from(offset),
                    width: 4,
                });
            }
        }
        Ok(())
    }
    fn read_rm32(&self, operand: ModRm) -> Result<u32, Trap> {
        self.validate_rm32(operand)?;
        Ok(match operand.memory {
            None => self.cpu.reg32(operand.rm),
            Some((seg, off)) => {
                self.memory.read_u16(seg, off) as u32
                    | ((self.memory.read_u16(seg, off + 2) as u32) << 16)
            }
        })
    }
    fn write_rm32(&mut self, operand: ModRm, value: u32) -> Result<(), Trap> {
        self.validate_rm32(operand)?;
        if let Some((seg, off)) = operand.memory {
            self.memory.write_u16(seg, off, value as u16);
            self.memory.write_u16(seg, off + 2, (value >> 16) as u16);
        } else {
            self.cpu.set_reg32(operand.rm, value);
        }
        Ok(())
    }
    fn execute32(
        &mut self,
        opcode: u8,
        segment: Option<u16>,
        repeat: Option<bool>,
        ip: u16,
    ) -> Result<(), Trap> {
        if repeat.is_some() {
            return Err(self.bad_opcode(ip));
        }
        match opcode {
            0xd7 => self.execute_xlat(segment, false)?,
            0x99 => {
                let eax = self.cpu.reg32(0);
                self.cpu
                    .set_reg32(2, if eax & 0x8000_0000 != 0 { u32::MAX } else { 0 });
            }
            0xb8..=0xbf => {
                let value = self.fetch_u32();
                self.cpu.set_reg32(opcode - 0xb8, value);
            }
            0x89 | 0x8b | 0xc7 => {
                let operand = self.decode_modrm(segment)?;
                match opcode {
                    0x89 => self.write_rm32(operand, self.cpu.reg32(operand.register))?,
                    0x8b => {
                        let value = self.read_rm32(operand)?;
                        self.cpu.set_reg32(operand.register, value);
                    }
                    _ => {
                        if operand.register != 0 {
                            return Err(self.bad_opcode(ip));
                        }
                        let value = self.fetch_u32();
                        self.write_rm32(operand, value)?;
                    }
                }
            }
            0x0f => {
                let second = self.fetch_u8();
                if second == 0xaf {
                    return self.imul_truncated(second, segment, ip, true);
                }
                if !matches!(second, 0xa4 | 0xa5 | 0xac | 0xad) {
                    return Err(self.bad_opcode(ip));
                }
                let operand = self.decode_modrm(segment)?;
                let count = if second & 1 == 0 {
                    self.fetch_u8()
                } else {
                    self.cpu.cl()
                } & 31;
                let value = self.read_rm32(operand)?;
                if count != 0 {
                    let source = self.cpu.reg32(operand.register);
                    let left = second & 8 == 0;
                    let result = if left {
                        (value << count) | (source >> (32 - count))
                    } else {
                        (value >> count) | (source << (32 - count))
                    };
                    let carry = if left {
                        (value >> (32 - count)) & 1 != 0
                    } else {
                        (value >> (count - 1)) & 1 != 0
                    };
                    self.write_rm32(operand, result)?;
                    self.set_flag(CpuState::FLAG_CF, carry);
                    self.set_flag(CpuState::FLAG_ZF, result == 0);
                    self.set_flag(CpuState::FLAG_SF, result >> 31 != 0);
                    self.set_flag(CpuState::FLAG_PF, Self::parity(result as u8));
                    if count == 1 {
                        self.set_flag(CpuState::FLAG_OF, (value ^ result) >> 31 != 0);
                    }
                }
            }
            0x69 | 0x6b => self.imul_truncated(opcode, segment, ip, true)?,
            0xf7 => {
                let operand = self.decode_modrm(segment)?;
                match operand.register {
                    4 | 5 => {
                        let value = self.read_rm32(operand)?;
                        self.multiply_accumulator(value, 32, operand.register == 5);
                    }
                    7 => {
                        let divisor = self.read_rm32(operand)? as i32 as i64;
                        let dividend =
                            (i64::from(self.cpu.reg32(2)) << 32) | i64::from(self.cpu.reg32(0));
                        let quotient = dividend
                            .checked_div(divisor)
                            .and_then(|value| i32::try_from(value).ok())
                            .ok_or(Trap::DivideError {
                                cs: self.cpu.cs,
                                ip,
                            })?;
                        // Keep the existing IDIV policy of preserving undefined status flags.
                        self.cpu.set_reg32(0, quotient as u32);
                        self.cpu.set_reg32(2, (dividend % divisor) as u32);
                    }
                    _ => return Err(self.bad_opcode(ip)),
                }
            }
            0xf6 => self.group_three(opcode, segment, ip)?,
            0xc0 | 0xc1 | 0xd0..=0xd3 => self.group_shift(opcode, segment, ip, true)?,
            // Byte operations retain their width under 66h.
            0xb0..=0xb7 | 0x88 | 0x8a | 0xc6 | 0x90 | 0xd4 => {
                self.execute(opcode, segment, repeat, ip)?
            }
            _ => return Err(self.bad_opcode(ip)),
        }
        Ok(())
    }

    fn execute(
        &mut self,
        opcode: u8,
        override_segment: Option<u16>,
        repeat: Option<bool>,
        start_ip: u16,
    ) -> Result<(), Trap> {
        match opcode {
            0xd7 => self.execute_xlat(override_segment, false)?,
            0xd4 => self.execute_aam(start_ip)?,
            0x69 | 0x6b => self.imul_truncated(opcode, override_segment, start_ip, false)?,
            0x0f => {
                if self.cpu_profile != CpuProfile::I80386RealMode {
                    return Err(self.bad_opcode(start_ip));
                }
                let second = self.fetch_u8();
                if second != 0xaf {
                    return Err(self.bad_opcode(start_ip));
                }
                self.imul_truncated(second, override_segment, start_ip, false)?;
            }

            0x90 => {}
            0xb0..=0xb7 => {
                let value = self.fetch_u8();
                self.cpu.set_reg8(opcode - 0xb0, value);
            }
            0xb8..=0xbf => {
                let value = self.fetch_u16();
                self.cpu.set_reg16(opcode - 0xb8, value);
            }
            0x88 | 0x89 | 0x8a | 0x8b => {
                let operand = self.decode_modrm(override_segment)?;
                match opcode {
                    0x88 => self.write_rm8(operand, self.cpu.reg8(operand.register)),
                    0x89 => self.write_rm16(operand, self.cpu.reg16(operand.register)),
                    0x8a => {
                        let value = self.read_rm8(operand);
                        self.cpu.set_reg8(operand.register, value);
                    }
                    _ => {
                        let value = self.read_rm16(operand);
                        self.cpu.set_reg16(operand.register, value);
                    }
                }
            }
            0x8f => {
                let operand = self.decode_modrm(override_segment)?;
                if operand.register != 0 {
                    return Err(self.bad_opcode(start_ip));
                }
                let value = self.pop_u16();
                self.write_rm16(operand, value);
            }
            0x8c => {
                let operand = self.decode_modrm(override_segment)?;
                let value = self.segment_value(operand.register)?;
                self.write_rm16(operand, value);
            }
            0x8e => {
                let operand = self.decode_modrm(override_segment)?;
                let value = self.read_rm16(operand);
                self.set_segment(operand.register, value)?;
            }
            0x8d => {
                let operand = self.decode_modrm(override_segment)?;
                let (_, offset) = operand.memory.ok_or_else(|| self.bad_opcode(start_ip))?;
                self.cpu.set_reg16(operand.register, offset);
            }
            0xa0 => {
                let offset = self.fetch_u16();
                self.cpu.set_al(
                    self.memory
                        .read_u8(override_segment.unwrap_or(self.cpu.ds), offset),
                );
            }
            0xa1 => {
                let offset = self.fetch_u16();
                self.cpu.ax = self
                    .memory
                    .read_u16(override_segment.unwrap_or(self.cpu.ds), offset);
            }
            0xa2 => {
                let offset = self.fetch_u16();
                self.memory.write_u8(
                    override_segment.unwrap_or(self.cpu.ds),
                    offset,
                    self.cpu.al(),
                );
            }
            0xa3 => {
                let offset = self.fetch_u16();
                self.memory
                    .write_u16(override_segment.unwrap_or(self.cpu.ds), offset, self.cpu.ax);
            }
            0xc6 | 0xc7 => {
                let operand = self.decode_modrm(override_segment)?;
                if operand.register != 0 {
                    return Err(self.bad_opcode(start_ip));
                }
                if opcode == 0xc6 {
                    let value = self.fetch_u8();
                    self.write_rm8(operand, value);
                } else {
                    let value = self.fetch_u16();
                    self.write_rm16(operand, value);
                }
            }
            0x50..=0x57 => self.push_u16(self.cpu.reg16(opcode - 0x50)),
            0x58..=0x5f => {
                let value = self.pop_u16();
                self.cpu.set_reg16(opcode - 0x58, value);
            }
            0x06 | 0x0e | 0x16 | 0x1e => self.push_u16(self.segment_value((opcode >> 3) & 3)?),
            0x07 | 0x17 | 0x1f => {
                let value = self.pop_u16();
                self.set_segment((opcode >> 3) & 3, value)?;
            }
            0x9c => self.push_u16(self.cpu.flags | 0x0002),
            0x9d => self.cpu.flags = self.pop_u16() | 0x0002,
            0x9e => {
                let preserved = self.cpu.flags & 0xff02;
                self.cpu.flags = preserved | ((self.cpu.ah() as u16) & 0xd5);
            }
            0x9f => self.cpu.set_ah((self.cpu.flags & 0x00d5) as u8),
            0x9a => {
                let offset = self.fetch_u16();
                let segment = self.fetch_u16();
                self.push_u16(self.cpu.cs);
                self.push_u16(self.cpu.ip);
                self.cpu.cs = segment;
                self.cpu.ip = offset;
            }
            0xe8 => {
                let displacement = self.fetch_u16() as i16;
                self.push_u16(self.cpu.ip);
                self.cpu.ip = self.cpu.ip.wrapping_add(displacement as u16);
            }
            0xc3 => self.cpu.ip = self.pop_u16(),
            0xc2 => {
                let bytes = self.fetch_u16();
                self.cpu.ip = self.pop_u16();
                self.cpu.sp = self.cpu.sp.wrapping_add(bytes);
            }
            0xcb => {
                self.cpu.ip = self.pop_u16();
                self.cpu.cs = self.pop_u16();
            }
            0xca => {
                let bytes = self.fetch_u16();
                self.cpu.ip = self.pop_u16();
                self.cpu.cs = self.pop_u16();
                self.cpu.sp = self.cpu.sp.wrapping_add(bytes);
            }
            0xcf => {
                self.cpu.ip = self.pop_u16();
                self.cpu.cs = self.pop_u16();
                self.cpu.flags = self.pop_u16() | 0x0002;
            }
            0xea => {
                let offset = self.fetch_u16();
                self.cpu.cs = self.fetch_u16();
                self.cpu.ip = offset;
            }
            0xeb => {
                let displacement = self.fetch_u8() as i8;
                self.cpu.ip = self.cpu.ip.wrapping_add(displacement as i16 as u16);
            }
            0xe9 => {
                let displacement = self.fetch_u16() as i16;
                self.cpu.ip = self.cpu.ip.wrapping_add(displacement as u16);
            }
            0x70..=0x7f => {
                let displacement = self.fetch_u8() as i8;
                if self.condition(opcode & 0x0f) {
                    self.cpu.ip = self.cpu.ip.wrapping_add(displacement as i16 as u16);
                }
            }
            0xe0..=0xe2 => {
                let displacement = self.fetch_u8() as i8;
                self.cpu.cx = self.cpu.cx.wrapping_sub(1);
                let zero = self.cpu.flags & CpuState::FLAG_ZF != 0;
                let condition = match opcode {
                    0xe0 => !zero,
                    0xe1 => zero,
                    _ => true,
                };
                if self.cpu.cx != 0 && condition {
                    self.cpu.ip = self.cpu.ip.wrapping_add(displacement as i16 as u16);
                }
            }
            0xe3 => {
                let displacement = self.fetch_u8() as i8;
                if self.cpu.cx == 0 {
                    self.cpu.ip = self.cpu.ip.wrapping_add(displacement as i16 as u16);
                }
            }
            0xe4..=0xe7 | 0xec..=0xef => {
                let immediate_port = if opcode <= 0xe7 {
                    Some(self.fetch_u8())
                } else {
                    None
                };
                self.io
                    .execute(
                        self.memory.vga_addressing_enabled(),
                        &mut self.cpu,
                        opcode,
                        immediate_port,
                    )
                    .map_err(|trap| self.guest_io_trap(trap, start_ip))?;
            }
            0x04 => {
                let left = self.cpu.al();
                let right = self.fetch_u8();
                let value = self.add8(left, right);
                self.cpu.set_al(value);
            }
            0x05 => {
                let left = self.cpu.ax;
                let right = self.fetch_u16();
                self.cpu.ax = self.add16(left, right);
            }
            0x14 => {
                let immediate = self.fetch_u8();
                let value = self.adc8(self.cpu.al(), immediate);
                self.cpu.set_al(value);
            }
            0x15 => {
                let immediate = self.fetch_u16();
                self.cpu.ax = self.adc16(self.cpu.ax, immediate);
            }
            0x0c => {
                let immediate = self.fetch_u8();
                let value = self.logic8(self.cpu.al() | immediate);
                self.cpu.set_al(value);
            }
            0x0d => {
                let immediate = self.fetch_u16();
                self.cpu.ax = self.logic16(self.cpu.ax | immediate);
            }
            0x24 => {
                let immediate = self.fetch_u8();
                let value = self.logic8(self.cpu.al() & immediate);
                self.cpu.set_al(value);
            }
            0x25 => {
                let immediate = self.fetch_u16();
                self.cpu.ax = self.logic16(self.cpu.ax & immediate);
            }
            0x2c => {
                let left = self.cpu.al();
                let right = self.fetch_u8();
                let value = self.sub8(left, right);
                self.cpu.set_al(value);
            }
            0x2d => {
                let left = self.cpu.ax;
                let right = self.fetch_u16();
                self.cpu.ax = self.sub16(left, right);
            }
            0x1c => {
                let immediate = self.fetch_u8();
                let value = self.sbb8(self.cpu.al(), immediate);
                self.cpu.set_al(value);
            }
            0x1d => {
                let immediate = self.fetch_u16();
                self.cpu.ax = self.sbb16(self.cpu.ax, immediate);
            }
            0x3c => {
                let left = self.cpu.al();
                let right = self.fetch_u8();
                self.sub8(left, right);
            }
            0x3d => {
                let left = self.cpu.ax;
                let right = self.fetch_u16();
                self.sub16(left, right);
            }
            0x34 => {
                let immediate = self.fetch_u8();
                let value = self.logic8(self.cpu.al() ^ immediate);
                self.cpu.set_al(value);
            }
            0x35 => {
                let immediate = self.fetch_u16();
                self.cpu.ax = self.logic16(self.cpu.ax ^ immediate);
            }
            0xa8 => {
                let immediate = self.fetch_u8();
                self.logic8(self.cpu.al() & immediate);
            }
            0xa9 => {
                let immediate = self.fetch_u16();
                self.logic16(self.cpu.ax & immediate);
            }
            0x00..=0x03
            | 0x28..=0x2b
            | 0x38..=0x3b
            | 0x30..=0x33
            | 0x20..=0x23
            | 0x08..=0x0b
            | 0x10..=0x13
            | 0x18..=0x1b
            | 0x84
            | 0x85 => self.binary_modrm(opcode, override_segment)?,
            0x40..=0x47 => {
                let value = self.cpu.reg16(opcode - 0x40);
                let value = self.inc16(value);
                self.cpu.set_reg16(opcode - 0x40, value);
            }
            0x48..=0x4f => {
                let value = self.cpu.reg16(opcode - 0x48);
                let value = self.dec16(value);
                self.cpu.set_reg16(opcode - 0x48, value);
            }
            0x80 | 0x81 | 0x83 => self.group_one(opcode, override_segment)?,
            0xfe => {
                let operand = self.decode_modrm(override_segment)?;
                let value = self.read_rm8(operand);
                let value = match operand.register {
                    0 => self.inc8(value),
                    1 => self.dec8(value),
                    _ => return Err(self.bad_opcode(start_ip)),
                };
                self.write_rm8(operand, value);
            }
            0xc4 | 0xc5 => {
                let operand = self.decode_modrm(override_segment)?;
                let (segment, offset) = operand.memory.ok_or_else(|| self.bad_opcode(start_ip))?;
                let pointer = self.memory.read_u16(segment, offset);
                let selector = self.memory.read_u16(segment, offset.wrapping_add(2));
                self.cpu.set_reg16(operand.register, pointer);
                if opcode == 0xc4 {
                    self.cpu.es = selector;
                } else {
                    self.cpu.ds = selector;
                }
            }
            0xf6 | 0xf7 => self.group_three(opcode, override_segment, start_ip)?,
            0xc0 | 0xc1 | 0xd0..=0xd3 => {
                self.group_shift(opcode, override_segment, start_ip, false)?
            }
            0xff => self.group_five(override_segment, start_ip)?,
            0x68 | 0x6a | 0xc8 | 0xc9 => self.execute_80186(opcode, start_ip)?,
            0xcc => dos::dispatch(self, 3)?,
            0xce => {
                if self.cpu.flags & CpuState::FLAG_OF != 0 {
                    dos::dispatch(self, 4)?;
                }
            }
            0xfc => self.cpu.flags &= !CpuState::FLAG_DF,
            0xfd => self.cpu.flags |= CpuState::FLAG_DF,
            0xf8 => self.cpu.flags &= !CpuState::FLAG_CF,
            0xf9 => self.cpu.flags |= CpuState::FLAG_CF,
            0xfa => self.cpu.flags &= !CpuState::FLAG_IF,
            0xfb => self.cpu.flags |= CpuState::FLAG_IF,
            0x9b => {}
            0x98 => self.cpu.ax = self.cpu.al() as i8 as i16 as u16,
            0x99 => {
                self.cpu.dx = if self.cpu.ax & 0x8000 != 0 { 0xffff } else { 0 };
            }
            0x86 | 0x87 => {
                let operand = self.decode_modrm(override_segment)?;
                if opcode == 0x86 {
                    let left = self.read_rm8(operand);
                    let right = self.cpu.reg8(operand.register);
                    self.write_rm8(operand, right);
                    self.cpu.set_reg8(operand.register, left);
                } else {
                    let left = self.read_rm16(operand);
                    let right = self.cpu.reg16(operand.register);
                    self.write_rm16(operand, right);
                    self.cpu.set_reg16(operand.register, left);
                }
            }
            0x91..=0x97 => {
                let register = opcode - 0x90;
                let value = self.cpu.reg16(register);
                self.cpu.set_reg16(register, self.cpu.ax);
                self.cpu.ax = value;
            }
            0xa4..=0xaf => self.string_op(opcode, override_segment, repeat, start_ip),
            0xd8..=0xdf => {
                let _ = self.decode_modrm(override_segment)?;
            }
            0xcd => {
                let interrupt = self.fetch_u8();
                dos::dispatch(self, interrupt)?;
            }
            0xf4 => self.state = GuestState::Halted,
            _ => return Err(self.bad_opcode(start_ip)),
        }
        Ok(())
    }

    fn binary_modrm(&mut self, opcode: u8, override_segment: Option<u16>) -> Result<(), Trap> {
        let operand = self.decode_modrm(override_segment)?;
        let width16 = opcode & 1 != 0 || opcode == 0x85;
        let direction = opcode & 2 != 0;
        let kind = opcode & 0xf8;
        if width16 {
            let rm = self.read_rm16(operand);
            let reg = self.cpu.reg16(operand.register);
            let (left, right) = if direction { (reg, rm) } else { (rm, reg) };
            let is_compare = kind == 0x38;
            let is_test = opcode == 0x85;
            let value = match kind {
                0x00 => self.add16(left, right),
                0x10 => self.adc16(left, right),
                0x18 => self.sbb16(left, right),
                0x28 => self.sub16(left, right),
                0x30 => self.logic16(left ^ right),
                0x20 => self.logic16(left & right),
                0x08 => self.logic16(left | right),
                0x38 => {
                    self.sub16(left, right);
                    left
                }
                _ if is_test => self.logic16(left & right),
                _ => self.logic16(left & right),
            };
            if !is_compare && !is_test {
                if direction {
                    self.cpu.set_reg16(operand.register, value);
                } else {
                    self.write_rm16(operand, value);
                }
            }
        } else {
            let rm = self.read_rm8(operand);
            let reg = self.cpu.reg8(operand.register);
            let (left, right) = if direction { (reg, rm) } else { (rm, reg) };
            let is_compare = kind == 0x38;
            let is_test = opcode == 0x84;
            let value = match kind {
                0x00 => self.add8(left, right),
                0x10 => self.adc8(left, right),
                0x18 => self.sbb8(left, right),
                0x28 => self.sub8(left, right),
                0x30 => self.logic8(left ^ right),
                0x20 => self.logic8(left & right),
                0x08 => self.logic8(left | right),
                0x38 => {
                    self.sub8(left, right);
                    left
                }
                _ if is_test => self.logic8(left & right),
                _ => self.logic8(left & right),
            };
            if !is_compare && !is_test {
                if direction {
                    self.cpu.set_reg8(operand.register, value);
                } else {
                    self.write_rm8(operand, value);
                }
            }
        }
        Ok(())
    }

    fn group_one(&mut self, opcode: u8, override_segment: Option<u16>) -> Result<(), Trap> {
        let operand = self.decode_modrm(override_segment)?;
        let width16 = opcode != 0x80;
        let immediate = if opcode == 0x80 {
            self.fetch_u8() as u16
        } else if opcode == 0x81 {
            self.fetch_u16()
        } else {
            self.fetch_u8() as i8 as i16 as u16
        };
        if width16 {
            let left = self.read_rm16(operand);
            let value = match operand.register {
                0 => self.add16(left, immediate),
                1 => self.logic16(left | immediate),
                2 => self.adc16(left, immediate),
                3 => self.sbb16(left, immediate),
                4 => self.logic16(left & immediate),
                5 => self.sub16(left, immediate),
                6 => self.logic16(left ^ immediate),
                7 => {
                    self.sub16(left, immediate);
                    left
                }
                _ => return Err(self.bad_opcode(self.cpu.ip)),
            };
            if operand.register != 7 {
                self.write_rm16(operand, value);
            }
        } else {
            let left = self.read_rm8(operand);
            let immediate = immediate as u8;
            let value = match operand.register {
                0 => self.add8(left, immediate),
                1 => self.logic8(left | immediate),
                2 => self.adc8(left, immediate),
                3 => self.sbb8(left, immediate),
                4 => self.logic8(left & immediate),
                5 => self.sub8(left, immediate),
                6 => self.logic8(left ^ immediate),
                7 => {
                    self.sub8(left, immediate);
                    left
                }
                _ => return Err(self.bad_opcode(self.cpu.ip)),
            };
            if operand.register != 7 {
                self.write_rm8(operand, value);
            }
        }
        Ok(())
    }

    fn group_five(&mut self, override_segment: Option<u16>, start_ip: u16) -> Result<(), Trap> {
        let operand = self.decode_modrm(override_segment)?;
        match operand.register {
            0 => {
                let value = self.inc16(self.read_rm16(operand));
                self.write_rm16(operand, value);
            }
            1 => {
                let value = self.dec16(self.read_rm16(operand));
                self.write_rm16(operand, value);
            }
            2 => {
                let value = self.read_rm16(operand);
                self.push_u16(self.cpu.ip);
                self.cpu.ip = value;
            }
            3 => {
                let (segment, offset) = operand.memory.ok_or_else(|| self.bad_opcode(start_ip))?;
                let target_offset = self.memory.read_u16(segment, offset);
                let target_segment = self.memory.read_u16(segment, offset.wrapping_add(2));
                self.push_u16(self.cpu.cs);
                self.push_u16(self.cpu.ip);
                self.cpu.cs = target_segment;
                self.cpu.ip = target_offset;
            }
            4 => self.cpu.ip = self.read_rm16(operand),
            5 => {
                let (segment, offset) = operand.memory.ok_or_else(|| self.bad_opcode(start_ip))?;
                self.cpu.ip = self.memory.read_u16(segment, offset);
                self.cpu.cs = self.memory.read_u16(segment, offset.wrapping_add(2));
            }
            6 => self.push_u16(self.read_rm16(operand)),
            _ => return Err(self.bad_opcode(start_ip)),
        }
        Ok(())
    }

    // Undefined arithmetic flags are retained, matching the existing shift policy.
    fn multiply(&mut self, left: u32, right: u32, bits: u32, signed: bool) -> u64 {
        let mask = (1u64 << bits) - 1;
        let (product, overflow) = if signed {
            let extend = |value: u32| ((value as i64) << (64 - bits)) >> (64 - bits);
            let product = extend(left) * extend(right);
            let low = ((product << (64 - bits)) >> (64 - bits)) as i64;
            (product as u64, product != low)
        } else {
            let product = (left as u64 & mask) * (right as u64 & mask);
            (product, product >> bits != 0)
        };
        self.set_flag(CpuState::FLAG_CF, overflow);
        self.set_flag(CpuState::FLAG_OF, overflow);
        product
    }

    fn multiply_accumulator(&mut self, value: u32, bits: u32, signed: bool) {
        let left = match bits {
            8 => self.cpu.al() as u32,
            16 => self.cpu.ax as u32,
            _ => self.cpu.reg32(0),
        };
        let product = self.multiply(left, value, bits, signed);
        match bits {
            8 => self.cpu.ax = product as u16,
            16 => {
                self.cpu.ax = product as u16;
                self.cpu.dx = (product >> 16) as u16;
            }
            _ => {
                self.cpu.set_reg32(0, product as u32);
                self.cpu.set_reg32(2, (product >> 32) as u32);
            }
        }
    }

    fn imul_truncated(
        &mut self,
        opcode: u8,
        segment: Option<u16>,
        ip: u16,
        dword: bool,
    ) -> Result<(), Trap> {
        if (opcode == 0xaf && self.cpu_profile != CpuProfile::I80386RealMode)
            || (opcode != 0xaf && !self.cpu_profile.supports_80186())
        {
            return Err(Trap::CpuProfileViolation {
                cs: self.cpu.cs,
                ip,
                opcode,
            });
        }
        let operand = self.decode_modrm(segment)?;
        let source = if dword {
            self.read_rm32(operand)?
        } else {
            self.read_rm16(operand) as u32
        };
        let other = match opcode {
            0xaf => {
                if dword {
                    self.cpu.reg32(operand.register)
                } else {
                    self.cpu.reg16(operand.register) as u32
                }
            }
            0x6b => self.fetch_u8() as i8 as i32 as u32,
            _ => {
                if dword {
                    self.fetch_u32()
                } else {
                    self.fetch_u16() as u32
                }
            }
        };
        let product = self.multiply(source, other, if dword { 32 } else { 16 }, true);
        if dword {
            self.cpu.set_reg32(operand.register, product as u32);
        } else {
            self.cpu.set_reg16(operand.register, product as u16);
        }
        Ok(())
    }

    fn group_three(
        &mut self,
        opcode: u8,
        override_segment: Option<u16>,
        start_ip: u16,
    ) -> Result<(), Trap> {
        let operand = self.decode_modrm(override_segment)?;
        let word = opcode == 0xf7;
        let value = if word {
            self.read_rm16(operand) as u32
        } else {
            self.read_rm8(operand) as u32
        };
        match operand.register {
            0 => {
                let immediate = if word {
                    self.fetch_u16() as u32
                } else {
                    self.fetch_u8() as u32
                };
                if word {
                    self.logic16((value as u16) & immediate as u16);
                } else {
                    self.logic8((value as u8) & immediate as u8);
                }
            }
            2 => {
                if word {
                    self.write_rm16(operand, !(value as u16));
                } else {
                    self.write_rm8(operand, !(value as u8));
                }
            }
            3 => {
                if word {
                    let result = (0u16).wrapping_sub(value as u16);
                    self.sub16(0, value as u16);
                    self.write_rm16(operand, result);
                } else {
                    let result = (0u8).wrapping_sub(value as u8);
                    self.sub8(0, value as u8);
                    self.write_rm8(operand, result);
                }
            }
            4 | 5 => {
                self.multiply_accumulator(value, if word { 16 } else { 8 }, operand.register == 5)
            }
            6 => {
                if value == 0 {
                    return Err(Trap::DivideError {
                        cs: self.cpu.cs,
                        ip: start_ip,
                    });
                }
                if word {
                    let dividend = ((self.cpu.dx as u32) << 16) | self.cpu.ax as u32;
                    let quotient = dividend / value;
                    if quotient > u16::MAX as u32 {
                        return Err(Trap::DivideError {
                            cs: self.cpu.cs,
                            ip: start_ip,
                        });
                    }
                    self.cpu.ax = quotient as u16;
                    self.cpu.dx = (dividend % value) as u16;
                } else {
                    let dividend = self.cpu.ax;
                    let quotient = dividend / value as u16;
                    if quotient > u8::MAX as u16 {
                        return Err(Trap::DivideError {
                            cs: self.cpu.cs,
                            ip: start_ip,
                        });
                    }
                    self.cpu.set_al(quotient as u8);
                    self.cpu.set_ah((dividend % value as u16) as u8);
                }
            }
            7 => {
                if value == 0 {
                    return Err(Trap::DivideError {
                        cs: self.cpu.cs,
                        ip: start_ip,
                    });
                }
                if word {
                    let dividend = ((self.cpu.dx as i32) << 16) | self.cpu.ax as i32;
                    let divisor = value as u16 as i16 as i32;
                    let quotient = dividend
                        .checked_div(divisor)
                        .and_then(|value| i16::try_from(value).ok())
                        .ok_or(Trap::DivideError {
                            cs: self.cpu.cs,
                            ip: start_ip,
                        })?;
                    self.cpu.ax = quotient as u16;
                    self.cpu.dx = (dividend % divisor) as u16;
                } else {
                    let dividend = self.cpu.ax as i16;
                    let divisor = value as u8 as i8 as i16;
                    let quotient = dividend / divisor;
                    if !(i8::MIN as i16..=i8::MAX as i16).contains(&quotient) {
                        return Err(Trap::DivideError {
                            cs: self.cpu.cs,
                            ip: start_ip,
                        });
                    }
                    self.cpu.set_al(quotient as u8);
                    self.cpu.set_ah((dividend % divisor) as u8);
                }
            }
            _ => return Err(self.bad_opcode(start_ip)),
        }
        Ok(())
    }

    fn group_shift(
        &mut self,
        opcode: u8,
        override_segment: Option<u16>,
        start_ip: u16,
        operand32: bool,
    ) -> Result<(), Trap> {
        if matches!(opcode, 0xc0 | 0xc1) && !self.cpu_profile.supports_80186() {
            return Err(Trap::CpuProfileViolation {
                cs: self.cpu.cs,
                ip: start_ip,
                opcode,
            });
        }
        let operand = self.decode_modrm(override_segment)?;
        // Group 2 /6 is reserved; do not silently treat it as SHR.
        if operand.register == 6 {
            return Err(self.bad_opcode(start_ip));
        }
        let raw_count = match opcode {
            0xc0 | 0xc1 => self.fetch_u8(),
            0xd0 | 0xd1 => 1,
            _ => self.cpu.cl(),
        };
        // 8086 consumes all eight CL bits. 186/286 bound counts to five bits.
        let mut count = if self.cpu_profile == CpuProfile::I8086 {
            raw_count
        } else {
            raw_count & 0x1f
        };
        // Through-carry rotations have a width+1 ring. Full-ring counts
        // leave both the operand and flags unchanged on the later profiles.
        if self.cpu_profile != CpuProfile::I8086 && matches!(operand.register, 2 | 3) {
            count %= if opcode & 1 == 0 {
                9
            } else if operand32 {
                33
            } else {
                17
            };
        }
        let word = opcode & 1 != 0;
        if word && operand32 {
            let old = self.read_rm32(operand)?;
            if count != 0 {
                let value = self.shift_width(operand.register, old, count, 32, start_ip)?;
                self.write_rm32(operand, value)?;
            }
            return Ok(());
        }
        if count == 0 {
            return Ok(());
        }
        if word {
            let value = self.shift16(operand.register, self.read_rm16(operand), count, start_ip)?;
            self.write_rm16(operand, value);
        } else {
            let value = self.shift8(operand.register, self.read_rm8(operand), count, start_ip)?;
            self.write_rm8(operand, value);
        }
        Ok(())
    }

    fn execute_80186(&mut self, opcode: u8, start_ip: u16) -> Result<(), Trap> {
        if !self.cpu_profile.supports_80186() {
            return Err(Trap::CpuProfileViolation {
                cs: self.cpu.cs,
                ip: start_ip,
                opcode,
            });
        }
        match opcode {
            0x68 => {
                let immediate = self.fetch_u16();
                self.push_u16(immediate);
            }
            0x6a => {
                let immediate = self.fetch_u8() as i8 as i16 as u16;
                self.push_u16(immediate);
            }
            0xc8 => {
                let size = self.fetch_u16();
                let nesting = self.fetch_u8() & 0x1f;
                self.push_u16(self.cpu.bp);
                let frame = self.cpu.sp;
                for _ in 1..nesting {
                    self.cpu.bp = self.cpu.bp.wrapping_sub(2);
                    self.push_u16(self.memory.read_u16(self.cpu.ss, self.cpu.bp));
                }
                if nesting != 0 {
                    self.push_u16(frame);
                }
                self.cpu.bp = frame;
                self.cpu.sp = self.cpu.sp.wrapping_sub(size);
            }
            0xc9 => {
                self.cpu.sp = self.cpu.bp;
                self.cpu.bp = self.pop_u16();
            }
            _ => return Err(self.bad_opcode(start_ip)),
        }
        Ok(())
    }

    fn string_op(
        &mut self,
        opcode: u8,
        override_segment: Option<u16>,
        repeat: Option<bool>,
        start_ip: u16,
    ) {
        if repeat.is_some() && self.cpu.cx == 0 {
            return;
        }
        let word = opcode & 1 != 0;
        let step = if self.cpu.flags & CpuState::FLAG_DF != 0 {
            if word {
                0xfffe
            } else {
                0xffff
            }
        } else if word {
            2
        } else {
            1
        };
        match opcode {
            0xa4 | 0xa5 => {
                let source_segment = override_segment.unwrap_or(self.cpu.ds);
                if word {
                    let value = self.memory.read_u16(source_segment, self.cpu.si);
                    self.memory.write_u16(self.cpu.es, self.cpu.di, value);
                } else {
                    let value = self.memory.read_u8(source_segment, self.cpu.si);
                    self.memory.write_u8(self.cpu.es, self.cpu.di, value);
                }
                self.cpu.si = self.cpu.si.wrapping_add(step);
                self.cpu.di = self.cpu.di.wrapping_add(step);
            }
            0xa6 | 0xa7 => {
                let source_segment = override_segment.unwrap_or(self.cpu.ds);
                if word {
                    let source = self.memory.read_u16(source_segment, self.cpu.si);
                    let destination = self.memory.read_u16(self.cpu.es, self.cpu.di);
                    self.sub16(source, destination);
                } else {
                    let source = self.memory.read_u8(source_segment, self.cpu.si);
                    let destination = self.memory.read_u8(self.cpu.es, self.cpu.di);
                    self.sub8(source, destination);
                }
                self.cpu.si = self.cpu.si.wrapping_add(step);
                self.cpu.di = self.cpu.di.wrapping_add(step);
            }
            0xaa | 0xab => {
                if word {
                    self.memory.write_u16(self.cpu.es, self.cpu.di, self.cpu.ax);
                } else {
                    self.memory
                        .write_u8(self.cpu.es, self.cpu.di, self.cpu.al());
                }
                self.cpu.di = self.cpu.di.wrapping_add(step);
            }
            0xae | 0xaf => {
                if word {
                    let value = self.memory.read_u16(self.cpu.es, self.cpu.di);
                    self.sub16(self.cpu.ax, value);
                } else {
                    let value = self.memory.read_u8(self.cpu.es, self.cpu.di);
                    self.sub8(self.cpu.al(), value);
                }
                self.cpu.di = self.cpu.di.wrapping_add(step);
            }
            _ => {
                let source_segment = override_segment.unwrap_or(self.cpu.ds);
                if word {
                    self.cpu.ax = self.memory.read_u16(source_segment, self.cpu.si);
                } else {
                    self.cpu
                        .set_al(self.memory.read_u8(source_segment, self.cpu.si));
                }
                self.cpu.si = self.cpu.si.wrapping_add(step);
            }
        }
        if let Some(repeat_while_equal) = repeat {
            self.cpu.cx = self.cpu.cx.wrapping_sub(1);
            let stop_on_match = matches!(opcode, 0xa6 | 0xa7 | 0xae | 0xaf)
                && (self.cpu.flags & CpuState::FLAG_ZF != 0) != repeat_while_equal;
            if self.cpu.cx != 0 && !stop_on_match {
                self.cpu.ip = start_ip;
            }
        }
    }

    fn decode_modrm(&mut self, override_segment: Option<u16>) -> Result<ModRm, Trap> {
        let byte = self.fetch_u8();
        let mode = byte >> 6;
        let register = (byte >> 3) & 7;
        let rm = byte & 7;
        if mode == 3 {
            return Ok(ModRm {
                register,
                rm,
                memory: None,
            });
        }
        let displacement = match mode {
            0 if rm == 6 => self.fetch_u16(),
            1 => self.fetch_u8() as i8 as i16 as u16,
            2 => self.fetch_u16(),
            _ => 0,
        };
        let (base, uses_bp) = match rm {
            0 => (self.cpu.bx.wrapping_add(self.cpu.si), false),
            1 => (self.cpu.bx.wrapping_add(self.cpu.di), false),
            2 => (self.cpu.bp.wrapping_add(self.cpu.si), true),
            3 => (self.cpu.bp.wrapping_add(self.cpu.di), true),
            4 => (self.cpu.si, false),
            5 => (self.cpu.di, false),
            6 if mode == 0 => (0, false),
            6 => (self.cpu.bp, true),
            _ => (self.cpu.bx, false),
        };
        Ok(ModRm {
            register,
            rm,
            memory: Some((
                override_segment.unwrap_or(if uses_bp { self.cpu.ss } else { self.cpu.ds }),
                base.wrapping_add(displacement),
            )),
        })
    }

    /// XLAT uses the address-size-selected BX/EBX plus unsigned AL, then reads
    /// one byte through the ordinary guest memory bus. Real-mode segment
    /// limits still apply when a 32-bit address calculation is selected.
    fn execute_xlat(&mut self, override_segment: Option<u16>, address32: bool) -> Result<(), Trap> {
        let offset = if address32 {
            self.cpu.reg32(3).wrapping_add(u32::from(self.cpu.al()))
        } else {
            u32::from(self.cpu.bx.wrapping_add(u16::from(self.cpu.al())))
        };
        if offset > u32::from(u16::MAX) {
            return Err(Trap::SegmentLimit {
                segment: override_segment.unwrap_or(self.cpu.ds),
                offset,
                width: 1,
            });
        }
        let segment = override_segment.unwrap_or(self.cpu.ds);
        let value = self.memory.read_u8(segment, offset as u16);
        self.cpu.set_al(value);
        Ok(())
    }

    /// ASCII-adjust-after-multiply is always an 8-bit operation.  Its
    /// immediate radix is part of the instruction even when a 66h operand
    /// override selected this dispatcher through the byte-operation fallback.
    fn execute_aam(&mut self, start_ip: u16) -> Result<(), Trap> {
        let base = self.fetch_u8();
        if base == 0 {
            return Err(Trap::DivideError {
                cs: self.cpu.cs,
                ip: start_ip,
            });
        }
        let al = self.cpu.al();
        self.cpu.set_ah(al / base);
        self.cpu.set_al(al % base);
        // CF, AF, and OF are undefined and retain Chronos's existing policy;
        // SF, ZF, and PF describe the resulting AL.
        self.flags8(self.cpu.al());
        Ok(())
    }

    fn read_rm8(&self, operand: ModRm) -> u8 {
        operand.memory.map_or_else(
            || self.cpu.reg8(operand.rm),
            |(segment, offset)| self.memory.read_u8(segment, offset),
        )
    }
    fn read_rm16(&self, operand: ModRm) -> u16 {
        operand.memory.map_or_else(
            || self.cpu.reg16(operand.rm),
            |(segment, offset)| self.memory.read_u16(segment, offset),
        )
    }
    fn write_rm8(&mut self, operand: ModRm, value: u8) {
        if let Some((segment, offset)) = operand.memory {
            self.memory.write_u8(segment, offset, value);
        } else {
            self.cpu.set_reg8(operand.rm, value);
        }
    }
    fn write_rm16(&mut self, operand: ModRm, value: u16) {
        if let Some((segment, offset)) = operand.memory {
            self.memory.write_u16(segment, offset, value);
        } else {
            self.cpu.set_reg16(operand.rm, value);
        }
    }
    fn fetch_u8(&mut self) -> u8 {
        let value = self.memory.read_u8(self.cpu.cs, self.cpu.ip);
        self.cpu.ip = self.cpu.ip.wrapping_add(1);
        value
    }
    fn fetch_u16(&mut self) -> u16 {
        u16::from_le_bytes([self.fetch_u8(), self.fetch_u8()])
    }
    fn push_u16(&mut self, value: u16) {
        self.cpu.sp = self.cpu.sp.wrapping_sub(2);
        self.memory.write_u16(self.cpu.ss, self.cpu.sp, value);
    }
    fn pop_u16(&mut self) -> u16 {
        let value = self.memory.read_u16(self.cpu.ss, self.cpu.sp);
        self.cpu.sp = self.cpu.sp.wrapping_add(2);
        value
    }
    fn segment_value(&self, register: u8) -> Result<u16, Trap> {
        match register {
            0 => Ok(self.cpu.es),
            1 => Ok(self.cpu.cs),
            2 => Ok(self.cpu.ss),
            3 => Ok(self.cpu.ds),
            _ => Err(Trap::InvalidSegmentRegister { encoding: register }),
        }
    }
    fn set_segment(&mut self, register: u8, value: u16) -> Result<(), Trap> {
        match register {
            0 => self.cpu.es = value,
            2 => self.cpu.ss = value,
            3 => self.cpu.ds = value,
            _ => return Err(Trap::InvalidSegmentRegister { encoding: register }),
        };
        Ok(())
    }
    fn bad_opcode(&self, ip: u16) -> Trap {
        Trap::UnsupportedOpcode {
            cs: self.cpu.cs,
            ip,
            bytes: self.instruction_bytes(ip),
            cpu: self.cpu,
        }
    }

    fn guest_io_trap(&self, trap: IoTrap, ip: u16) -> Trap {
        Trap::UnsupportedIoPort {
            operation: trap.operation,
            port: trap.port,
            width: trap.width,
            value: trap.value,
            cs: self.cpu.cs,
            ip,
            active_executable: self.active_process.executable_path.clone(),
            application_id: self.environment_app_id.clone(),
        }
    }
    fn instruction_bytes(&self, ip: u16) -> [u8; 4] {
        [
            self.memory.read_u8(self.cpu.cs, ip),
            self.memory.read_u8(self.cpu.cs, ip.wrapping_add(1)),
            self.memory.read_u8(self.cpu.cs, ip.wrapping_add(2)),
            self.memory.read_u8(self.cpu.cs, ip.wrapping_add(3)),
        ]
    }

    fn set_flag(&mut self, flag: u16, value: bool) {
        if value {
            self.cpu.flags |= flag;
        } else {
            self.cpu.flags &= !flag;
        }
    }
    fn parity(value: u8) -> bool {
        value.count_ones() % 2 == 0
    }
    fn flags8(&mut self, value: u8) {
        self.set_flag(CpuState::FLAG_ZF, value == 0);
        self.set_flag(CpuState::FLAG_SF, value & 0x80 != 0);
        self.set_flag(CpuState::FLAG_PF, Self::parity(value));
    }
    fn flags16(&mut self, value: u16) {
        self.set_flag(CpuState::FLAG_ZF, value == 0);
        self.set_flag(CpuState::FLAG_SF, value & 0x8000 != 0);
        self.set_flag(CpuState::FLAG_PF, Self::parity(value as u8));
    }
    fn add8(&mut self, left: u8, right: u8) -> u8 {
        let (value, carry) = left.overflowing_add(right);
        self.flags8(value);
        self.set_flag(CpuState::FLAG_CF, carry);
        self.set_flag(CpuState::FLAG_AF, (left & 0xf) + (right & 0xf) > 0xf);
        self.set_flag(
            CpuState::FLAG_OF,
            (!(left ^ right) & (left ^ value) & 0x80) != 0,
        );
        value
    }
    fn add16(&mut self, left: u16, right: u16) -> u16 {
        let (value, carry) = left.overflowing_add(right);
        self.flags16(value);
        self.set_flag(CpuState::FLAG_CF, carry);
        self.set_flag(CpuState::FLAG_AF, (left & 0xf) + (right & 0xf) > 0xf);
        self.set_flag(
            CpuState::FLAG_OF,
            (!(left ^ right) & (left ^ value) & 0x8000) != 0,
        );
        value
    }
    fn adc8(&mut self, left: u8, right: u8) -> u8 {
        let carry = (self.cpu.flags & CpuState::FLAG_CF != 0) as u8;
        let result = left.wrapping_add(right).wrapping_add(carry);
        self.flags8(result);
        self.set_flag(
            CpuState::FLAG_CF,
            (left as u16 + right as u16 + carry as u16) > u8::MAX as u16,
        );
        self.set_flag(
            CpuState::FLAG_AF,
            (left & 0xf) + (right & 0xf) + carry > 0xf,
        );
        self.set_flag(
            CpuState::FLAG_OF,
            (!(left ^ right) & (left ^ result) & 0x80) != 0,
        );
        result
    }
    fn adc16(&mut self, left: u16, right: u16) -> u16 {
        let carry = (self.cpu.flags & CpuState::FLAG_CF != 0) as u16;
        let result = left.wrapping_add(right).wrapping_add(carry);
        self.flags16(result);
        self.set_flag(
            CpuState::FLAG_CF,
            (left as u32 + right as u32 + carry as u32) > u16::MAX as u32,
        );
        self.set_flag(
            CpuState::FLAG_AF,
            (left & 0xf) + (right & 0xf) + carry > 0xf,
        );
        self.set_flag(
            CpuState::FLAG_OF,
            (!(left ^ right) & (left ^ result) & 0x8000) != 0,
        );
        result
    }
    fn sub8(&mut self, left: u8, right: u8) -> u8 {
        let (value, borrow) = left.overflowing_sub(right);
        self.flags8(value);
        self.set_flag(CpuState::FLAG_CF, borrow);
        self.set_flag(CpuState::FLAG_AF, (left & 0xf) < (right & 0xf));
        self.set_flag(
            CpuState::FLAG_OF,
            ((left ^ right) & (left ^ value) & 0x80) != 0,
        );
        value
    }
    fn sub16(&mut self, left: u16, right: u16) -> u16 {
        let (value, borrow) = left.overflowing_sub(right);
        self.flags16(value);
        self.set_flag(CpuState::FLAG_CF, borrow);
        self.set_flag(CpuState::FLAG_AF, (left & 0xf) < (right & 0xf));
        self.set_flag(
            CpuState::FLAG_OF,
            ((left ^ right) & (left ^ value) & 0x8000) != 0,
        );
        value
    }
    fn sbb8(&mut self, left: u8, right: u8) -> u8 {
        let borrow = (self.cpu.flags & CpuState::FLAG_CF != 0) as u8;
        let result = left.wrapping_sub(right).wrapping_sub(borrow);
        self.flags8(result);
        self.set_flag(
            CpuState::FLAG_CF,
            (left as u16) < right as u16 + borrow as u16,
        );
        self.set_flag(CpuState::FLAG_AF, (left & 0xf) < (right & 0xf) + borrow);
        self.set_flag(
            CpuState::FLAG_OF,
            ((left ^ right) & (left ^ result) & 0x80) != 0,
        );
        result
    }
    fn sbb16(&mut self, left: u16, right: u16) -> u16 {
        let borrow = (self.cpu.flags & CpuState::FLAG_CF != 0) as u16;
        let result = left.wrapping_sub(right).wrapping_sub(borrow);
        self.flags16(result);
        self.set_flag(
            CpuState::FLAG_CF,
            (left as u32) < right as u32 + borrow as u32,
        );
        self.set_flag(CpuState::FLAG_AF, (left & 0xf) < (right & 0xf) + borrow);
        self.set_flag(
            CpuState::FLAG_OF,
            ((left ^ right) & (left ^ result) & 0x8000) != 0,
        );
        result
    }
    fn logic8(&mut self, value: u8) -> u8 {
        self.flags8(value);
        self.cpu.flags &= !(CpuState::FLAG_CF | CpuState::FLAG_OF | CpuState::FLAG_AF);
        value
    }
    fn logic16(&mut self, value: u16) -> u16 {
        self.flags16(value);
        self.cpu.flags &= !(CpuState::FLAG_CF | CpuState::FLAG_OF | CpuState::FLAG_AF);
        value
    }
    fn inc16(&mut self, value: u16) -> u16 {
        let carry = self.cpu.flags & CpuState::FLAG_CF;
        let result = self.add16(value, 1);
        self.cpu.flags = (self.cpu.flags & !CpuState::FLAG_CF) | carry;
        result
    }
    fn inc8(&mut self, value: u8) -> u8 {
        let carry = self.cpu.flags & CpuState::FLAG_CF;
        let result = self.add8(value, 1);
        self.cpu.flags = (self.cpu.flags & !CpuState::FLAG_CF) | carry;
        result
    }
    fn dec16(&mut self, value: u16) -> u16 {
        let carry = self.cpu.flags & CpuState::FLAG_CF;
        let result = self.sub16(value, 1);
        self.cpu.flags = (self.cpu.flags & !CpuState::FLAG_CF) | carry;
        result
    }
    fn dec8(&mut self, value: u8) -> u8 {
        let carry = self.cpu.flags & CpuState::FLAG_CF;
        let result = self.sub8(value, 1);
        self.cpu.flags = (self.cpu.flags & !CpuState::FLAG_CF) | carry;
        result
    }

    fn shift8(&mut self, kind: u8, value: u8, count: u8, ip: u16) -> Result<u8, Trap> {
        self.shift_width(kind, value as u32, count, 8, ip)
            .map(|v| v as u8)
    }
    fn shift16(&mut self, kind: u8, value: u16, count: u8, ip: u16) -> Result<u16, Trap> {
        self.shift_width(kind, value as u32, count, 16, ip)
            .map(|v| v as u16)
    }
    fn shift_width(
        &mut self,
        kind: u8,
        mut value: u32,
        count: u8,
        bits: u32,
        ip: u16,
    ) -> Result<u32, Trap> {
        if kind > 7 || kind == 6 {
            return Err(self.bad_opcode(ip));
        }
        if count == 0 {
            return Ok(value);
        }
        let mask = u32::MAX >> (32 - bits);
        let sign = 1u32 << (bits - 1);
        let original = value;
        for _ in 0..count {
            let low = value & 1;
            let high = value & sign != 0;
            let carry_in = (self.cpu.flags & CpuState::FLAG_CF != 0) as u32;
            let carry = match kind {
                0 => {
                    value = ((value << 1) | high as u32) & mask;
                    high
                }
                1 => {
                    value = (value >> 1) | (low << (bits - 1));
                    low != 0
                }
                2 => {
                    value = ((value << 1) | carry_in) & mask;
                    high
                }
                3 => {
                    value = (value >> 1) | (carry_in << (bits - 1));
                    low != 0
                }
                4 => {
                    value = (value << 1) & mask;
                    high
                }
                5 => {
                    value >>= 1;
                    low != 0
                }
                7 => {
                    value = (value >> 1) | (value & sign);
                    low != 0
                }
                _ => unreachable!(),
            };
            self.set_flag(CpuState::FLAG_CF, carry);
        }
        if kind >= 4 {
            self.set_flag(CpuState::FLAG_ZF, value == 0);
            self.set_flag(CpuState::FLAG_SF, value & sign != 0);
            self.set_flag(CpuState::FLAG_PF, Self::parity(value as u8));
        }
        if count == 1 {
            let overflow = match kind {
                0 | 2 | 4 => (value & sign != 0) ^ (self.cpu.flags & CpuState::FLAG_CF != 0),
                1 | 3 => ((value >> (bits - 1)) ^ (value >> (bits - 2))) & 1 != 0,
                5 => original & sign != 0,
                _ => false,
            };
            self.set_flag(CpuState::FLAG_OF, overflow);
        }
        Ok(value)
    }
    fn condition(&self, code: u8) -> bool {
        let f = self.cpu.flags;
        let cf = f & CpuState::FLAG_CF != 0;
        let zf = f & CpuState::FLAG_ZF != 0;
        let sf = f & CpuState::FLAG_SF != 0;
        let of = f & CpuState::FLAG_OF != 0;
        match code {
            2 => cf,
            3 => !cf,
            4 => zf,
            5 => !zf,
            6 => cf || zf,
            7 => !cf && !zf,
            8 => sf,
            9 => !sf,
            12 => sf != of,
            13 => sf == of,
            14 => zf || sf != of,
            15 => !zf && sf == of,
            _ => false,
        }
    }

    pub(crate) fn exit(&mut self, code: u8) -> Result<(), Trap> {
        self.active_process.searches.clear();
        self.active_process.search_index = 0;
        if self.parent_process.is_some() {
            self.restore_parent(ChildResult {
                code,
                termination: TerminationType::Normal,
            });
        } else {
            self.release_process_vectors(&self.active_process.clone());
            self.arena.free_owner(self.active_process.psp_segment);
            self.arena.sync_mcbs(&mut self.memory);
            self.state = GuestState::Exited { code };
        }
        Ok(())
    }
    pub(crate) fn has_dos_byte(&self) -> bool {
        self.dos_scan_tail.is_some() || !self.keyboard.is_empty()
    }

    /// Read the DOS character stream without placing scan tails in the BIOS
    /// event queue. Only original characters are eligible for console echo.
    pub(crate) fn pop_dos_byte(&mut self) -> Option<(u8, bool)> {
        if let Some(scan) = self.dos_scan_tail.take() {
            return Some((scan, false));
        }
        let key = self.pop_key()?;
        let [ascii, scan] = key.bios_word(false).to_le_bytes();
        if ascii == 0 && scan != 0 {
            self.dos_scan_tail = Some(scan);
            return Some((0, false));
        }
        Some((ascii, true))
    }
    pub(crate) fn peek_key(&self) -> Option<BiosKey> {
        self.keyboard.front().copied()
    }
    pub(crate) fn pop_key(&mut self) -> Option<BiosKey> {
        self.keyboard.pop_front()
    }
    pub(crate) fn clear_keys(&mut self) {
        self.keyboard.clear();
        self.dos_scan_tail = None;
    }
    pub(crate) fn shift_flags(&self) -> u8 {
        (self.modifier_sides & 0x03)
            | ((self.ctrl as u8) << 2)
            | ((self.alt as u8) << 3)
            | ((self.num_lock as u8) << 5)
    }
    pub(crate) fn enhanced_shift_flags(&self) -> u16 {
        u16::from_le_bytes([self.shift_flags(), self.modifier_sides >> 2])
    }
    pub(crate) fn wait_for_key(&mut self, mode: InputMode) -> Result<(), Trap> {
        self.pending_input = Some(mode);
        self.complete_pending_input();
        Ok(())
    }
    pub(crate) fn wait_for_console_read(&mut self, segment: u16, offset: u16) -> Result<(), Trap> {
        self.pending_input = Some(InputMode::ConsoleRead { segment, offset });
        self.complete_pending_input();
        Ok(())
    }
    fn complete_pending_input(&mut self) {
        while let Some(mode) = self.pending_input {
            self.complete_input_step(mode);
            if !matches!(self.state, GuestState::WaitingForInput) || self.keyboard.is_empty() {
                break;
            }
        }
    }

    fn complete_input_step(&mut self, mode: InputMode) {
        if let InputMode::Dos { echo } = mode {
            let Some((byte, character)) = self.pop_dos_byte() else {
                self.state = GuestState::WaitingForInput;
                return;
            };
            self.cpu.set_al(byte);
            if echo && character {
                self.teletype(byte, DEFAULT_ATTRIBUTE);
            }
            self.pending_input = None;
            self.state = GuestState::Running;
            return;
        }
        let Some(key) = self.pop_key() else {
            self.state = GuestState::WaitingForInput;
            return;
        };
        let ascii = key.console_byte();
        match mode {
            InputMode::Bios { enhanced } => {
                self.cpu.ax = key.bios_word(enhanced);
                self.trace_keyboard_return(if enhanced { 0x10 } else { 0x00 });
                self.pending_input = None;
                self.state = GuestState::Running;
            }
            InputMode::Dos { .. } => unreachable!(),
            InputMode::ConsoleRead { segment, offset } => {
                self.memory.write_u8(segment, offset, ascii);
                if ascii == b'\r' {
                    self.teletype(b'\r', DEFAULT_ATTRIBUTE);
                    // DOS console handles expose Enter as the two-byte CR/LF
                    // sequence.  A one-byte read returns CR first, then the
                    // following read must receive LF without another host
                    // keypress.  Pascal's ReadLn relies on that second byte.
                    self.keyboard.push_front(BiosKey {
                        ascii: b'\n',
                        scan_code: 0,
                    });
                } else {
                    self.teletype(ascii, DEFAULT_ATTRIBUTE);
                }
                self.cpu.ax = 1;
                self.cpu.flags &= !CpuState::FLAG_CF;
                self.pending_input = None;
                self.state = GuestState::Running;
            }
            InputMode::Line {
                segment,
                offset,
                maximum,
                mut length,
            } => {
                if ascii == b'\r' {
                    self.memory
                        .write_u8(segment, offset.wrapping_add(1), length);
                    self.memory.write_u8(
                        segment,
                        offset.wrapping_add(2).wrapping_add(length as u16),
                        b'\r',
                    );
                    self.teletype(b'\r', DEFAULT_ATTRIBUTE);
                    self.teletype(b'\n', DEFAULT_ATTRIBUTE);
                    self.pending_input = None;
                    self.state = GuestState::Running;
                } else if ascii == 0x08 {
                    if length > 0 {
                        length -= 1;
                        self.memory
                            .write_u8(segment, offset.wrapping_add(1), length);
                        self.teletype(0x08, DEFAULT_ATTRIBUTE);
                    }
                    self.pending_input = Some(InputMode::Line {
                        segment,
                        offset,
                        maximum,
                        length,
                    });
                    self.state = GuestState::WaitingForInput;
                } else if ascii >= 0x20 && length < maximum {
                    self.memory.write_u8(
                        segment,
                        offset.wrapping_add(2).wrapping_add(length as u16),
                        ascii,
                    );
                    length += 1;
                    self.memory
                        .write_u8(segment, offset.wrapping_add(1), length);
                    self.teletype(ascii, DEFAULT_ATTRIBUTE);
                    self.pending_input = Some(InputMode::Line {
                        segment,
                        offset,
                        maximum,
                        length,
                    });
                    self.state = GuestState::WaitingForInput;
                } else {
                    self.pending_input = Some(InputMode::Line {
                        segment,
                        offset,
                        maximum,
                        length,
                    });
                    self.state = GuestState::WaitingForInput;
                }
            }
        }
    }

    pub(crate) fn reset_video(&mut self) {
        self.set_video_mode(GuestVideoMode::Text80x25Color);
    }

    pub(crate) fn set_video_mode(&mut self, mode: GuestVideoMode) {
        self.set_interrupt_vector(
            0x43,
            crate::font::ROM_SEGMENT,
            match mode {
                GuestVideoMode::Text80x25Color => 0x800,
                GuestVideoMode::Vga320x200x256 => 0,
            },
        );
        // A VGA BIOS mode set reprograms the Attribute Controller mode-control
        // bit to its documented default: text attribute bit 7 blinks.
        self.set_text_attribute_mode(TextAttributeMode::Blink);
        self.video_mode = mode;
        self.video_mode_generation = self.video_mode_generation.wrapping_add(1);
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.memory
            .write_u8(BDA_SEGMENT, BDA_VIDEO_MODE, mode.bios_mode());
        self.memory.write_u8(BDA_SEGMENT, BDA_ACTIVE_PAGE, 0);
        self.memory.write_u16(BDA_SEGMENT, 0x4e, 0); // Page offset
        self.memory.write_u16(BDA_SEGMENT, 0x63, 0x3d4); // Color CRTC
        self.memory
            .write_u8(BDA_SEGMENT, 0x84, mode.character_rows() - 1);
        self.memory
            .write_u16(BDA_SEGMENT, 0x85, mode.character_height());
        self.memory.write_u8(
            BDA_SEGMENT,
            0x88,
            (crate::video::EGA_FEATURE_BITS << 4) | crate::video::EGA_SWITCH_SETTINGS,
        );
        for page in 0..8 {
            self.memory
                .write_u16(BDA_SEGMENT, BDA_CURSOR_POSITIONS + page * 2, 0);
        }
        match mode {
            GuestVideoMode::Text80x25Color => {
                self.mouse.leave_graphics_mode();
                TextModeSurface::clear(&mut self.memory, DEFAULT_ATTRIBUTE);
                self.cursor_shape = 0x0607;
                self.memory
                    .write_u16(BDA_SEGMENT, BDA_COLUMNS, TEXT_COLUMNS as u16);
                self.memory.write_u16(BDA_SEGMENT, BDA_PAGE_SIZE, 0x1000);
            }
            GuestVideoMode::Vga320x200x256 => {
                self.io.reset_mode13();
                self.memory.clear_graphics_framebuffer();
                self.cursor_shape = 0x2000;
                // Conventional VGA BIOS mode-13h BDA values: 40 character
                // columns and one 0xFA00-byte visible graphics page.
                self.memory.write_u16(BDA_SEGMENT, BDA_COLUMNS, 40);
                self.memory
                    .write_u16(BDA_SEGMENT, BDA_PAGE_SIZE, VGA_FRAMEBUFFER_BYTES as u16);
            }
        }
        self.memory
            .write_u16(BDA_SEGMENT, BDA_CURSOR_SHAPE, self.cursor_shape);
        self.sync_cursor_bda();
    }

    /// Selects how text attribute bit 7 is interpreted. This is the virtual
    /// counterpart of the VGA Attribute Controller mode-control bit, not a
    /// hardware-port implementation. Character and attribute bytes in guest
    /// memory are never rewritten by this change.
    pub(crate) fn set_text_attribute_mode(&mut self, mode: TextAttributeMode) {
        if self.text_attribute_mode != mode {
            self.text_attribute_mode = mode;
            self.text_attribute_generation = self.text_attribute_generation.wrapping_add(1);
        }
    }

    fn sync_cursor_bda(&mut self) {
        self.memory.write_u16(
            BDA_SEGMENT,
            BDA_CURSOR_POSITIONS,
            ((self.cursor_row as u16) << 8) | self.cursor_column as u16,
        );
    }
    pub(crate) fn set_cursor(&mut self, column: usize, row: usize) {
        self.cursor_column = column.min(TEXT_COLUMNS - 1);
        self.cursor_row = row.min(TEXT_ROWS - 1);
        self.sync_cursor_bda();
    }
    pub(crate) fn set_cursor_shape(&mut self, shape: u16) {
        self.cursor_shape = shape;
        self.memory
            .write_u16(BDA_SEGMENT, BDA_CURSOR_SHAPE, self.cursor_shape);
    }

    /// Copies the documented 96-byte video BIOS data area
    /// (`0040:0049h..0040:00A8h`) out of authoritative guest memory.
    pub(crate) fn read_video_bda(&self, out: &mut [u8; BDA_VIDEO_STATE_BYTES]) -> bool {
        self.memory
            .read_slice(BDA_SEGMENT, BDA_VIDEO_STATE_OFFSET, out)
            .is_ok()
    }

    /// Restores the video BIOS data area into the live model. The whole area
    /// is validated before any state changes, so a rejected buffer leaves both
    /// the model and guest memory untouched. Only the fields this virtual VGA
    /// derives from the BDA are checked; the remaining bytes are guest-owned
    /// BDA storage and are restored verbatim by the caller.
    pub(crate) fn restore_video_bda(&mut self, bytes: &[u8; BDA_VIDEO_STATE_BYTES]) -> bool {
        let mode = match bytes[0] {
            0x03 => GuestVideoMode::Text80x25Color,
            0x13 => GuestVideoMode::Vga320x200x256,
            _ => return false,
        };
        let word = |index: usize| u16::from_le_bytes([bytes[index], bytes[index + 1]]);
        let (columns, page_size) = match mode {
            GuestVideoMode::Text80x25Color => (TEXT_COLUMNS as u16, 0x1000),
            GuestVideoMode::Vga320x200x256 => (40, VGA_FRAMEBUFFER_BYTES as u16),
        };
        // 0040:004Ah columns, 0040:004Ch page size, 0040:004Eh page offset.
        if word(0x01) != columns || word(0x03) != page_size || word(0x05) != 0 {
            return false;
        }
        // 0040:0050h..005Fh: eight page cursors. Only page zero is modeled.
        for page in 1..8 {
            if word(0x07 + page * 2) != 0 {
                return false;
            }
        }
        let cursor = word(0x07);
        let column = usize::from(cursor as u8);
        let row = usize::from((cursor >> 8) as u8);
        if column >= TEXT_COLUMNS || row >= TEXT_ROWS {
            return false;
        }
        let shape = word(0x17); // 0040:0060h
        if bytes[0x19] != 0 {
            return false; // 0040:0062h active page
        }
        if word(0x1a) != 0x3d4 {
            return false; // 0040:0063h color CRTC base
        }
        if bytes[0x3b] != mode.character_rows() - 1 || word(0x3c) != mode.character_height() {
            return false; // 0040:0084h rows-1, 0040:0085h character height
        }
        if bytes[0x3f] != (crate::video::EGA_FEATURE_BITS << 4) | crate::video::EGA_SWITCH_SETTINGS
        {
            return false; // 0040:0088h feature/switch settings
        }

        if self.video_mode != mode {
            self.video_mode = mode;
            self.video_mode_generation = self.video_mode_generation.wrapping_add(1);
            if mode == GuestVideoMode::Text80x25Color {
                self.mouse.leave_graphics_mode();
            }
        }
        self.set_cursor(column, row);
        self.set_cursor_shape(shape);
        true
    }

    pub(crate) fn put_cell(&mut self, column: usize, row: usize, character: u8, attribute: u8) {
        if column < TEXT_COLUMNS && row < TEXT_ROWS {
            let offset = ((row * TEXT_COLUMNS + column) * 2) as u16;
            self.memory.write_u8(VIDEO_SEGMENT, offset, character);
            self.memory
                .write_u8(VIDEO_SEGMENT, offset.wrapping_add(1), attribute);
        }
    }
    pub(crate) fn advance_cursor(&mut self) {
        self.cursor_column += 1;
        if self.cursor_column >= TEXT_COLUMNS {
            self.cursor_column = 0;
            self.cursor_row += 1;
        }
        if self.cursor_row >= TEXT_ROWS {
            let _ = self.scroll_up(0, 0, TEXT_ROWS - 1, TEXT_COLUMNS - 1, 1, DEFAULT_ATTRIBUTE);
            self.cursor_row = TEXT_ROWS - 1;
        }
        self.sync_cursor_bda();
    }
    pub(crate) fn teletype(&mut self, byte: u8, attribute: u8) {
        match byte {
            b'\r' => self.cursor_column = 0,
            b'\n' => {
                self.cursor_row += 1;
                if self.cursor_row >= TEXT_ROWS {
                    let _ = self.scroll_up(0, 0, TEXT_ROWS - 1, TEXT_COLUMNS - 1, 1, attribute);
                    self.cursor_row = TEXT_ROWS - 1;
                }
            }
            0x08 => {
                if self.cursor_column > 0 {
                    self.cursor_column -= 1;
                    self.put_cell(self.cursor_column, self.cursor_row, b' ', attribute);
                }
            }
            _ => {
                self.put_cell(self.cursor_column, self.cursor_row, byte, attribute);
                self.advance_cursor();
                return;
            }
        }
        self.sync_cursor_bda();
    }
    pub(crate) fn scroll_down(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        lines: usize,
        attribute: u8,
    ) -> Result<(), Trap> {
        if top > bottom || left > right || bottom >= TEXT_ROWS || right >= TEXT_COLUMNS {
            return Err(Trap::InvalidVideoRectangle);
        }
        let height = bottom - top + 1;
        let lines = if lines == 0 {
            height
        } else {
            lines.min(height)
        };
        for row in (top..=bottom).rev() {
            for column in left..=right {
                let cell = if row >= top + lines {
                    self.cell(column, row - lines)
                } else {
                    TextCell {
                        character: b' ',
                        attribute,
                    }
                };
                self.put_cell(column, row, cell.character, cell.attribute);
            }
        }
        Ok(())
    }
    pub(crate) fn scroll_up(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        lines: usize,
        attribute: u8,
    ) -> Result<(), Trap> {
        if top > bottom || left > right || bottom >= TEXT_ROWS || right >= TEXT_COLUMNS {
            return Err(Trap::InvalidVideoRectangle);
        }
        let height = bottom - top + 1;
        let lines = if lines == 0 {
            height
        } else {
            lines.min(height)
        };
        for row in top..=bottom {
            for column in left..=right {
                let source = row + lines;
                let cell = if source <= bottom {
                    self.cell(column, source)
                } else {
                    TextCell {
                        character: b' ',
                        attribute,
                    }
                };
                self.put_cell(column, row, cell.character, cell.attribute);
            }
        }
        Ok(())
    }
}

fn default_environment(app_id: &[u8], executable_path: &[u8]) -> Vec<u8> {
    let mut entries = Vec::new();
    for entry in [
        b"PATH=C:\\;C:\\TESTS".as_slice(),
        b"TEMP=T:\\".as_slice(),
        b"TMP=T:\\".as_slice(),
    ] {
        entries.extend_from_slice(entry);
        entries.push(0);
    }
    entries.extend_from_slice(b"COMSPEC=");
    entries.extend_from_slice(executable_path);
    entries.push(0);
    entries.extend_from_slice(b"SHELL=");
    entries.extend_from_slice(executable_path);
    entries.push(0);
    entries.extend_from_slice(b"APPID=");
    entries.extend_from_slice(&app_id[..app_id.len().min(80)]);
    entries.push(0);
    entries.push(0);
    entries
}

fn environment_contents(entries: &[u8], executable_path: &[u8]) -> Result<Vec<u8>, LoaderError> {
    let mut contents = entries.to_vec();
    if !contents.ends_with(&[0, 0]) {
        if !contents.ends_with(&[0]) {
            contents.push(0);
        }
        contents.push(0);
    }
    contents.extend_from_slice(&1u16.to_le_bytes());
    contents.extend_from_slice(executable_path);
    contents.push(0);
    if contents.len() > ENVIRONMENT_MAX_BYTES {
        return Err(LoaderError::InsufficientMemory {
            requested: u16::MAX,
            largest: 0,
        });
    }
    contents.resize(ENVIRONMENT_MAX_BYTES, 0);
    Ok(contents)
}

fn environment_paragraphs() -> Result<u16, LoaderError> {
    u16::try_from(ENVIRONMENT_MAX_BYTES.div_ceil(16)).map_err(|_| LoaderError::InsufficientMemory {
        requested: u16::MAX,
        largest: 0,
    })
}

fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u8, day as u8)
}

#[cfg(test)]
mod tests {
    use super::{translate_key_press, BiosKey, GuestState, HostKeyEvent, Runtime};
    use crate::{
        CpuState, DosDrive, GuestVideoMode, IoOperation, IoWidth, MouseViewport, Rgb8,
        TerminationType, TextAttributeMode, Trap, CHRONOS_INTERACTIVE_COM, DEFAULT_ATTRIBUTE,
        DEFAULT_VGA_PALETTE, HELLO_CHRONOS_COM, PSP_SEGMENT, VGA_FRAMEBUFFER_BYTES, VGA_HEIGHT,
    };

    fn runtime_for(bytes: &[u8]) -> Runtime {
        Runtime::from_com(bytes).unwrap()
    }

    fn runtime386(bytes: &[u8]) -> Runtime {
        let mut r = runtime_for(bytes);
        r.set_cpu_profile(crate::CpuProfile::I80386RealMode);
        r
    }

    #[test]
    fn multiplication_full_product_boundary_tables() {
        // Independently specified full products and signed/unsigned overflow.
        let cases: &[(u32, u32, u32, bool, u64, bool)] = &[
            (8, 6, 7, false, 42, false),
            (8, 0xff, 2, true, 0xffff_ffff_ffff_fffe, false),
            (8, 0x80, 0xff, true, 128, true),
            (8, 0x80, 1, true, 0xffff_ffff_ffff_ff80, false),
            (8, 0xff, 0xff, false, 0xfe01, true),
            (16, 0x7fff, 1, true, 0x7fff, false),
            (16, 0x8000, 0xffff, true, 0x8000, true),
            (16, 0xffff, 0xffff, true, 1, false),
            (16, 0xffff, 0xffff, false, 0xfffe0001, true),
            (32, 95, 1024, true, 0x17c00, false),
            (32, 0xffff_fffe, 3, true, 0xffff_ffff_ffff_fffa, false),
            (32, 0xffff_fffe, 0xffff_fffd, true, 6, false),
            (32, 0x7fff_ffff, 1, true, 0x7fff_ffff, false),
            (32, 0x7fff_ffff, 2, true, 0xffff_fffe, true),
            (32, 0x8000_0000, 1, true, 0xffff_ffff_8000_0000, false),
            (32, 0x8000_0000, 0xffff_ffff, true, 0x8000_0000, true),
            (
                32,
                0xffff_ffff,
                0xffff_ffff,
                false,
                0xffff_fffe_0000_0001,
                true,
            ),
            (32, 0xffff_ffff, 0, false, 0, false),
            (32, 0, 0x8000_0000, true, 0, false),
        ];
        for &(bits, left, right, signed, expected, overflow) in cases {
            let mut code = Vec::new();
            if bits == 32 {
                code.push(0x66);
            }
            code.extend_from_slice(&[
                if bits == 8 { 0xf6 } else { 0xf7 },
                if signed { 0xe9 } else { 0xe1 },
            ]);
            let mut r = runtime386(&code);
            for index in 0..8 {
                r.cpu.set_reg32(index, 0xabcd1234 + index as u32);
            }
            r.cpu.set_reg32(0, left);
            r.cpu.set_reg32(1, right);
            r.cpu.flags = 0xf7d6;
            let before = r.cpu;
            r.step();
            assert!(!matches!(r.state(), GuestState::Trapped(_)));
            assert_eq!(r.cpu.ip, 0x100 + code.len() as u16);
            let actual = match bits {
                8 => r.cpu.ax as u64,
                16 => ((r.cpu.dx as u64) << 16) | r.cpu.ax as u64,
                _ => ((r.cpu.reg32(2) as u64) << 32) | r.cpu.reg32(0) as u64,
            };
            let mask = if bits == 32 {
                u64::MAX
            } else {
                (1u64 << (bits * 2)) - 1
            };
            assert_eq!(actual, expected & mask, "{bits} {left:x} {right:x}");
            assert_eq!(r.cpu.flags & 0x801, if overflow { 0x801 } else { 0 });
            assert_eq!(r.cpu.flags & !0x801, before.flags & !0x801);
            for index in [1, 3, 4, 5, 6, 7] {
                assert_eq!(r.cpu.reg32(index), before.reg32(index));
            }
            if bits < 32 {
                assert_eq!(r.cpu.gpr_upper, before.gpr_upper);
            }
            if bits == 8 {
                assert_eq!(r.cpu.reg32(2), before.reg32(2));
            }
        }
    }

    #[test]
    fn idiv_register_results_and_faults() {
        for word in [false, true] {
            let bits = if word { 16 } else { 32 };
            let min = -(1i64 << (bits - 1));
            let max = -min - 1;
            let mask = ((1u64 << bits) - 1) as u32;
            let code = if word {
                &[0xf7, 0xfb][..]
            } else {
                &[0x66, 0xf7, 0xfb][..]
            };
            for (dividend, divisor, result) in [
                (23, 4, Some((5, 3))),
                (-23, 4, Some((-5, -3))),
                (23, -4, Some((-5, 3))),
                (-23, -4, Some((5, -3))),
                (0, -1, Some((0, 0))),
                (min, 1, Some((min, 0))),
                (max, 1, Some((max, 0))),
                (1i64 << bits, 4, Some((1i64 << (bits - 2), 0))),
                (5, 0, None),
                (max + 1, 1, None),
                (min - 1, 1, None),
                (min, -1, None),
                (if word { i64::from(i32::MIN) } else { i64::MIN }, -1, None),
            ] {
                let mut r = runtime386(code);
                for index in 0..8 {
                    r.cpu.set_reg32(index, 0x1234_5678 + u32::from(index));
                }
                let put = |cpu: &mut CpuState, index, value: i64| {
                    cpu.set_reg32(index, (cpu.reg32(index) & !mask) | (value as u32 & mask));
                };
                put(&mut r.cpu, 0, dividend);
                put(&mut r.cpu, 2, dividend >> bits);
                put(&mut r.cpu, 3, divisor);
                r.cpu.flags = 0x0ed7;
                let mut expected = r.cpu;
                let start = expected.ip;
                r.step();
                expected.ip += code.len() as u16;
                if let Some((quotient, remainder)) = result {
                    put(&mut expected, 0, quotient);
                    put(&mut expected, 2, remainder);
                    assert_eq!(r.state(), &GuestState::Running);
                    assert_eq!(r.instructions_retired(), 1);
                } else {
                    assert!(
                        matches!(r.state(), GuestState::Trapped(Trap::DivideError { cs, ip })
                        if *cs == expected.cs && *ip == start)
                    );
                    assert_eq!(r.instructions_retired(), 0);
                }
                assert_eq!(r.cpu, expected);
            }
        }
    }

    #[test]
    fn idiv_memory_prefixes_and_segment_limit() {
        // ES:[0200] as a dword, then DS:[0200] as a word.
        let mut r = runtime386(&[0x26, 0x66, 0xf7, 0x3e, 0, 2, 0xf7, 0x3e, 0, 2]);
        r.cpu.es = 0x3000;
        r.cpu.set_reg32(0, 0x0001_0005);
        r.memory.write_u16(r.cpu.es, 0x200, 0);
        r.memory.write_u16(r.cpu.es, 0x202, 1);
        r.memory.write_u16(r.cpu.ds, 0x200, 2);
        r.memory.write_u16(r.cpu.ds, 0x202, 1);
        r.step();
        assert_eq!((r.cpu.reg32(0), r.cpu.reg32(2), r.cpu.ip), (1, 5, 0x106));
        r.cpu.ax = 13;
        r.cpu.dx = 0;
        r.step();
        assert_eq!((r.cpu.reg32(0), r.cpu.reg32(2), r.cpu.ip), (6, 1, 0x10a));
        assert_eq!(r.instructions_retired(), 2);

        let mut invalid = runtime386(&[0x66, 0xf7, 0x3e, 0xfd, 0xff]);
        let mut expected = invalid.cpu;
        invalid.step();
        assert!(matches!(
            invalid.state(),
            GuestState::Trapped(Trap::SegmentLimit { segment, offset: 0xfffd, width: 4 })
                if *segment == expected.ds
        ));
        assert_eq!(invalid.instructions_retired(), 0);
        expected.ip += 5;
        assert_eq!(invalid.cpu, expected);
    }

    #[test]
    fn multiplication_memory_widths_and_segment_boundary() {
        for (code, bits) in [
            (&[0xf6, 0x26, 0xfc, 0xff][..], 8),
            (&[0xf7, 0x26, 0xfc, 0xff][..], 16),
            (&[0x66, 0xf7, 0x26, 0xfc, 0xff][..], 32),
        ] {
            let mut r = runtime386(code);
            r.cpu.set_reg32(0, 3);
            r.memory.write_u16(r.cpu.ds, 0xfffc, 7);
            r.memory.write_u16(r.cpu.ds, 0xfffe, 0);
            r.step();
            assert_eq!(r.cpu.ax, 21);
            assert_eq!(r.cpu.flags & 0x801, 0);
            if bits == 32 {
                assert_eq!(r.cpu.reg32(2), 0);
            }
        }
        for code in [
            &[0x66, 0x69, 0xc1, 0xfe, 0xff, 0xff, 0xff][..],
            &[0x66, 0x6b, 0xc1, 0xfe][..],
        ] {
            let mut r = runtime386(code);
            r.cpu.set_reg32(1, 0x8000_0000);
            r.step();
            assert_eq!(r.cpu.reg32(0), 0);
            assert_eq!(r.cpu.flags & 0x801, 0x801);
            assert_eq!(r.cpu.reg32(1), 0x8000_0000);
        }
        let mut r = runtime_for(&[0x6b, 0xc1, 0xfe]);
        r.set_cpu_profile(crate::CpuProfile::I80186);
        r.cpu.cx = 3;
        r.step();
        assert_eq!(r.cpu.ax, 0xfffa);
    }

    #[test]
    fn multiplication_memory_immediates_profiles_and_prefixes() {
        for dword in [false, true] {
            for opcode in [0x69, 0x6b, 0xaf] {
                let mut code = vec![0x26];
                if dword {
                    code.push(0x66);
                }
                if opcode == 0xaf {
                    code.push(0x0f);
                }
                code.extend_from_slice(&[opcode, 0x0e, 0x00, 0x02]); // ES:[0200] -> (E)CX
                if opcode == 0x6b {
                    code.push(0xfe);
                }
                if opcode == 0x69 {
                    code.extend_from_slice(&[0xfe, 0xff]);
                    if dword {
                        code.extend_from_slice(&[0xff, 0xff]);
                    }
                }
                code.extend_from_slice(&[0xb9, 0x34, 0x12]);
                let mut r = runtime386(&code);
                r.cpu.es = 0x3000;
                r.memory.write_u16(0x3000, 0x200, 3);
                r.memory.write_u16(0x3000, 0x202, 0);
                r.cpu.set_reg32(1, 0xffff_fffe);
                let before = r.cpu;
                r.step();
                assert_eq!(r.cpu.reg16(1), 0xfffa);
                if dword {
                    assert_eq!(r.cpu.reg32(1), 0xffff_fffa);
                }
                assert_eq!(r.cpu.flags & 0x801, 0);
                assert_eq!(r.cpu.reg32(0), before.reg32(0));
                assert_eq!(r.cpu.reg32(2), before.reg32(2));
                assert_eq!(r.cpu.ip, 0x100 + code.len() as u16 - 3);
                r.step(); // operand prefix expires
                assert_eq!(r.cpu.cx, 0x1234);
            }
        }
        let mut memory = runtime386(&[0x26, 0x66, 0xf7, 0x2e, 0x00, 0x02]);
        memory.cpu.es = 0x3000;
        memory.memory.write_u16(0x3000, 0x200, 0xfffe);
        memory.memory.write_u16(0x3000, 0x202, 0xffff);
        memory.cpu.set_reg32(0, 3);
        memory.step();
        assert_eq!(memory.cpu.reg32(0), 0xffff_fffa);
        assert_eq!(memory.cpu.reg32(2), 0xffff_ffff);
        let mut invalid = runtime386(&[0x66, 0xf7, 0x2e, 0xfd, 0xff]);
        let before = invalid.cpu;
        invalid.step();
        assert!(matches!(
            invalid.state(),
            GuestState::Trapped(Trap::SegmentLimit { .. })
        ));
        assert_eq!(invalid.cpu.ax, before.ax);
        assert_eq!(invalid.cpu.dx, before.dx);
        assert_eq!(invalid.cpu.flags, before.flags);
        for code in [
            &[0x69, 0xc1, 1, 0][..],
            &[0x6b, 0xc1, 1][..],
            &[0x66, 0xf7, 0xe9][..],
            &[0x0f, 0xaf, 0xc1][..],
        ] {
            let mut r = runtime_for(code);
            r.step();
            assert!(matches!(r.state(), GuestState::Trapped(_)));
        }
        let mut overflow = runtime386(&[0x66, 0x0f, 0xaf, 0xc1]);
        overflow.cpu.set_reg32(0, 0x8000_0000);
        overflow.cpu.set_reg32(1, 0xffff_ffff);
        overflow.step();
        assert_eq!(overflow.cpu.reg32(0), 0x8000_0000);
        assert_eq!(overflow.cpu.flags & 0x801, 0x801);
    }

    #[test]
    fn dword_aliases_prefix_lifetime_and_dn_shift() {
        let mut r = runtime386(&[
            0x66, 0xb8, 0x78, 0x56, 0x34, 0x12, 0xb8, 0x5f, 0, 0xb4, 0xab, 0xb0, 0xcd, 0x66, 0xc1,
            0xe0, 0x10, 0x90,
        ]);
        r.step();
        assert_eq!(r.cpu.reg32(0), 0x12345678);
        r.step();
        assert_eq!(r.cpu.reg32(0), 0x1234005f);
        r.step();
        r.step();
        assert_eq!(r.cpu.reg32(0), 0x1234abcd);
        r.step();
        assert_eq!(r.cpu.reg32(0), 0xabcd0000);
        assert_eq!(r.cpu.ip, 0x111);
        r.step();
        assert_eq!(r.cpu.ip, 0x112);
        for index in 0..8 {
            r.cpu.set_reg32(index, 0xfeed1234);
            r.cpu.set_reg16(index, 0xabcd);
            assert_eq!(r.cpu.reg32(index), 0xfeedabcd);
        }
    }

    #[test]
    fn cwd_sign_extends_ax_without_changing_flags_or_other_state() {
        for (ax, expected_dx) in [
            (0x0001, 0x0000),
            (0x7fff, 0x0000),
            (0x8000, 0xffff),
            (0xffff, 0xffff),
            (0x0000, 0x0000),
        ] {
            let mut r = runtime386(&[0x99]);
            r.cpu.ax = ax;
            r.cpu.dx = 0x1234;
            r.cpu.flags = 0xfed7;
            let before = r.cpu;
            r.step();
            assert_eq!(r.cpu.dx, expected_dx, "AX={ax:04x}");
            let mut expected = before;
            expected.dx = expected_dx;
            expected.ip = 0x101;
            assert_eq!(r.cpu, expected);
        }
    }

    #[test]
    fn cdq_sign_extends_eax_and_operand_override_is_consumed_once() {
        for (eax, expected_edx) in [
            (1, 0),
            (0x7fff_ffff, 0),
            (0x8000_0000, u32::MAX),
            (u32::MAX, u32::MAX),
            (0, 0),
        ] {
            let mut r = runtime386(&[0x66, 0x99, 0x99]);
            r.cpu.set_reg32(0, eax);
            r.cpu.set_reg32(2, 0x1234_5678);
            r.cpu.flags = 0xa5d7;
            let before = r.cpu;
            r.step();
            assert_eq!(r.cpu.reg32(2), expected_edx, "EAX={eax:08x}");
            assert_eq!(r.cpu.reg32(0), eax);
            assert_eq!(r.cpu.flags, 0xa5d7);
            assert_eq!(r.cpu.ip, 0x102);
            assert_eq!(r.instructions_retired(), 1);
            let mut expected = before;
            expected.set_reg32(2, expected_edx);
            expected.ip = 0x102;
            assert_eq!(r.cpu, expected);
            // The following unprefixed 99 is CWD, proving 66 does not leak.
            let expected_dx = if eax as u16 & 0x8000 != 0 { 0xffff } else { 0 };
            r.step();
            assert_eq!(r.cpu.dx, expected_dx, "subsequent CWD after EAX={eax:08x}");
            assert_eq!(r.cpu.reg32(0), eax);
            assert_eq!(r.cpu.flags, 0xa5d7);
            assert_eq!(r.cpu.ip, 0x103);
            assert_eq!(r.instructions_retired(), 2);
        }
    }

    #[test]
    fn aam_honors_its_immediate_radix_and_byte_width() {
        for (al, base, quotient, remainder) in [
            (0, 10, 0, 0),
            (9, 10, 0, 9),
            (10, 10, 1, 0),
            (99, 10, 9, 9),
            (17, 3, 5, 2),
            (255, 16, 15, 15),
            (255, 254, 1, 1),
            (128, 255, 0, 128),
            (255, 255, 1, 0),
        ] {
            // 66 must not widen AAM; the following AAM proves both immediate
            // operands were consumed and that the override did not leak.
            let mut r = runtime386(&[0x66, 0xd4, base, 0xd4, 2]);
            for index in 0..8 {
                r.cpu.set_reg32(index, 0xa5a5_0000 | u32::from(index));
            }
            r.cpu.set_al(al);
            r.cpu.set_ah(0xcc);
            r.cpu.flags = 0xfbd7;
            if remainder & 0x80 != 0 {
                r.cpu.flags &= !CpuState::FLAG_SF;
            }
            let before = r.cpu;
            r.step();
            assert_eq!((r.cpu.ah(), r.cpu.al()), (quotient, remainder));
            assert_eq!(r.cpu.ip, 0x103);
            assert_eq!(r.instructions_retired(), 1);
            assert_eq!(r.cpu.reg32(0) & !0xffff, before.reg32(0) & !0xffff);
            for index in 1..8 {
                assert_eq!(r.cpu.reg32(index), before.reg32(index));
            }
            // AAM defines PF, ZF, and SF from AL; its remaining arithmetic
            // flags are undefined and retain the emulator's prior values.
            assert_eq!(
                r.cpu.flags & !(CpuState::FLAG_PF | CpuState::FLAG_ZF | CpuState::FLAG_SF),
                before.flags & !(CpuState::FLAG_PF | CpuState::FLAG_ZF | CpuState::FLAG_SF)
            );
            assert_eq!(r.cpu.flags & CpuState::FLAG_ZF != 0, remainder == 0);
            assert_eq!(r.cpu.flags & CpuState::FLAG_SF != 0, remainder & 0x80 != 0);
            assert_eq!(
                r.cpu.flags & CpuState::FLAG_PF != 0,
                remainder.count_ones() % 2 == 0
            );
            r.step();
            assert_eq!(r.cpu.ip, 0x105);
            assert_eq!((r.cpu.ah(), r.cpu.al()), (remainder / 2, remainder % 2));
        }
    }

    #[test]
    fn aam_base_zero_uses_divide_error_without_partial_state() {
        let mut r = runtime386(&[0xd4, 0]);
        r.cpu.set_reg32(0, 0xface_beef);
        r.cpu.flags = 0xfad7;
        let mut expected = r.cpu;
        let start = expected.ip;
        r.step();
        expected.ip += 2;
        assert!(matches!(
            r.state(),
            GuestState::Trapped(Trap::DivideError { cs, ip }) if *cs == expected.cs && *ip == start
        ));
        assert_eq!(r.cpu, expected);
        assert_eq!(r.instructions_retired(), 0);
    }

    #[test]
    fn dword_shift_matrix_flags_counts_and_memory() {
        for kind in [0u8, 1, 2, 3, 4, 5, 7] {
            for count in [0u8, 1, 16, 31, 32, 33, 255] {
                for opcode in [0xc1u8, 0xd1, 0xd3] {
                    let effective = if opcode == 0xd1 { 1 } else { count & 31 };
                    let mut expected = 0x80000001u32;
                    let mut carry = true;
                    for _ in 0..effective {
                        let low = expected & 1 != 0;
                        let high = expected >> 31 != 0;
                        expected = match kind {
                            0 => expected.rotate_left(1),
                            1 => expected.rotate_right(1),
                            2 => (expected << 1) | carry as u32,
                            3 => (expected >> 1) | ((carry as u32) << 31),
                            4 => expected << 1,
                            5 => expected >> 1,
                            _ => ((expected as i32) >> 1) as u32,
                        };
                        carry = if matches!(kind, 0 | 2 | 4) { high } else { low };
                    }
                    for memory in [false, true] {
                        let mut bytes = vec![
                            0x26,
                            0x66,
                            opcode,
                            (kind << 3) | if memory { 6 } else { 0xc0 },
                        ];
                        if memory {
                            bytes.extend_from_slice(&[1, 2]);
                        }
                        if opcode == 0xc1 {
                            bytes.push(count);
                        }
                        let mut r = runtime386(&bytes);
                        r.cpu.es = 0xffff;
                        r.cpu.cx = count as u16;
                        r.cpu.flags = 0x0ad7;
                        r.cpu.set_reg32(0, 0x80000001);
                        r.memory.write_u16(r.cpu.es, 0x201, 1);
                        r.memory.write_u16(r.cpu.es, 0x203, 0x8000);
                        r.step();
                        assert_eq!(r.instructions_retired(), 1);
                        let actual = if memory {
                            r.memory.read_u16(r.cpu.es, 0x201) as u32
                                | ((r.memory.read_u16(r.cpu.es, 0x203) as u32) << 16)
                        } else {
                            r.cpu.reg32(0)
                        };
                        assert_eq!(
                            actual, expected,
                            "kind={kind} count={count} opcode={opcode:x}"
                        );
                        assert_eq!(r.cpu.ip, 0x100 + bytes.len() as u16);
                        if effective == 0 {
                            assert_eq!(r.cpu.flags, 0x0ad7);
                        } else {
                            assert_eq!(r.cpu.flags & CpuState::FLAG_CF != 0, carry);
                            if kind >= 4 {
                                assert_eq!(r.cpu.flags & CpuState::FLAG_ZF != 0, expected == 0);
                                assert_eq!(
                                    r.cpu.flags & CpuState::FLAG_SF != 0,
                                    expected >> 31 != 0
                                );
                                assert_eq!(
                                    r.cpu.flags & CpuState::FLAG_PF != 0,
                                    (expected as u8).count_ones() % 2 == 0
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn double_shifts_use_full_source_and_mask_counts() {
        for (second, expected) in [(0xac, 0x123489ab), (0xa4, 0xcdef5678)] {
            let mut r = runtime386(&[0x66, 0x0f, second, 0xd0, 16]);
            r.cpu.set_reg32(0, 0x89abcdef);
            r.cpu.set_reg32(2, 0x56781234);
            r.step();
            assert_eq!(r.cpu.reg32(0), expected);
            assert_eq!(r.cpu.reg32(2), 0x56781234);
            assert_eq!(r.cpu.ip, 0x105);
        }
        for second in [0xa4, 0xa5, 0xac, 0xad] {
            for count in [0u8, 1, 31, 32, 33] {
                let mut r = runtime386(&[0x66, 0x0f, second, 0x16, 0xfc, 0xff, count]);
                r.cpu.cx = count as u16;
                r.cpu.set_reg32(2, 0x80000001);
                r.memory.write_u16(r.cpu.ds, 0xfffc, 0x4567);
                r.memory.write_u16(r.cpu.ds, 0xfffe, 0x8123);
                r.cpu.flags = 0x246;
                r.step();
                let n = count & 31;
                let expected = if n == 0 {
                    0x81234567
                } else if second & 8 == 0 {
                    ((0x8123456780000001u64 << n) >> 32) as u32
                } else {
                    (0x8000000181234567u64 >> n) as u32
                };
                assert_eq!(
                    r.memory.read_u16(r.cpu.ds, 0xfffc) as u32
                        | ((r.memory.read_u16(r.cpu.ds, 0xfffe) as u32) << 16),
                    expected
                );
                if n == 0 {
                    assert_eq!(r.cpu.flags, 0x246);
                }
                if n == 1 {
                    assert_eq!(
                        r.cpu.flags & CpuState::FLAG_OF != 0,
                        (expected ^ 0x81234567) >> 31 != 0
                    );
                }
            }
        }
    }

    #[test]
    fn dword_mov_unaligned_memory_and_nested_interrupt_aliases() {
        let mut r = runtime386(&[
            0x66, 0xc7, 0x06, 1, 2, 0x78, 0x56, 0x34, 0x12, 0x66, 0x8b, 0x06, 1, 2, 0x66, 0x89,
            0xc3, 0xcd, 0x60,
        ]);
        r.memory.write_u16(0, 0x60 * 4, 0x300);
        r.memory.write_u16(0, 0x60 * 4 + 2, r.cpu.cs);
        r.memory.write_u16(0, 0x61 * 4, 0x310);
        r.memory.write_u16(0, 0x61 * 4 + 2, r.cpu.cs);
        r.memory
            .write_slice(r.cpu.cs, 0x300, &[0xcd, 0x61, 0xcf])
            .unwrap();
        r.memory
            .write_slice(r.cpu.cs, 0x310, &[0xb4, 0xab, 0xcf])
            .unwrap();
        let sp = r.cpu.sp;
        r.run_slice(8);
        assert_eq!(r.cpu.reg32(0), 0x1234ab78);
        assert_eq!(r.cpu.reg32(3), 0x12345678);
        assert_eq!(r.cpu.sp, sp);
        assert_eq!(r.cpu.ip, 0x113);
    }

    #[test]
    fn staged_prefixes_and_segment_limits_fail_without_operand_writes() {
        for bytes in [
            &[0x67, 0x66, 0xc1, 0xe0, 16][..],
            &[0x66, 0x01, 0xc0],
            &[0x66, 0xc1, 0x26, 0xfd, 0xff, 1],
            &[0xf0, 0x66, 0xc1, 0xe0, 1],
        ] {
            let mut r = runtime386(bytes);
            r.cpu.set_reg32(0, 0x12345678);
            r.cpu.flags = 0x246;
            r.memory.write_u16(r.cpu.ds, 0xfffd, 0xabcd);
            r.step();
            assert!(matches!(r.state(), GuestState::Trapped(_)));
            assert_eq!(r.cpu.reg32(0), 0x12345678);
            assert_eq!(r.cpu.flags, 0x246);
            assert_eq!(r.memory.read_u16(r.cpu.ds, 0xfffd), 0xabcd);
        }
        let mut old = runtime_for(&[0x66, 0xc1, 0xe0, 16]);
        old.step();
        assert!(matches!(old.state(), GuestState::Trapped(_)));
    }

    #[test]
    fn xlat_uses_segment_override_preserves_state_and_consumes_prefixes() {
        for (prefix, segment) in [
            (0x26, 0x2222),
            (0x2e, 0x1111),
            (0x36, 0x4444),
            (0x3e, 0x3333),
        ] {
            let mut r = runtime_for(&[prefix, 0xd7, 0x90]);
            r.cpu.ds = 0x3333;
            r.cpu.es = 0x2222;
            r.cpu.cs = 0x1111;
            r.cpu.ss = 0x4444;
            r.memory
                .write_slice(r.cpu.cs, 0x100, &[prefix, 0xd7, 0xa0, 0x00, 0x02])
                .unwrap();
            r.cpu.bx = 0x1200;
            r.cpu.ax = 0xabff;
            r.cpu.flags = 0x0ed7;
            r.memory.write_u8(segment, 0x12ff, 0x5a);
            r.memory.write_u8(0x3333, 0x12ff, 0xc3);
            r.memory.write_u8(0x3333, 0x0200, 0x66);
            r.memory.write_u8(0x2222, 0x0200, 0x77);
            let bx = r.cpu.bx;
            let flags = r.cpu.flags;
            r.step();
            assert_eq!(
                r.cpu.ax,
                if prefix == 0x3e { 0xabc3 } else { 0xab5a },
                "prefix {prefix:02x}"
            );
            assert_eq!(r.cpu.bx, bx);
            assert_eq!(r.cpu.flags, flags);
            assert_eq!(r.cpu.ip, 0x102);
            assert_eq!(r.instructions_retired(), 1);
            r.step();
            assert_eq!(
                r.cpu.al(),
                0x66,
                "segment override leaked after {prefix:02x}"
            );
            assert_eq!(r.cpu.ip, 0x105);
            assert_eq!(r.instructions_retired(), 2);
        }
    }

    #[test]
    fn xlat_address_and_operand_size_prefixes_follow_independent_rules() {
        let mut r = runtime386(&[0x67, 0x26, 0x66, 0xd7]);
        r.cpu.es = 0x2222;
        r.cpu.bx = 0x1000;
        r.cpu.set_reg32(3, 0x0000_2000);
        r.cpu.set_reg32(0, 0xcafe_00ff);
        r.cpu.flags = 0x0ed7;
        r.memory.write_u8(0x2222, 0x20ff, 0x81);
        let flags = r.cpu.flags;
        r.step();
        assert_eq!(r.cpu.reg32(0), 0xcafe_0081);
        assert_eq!(r.cpu.reg32(3), 0x0000_2000);
        assert_eq!(r.cpu.flags, flags);
        assert_eq!(r.cpu.ip, 0x104);
        assert_eq!(r.instructions_retired(), 1);

        let mut operand_only = runtime386(&[0x66, 0xd7]);
        operand_only.cpu.bx = 0x1000;
        operand_only.cpu.set_reg32(0, 0xdead_00ff);
        operand_only
            .memory
            .write_u8(operand_only.cpu.ds, 0x10ff, 0x5e);
        operand_only.step();
        assert_eq!(operand_only.cpu.reg32(0), 0xdead_005e);
        assert_eq!(operand_only.cpu.ip, 0x102);

        let mut overflow = runtime386(&[0x67, 0xd7]);
        overflow.cpu.set_reg32(3, 0x0000_ffff);
        overflow.cpu.set_al(1);
        overflow.step();
        assert!(matches!(
            overflow.state(),
            GuestState::Trapped(Trap::SegmentLimit {
                offset: 0x1_0000,
                width: 1,
                ..
            })
        ));

        let mut protected = runtime_for(&[0x26, 0xd7]);
        protected.cpu.es = 0xa000;
        protected.cpu.bx = 0;
        protected.cpu.set_al(0);
        protected.memory.write_u8(0xa000, 0, 0x12);
        protected.memory.set_vga_addressing_enabled(false);
        protected.step();
        assert_eq!(protected.cpu.al(), 0xff);
    }

    #[test]
    fn xlat_uses_ds_by_default_and_last_segment_prefix_wins() {
        let mut r = runtime_for(&[0xd7]);
        r.cpu.ds = 0x3333;
        r.cpu.bx = 0x0100;
        r.cpu.set_al(0x80);
        r.memory.write_u8(r.cpu.ds, 0x0180, 0x42);
        r.step();
        assert_eq!(r.cpu.al(), 0x42);

        let mut wrapped = runtime_for(&[0xd7]);
        wrapped.cpu.ds = 0x3333;
        wrapped.cpu.bx = 0xffff;
        wrapped.cpu.set_al(1);
        wrapped.memory.write_u8(0x3333, 0, 0x9b);
        wrapped.step();
        assert_eq!(wrapped.cpu.al(), 0x9b);

        let mut conflict = runtime_for(&[0x26, 0x3e, 0xd7]);
        conflict.cpu.es = 0x2222;
        conflict.cpu.ds = 0x3333;
        conflict.cpu.bx = 0x0100;
        conflict.memory.write_u8(0x2222, 0x0100, 0x11);
        conflict.memory.write_u8(0x3333, 0x0100, 0x22);
        conflict.step();
        assert_eq!(conflict.cpu.al(), 0x22);

        for prefix in [0x64, 0x65] {
            let mut unsupported = runtime386(&[prefix, 0xd7]);
            unsupported.cpu.set_al(0x03);
            unsupported.step();
            assert!(matches!(
                unsupported.state(),
                GuestState::Trapped(Trap::UnsupportedOpcode { .. })
            ));
            assert_eq!(unsupported.cpu.al(), 0x03);
            assert_eq!(unsupported.instructions_retired(), 0);
        }
    }

    #[test]
    fn immediate_count_shift_executes_dn_instruction() {
        let mut runtime = runtime_for(&[0xc1, 0xe7, 0x02]);
        runtime.set_cpu_profile(crate::CpuProfile::I80286);
        runtime.cpu.di = 1;
        runtime.step();
        assert_eq!(runtime.cpu.di, 4);
        assert_eq!(runtime.cpu.ip, 0x103);
        assert_eq!(runtime.state(), &GuestState::Running);
    }

    #[test]
    fn group_two_instruction_matrix() {
        // Expected result, carry, and one-bit overflow, independently tabulated.
        for (kind, byte, word, cf, of) in [
            (0, 0x03, 0x0003, true, true),
            (1, 0xc0, 0xc000, true, false),
            (2, 0x03, 0x0003, true, true),
            (3, 0xc0, 0xc000, true, false),
            (4, 0x02, 0x0002, true, true),
            (5, 0x40, 0x4000, true, true),
            (7, 0xc0, 0xc000, true, false),
        ] {
            for (opcode, wide) in [
                (0xc0, false),
                (0xc1, true),
                (0xd0, false),
                (0xd1, true),
                (0xd2, false),
                (0xd3, true),
            ] {
                let mut r = runtime_for(&[opcode, 0xc0 | kind << 3, 1]);
                r.set_cpu_profile(crate::CpuProfile::I80286);
                r.cpu.ax = if wide { 0x8001 } else { 0x5581 };
                r.cpu.cx = 1;
                r.cpu.flags = 0xf057;
                let before = r.cpu;
                r.step();
                assert_eq!(r.state(), &GuestState::Running);
                assert_eq!(r.cpu.ax, if wide { word } else { 0x5500 | byte });
                assert_eq!(r.cpu.flags & CpuState::FLAG_CF != 0, cf);
                assert_eq!(r.cpu.flags & CpuState::FLAG_OF != 0, of);
                assert_eq!(r.cpu.ip, 0x100 + if opcode < 0xd0 { 3 } else { 2 });
                assert_eq!(r.cpu.cx, before.cx);
                assert_eq!(r.cpu.sp, before.sp);
                // AF is undefined for shifts: assert only preserved control flags.
                assert_eq!(r.cpu.flags & 0xf700, before.flags & 0xf700);
                if kind < 4 {
                    assert_eq!(r.cpu.flags & 0xd4, before.flags & 0xd4);
                } else {
                    assert_eq!(r.cpu.flags & CpuState::FLAG_ZF, 0);
                    assert_eq!(r.cpu.flags & CpuState::FLAG_SF != 0, kind == 7);
                    assert_eq!(
                        r.cpu.flags & CpuState::FLAG_PF != 0,
                        kind == 7 || (wide && kind == 5)
                    );
                }
            }
        }
    }

    #[test]
    fn immediate_group_two_memory_matches_registers() {
        for opcode in [0xc0, 0xc1] {
            for kind in [0, 1, 2, 3, 4, 5, 7] {
                for count in [0, 1, 2, 9, 17, 32, 255] {
                    let mut reg = runtime_for(&[opcode, 0xc0 | kind << 3, count]);
                    let mut mem = runtime_for(&[opcode, 0x40 | kind << 3, 0xff, count]);
                    for r in [&mut reg, &mut mem] {
                        r.set_cpu_profile(crate::CpuProfile::I80286);
                        r.cpu.ax = 0x8181;
                        r.cpu.bx = 0x2001;
                        r.cpu.si = 0;
                        r.cpu.flags = 0xf8d7;
                    }
                    mem.memory.write_u16(mem.cpu.ds, 0x2000, 0x8181);
                    reg.step();
                    mem.step();
                    assert_eq!(mem.state(), &GuestState::Running);
                    let mask = if opcode == 0xc0 { 0xff } else { 0xffff };
                    assert_eq!(
                        mem.memory.read_u16(mem.cpu.ds, 0x2000) & mask,
                        reg.cpu.ax & mask
                    );
                    assert_eq!(mem.cpu.flags, reg.cpu.flags);
                    assert_eq!(mem.cpu.ax, 0x8181);
                    assert_eq!(mem.cpu.bx, 0x2001);
                    assert_eq!(mem.cpu.ip, 0x104);
                }
            }
        }
    }

    #[test]
    fn rotate_rings_and_arithmetic_shift_boundaries() {
        for (opcode, bits, initial) in [(0xc0, 8u32, 0x81u32), (0xc1, 16, 0x8001)] {
            for kind in [0, 1, 2, 3, 7] {
                for count in [0u8, 2, 8, 9, 16, 17, 31, 32, 33, 255] {
                    let mut r = runtime_for(&[opcode, 0xc0 | kind << 3, count]);
                    r.set_cpu_profile(crate::CpuProfile::I80286);
                    r.cpu.ax = initial as u16;
                    r.cpu.flags = 0xf8d7;
                    let original_flags = r.cpu.flags;
                    let mask = (1u32 << bits) - 1;
                    let mut result = initial;
                    let mut carry = true;
                    let mut steps = count & 31;
                    if kind == 2 || kind == 3 {
                        steps %= (bits + 1) as u8;
                    }
                    for _ in 0..steps {
                        let high = result & (1 << (bits - 1)) != 0;
                        let low = result & 1 != 0;
                        (result, carry) = match kind {
                            0 => (((result << 1) | high as u32) & mask, high),
                            1 => ((result >> 1) | ((low as u32) << (bits - 1)), low),
                            2 => (((result << 1) | carry as u32) & mask, high),
                            3 => ((result >> 1) | ((carry as u32) << (bits - 1)), low),
                            _ => ((result >> 1) | ((high as u32) << (bits - 1)), low),
                        };
                    }
                    r.step();
                    assert_eq!(r.cpu.ax as u32 & mask, result);
                    assert_eq!(r.cpu.flags & CpuState::FLAG_CF != 0, carry);
                    if steps == 0 {
                        assert_eq!(r.cpu.flags, original_flags);
                    }
                    if kind < 4 {
                        assert_eq!(r.cpu.flags & 0xd4, original_flags & 0xd4);
                    }
                }
            }
        }
    }

    #[test]
    fn shift_counts_and_profile_rules() {
        for (wide, width) in [(false, 8), (true, 16)] {
            for count in [0, 1, 2, width, width + 1, 31, 32, 33, 255] {
                let mut r = runtime_for(&[if wide { 0xc1 } else { 0xc0 }, 0xe0, count]);
                r.set_cpu_profile(crate::CpuProfile::I80286);
                r.cpu.ax = 0x8181;
                r.cpu.flags = 0xf8d7;
                let before = r.cpu.flags;
                r.step();
                let bits = count & 31;
                let mask = if wide { 0xffff } else { 0xff };
                let initial: u32 = if wide { 0x8181 } else { 0x81 };
                let expected = (initial << bits) & mask;
                assert_eq!(r.cpu.ax & mask as u16, expected as u16);
                assert_eq!(r.cpu.ip, 0x103);
                if bits == 0 {
                    assert_eq!(r.cpu.flags, before);
                }
                // CF/OF are deliberately not asserted when architecturally undefined.
                if bits > 0 && bits < width {
                    assert_eq!(
                        r.cpu.flags & CpuState::FLAG_CF != 0,
                        initial & (1 << (width - bits)) != 0
                    );
                }
            }
        }
        for (profile, expected) in [
            (crate::CpuProfile::I8086, 0),
            (crate::CpuProfile::I80186, 0x81),
            (crate::CpuProfile::I80286, 0x81),
        ] {
            let mut r = runtime_for(&[0xd2, 0xe0]);
            r.set_cpu_profile(profile);
            r.cpu.ax = 0x81;
            r.cpu.cx = 32;
            r.step();
            assert_eq!(r.cpu.ax, expected);
        }
        for opcode in [0xc0, 0xc1] {
            let mut r = runtime_for(&[opcode, 0xe0, 1]);
            r.step();
            assert!(matches!(
                r.state(),
                GuestState::Trapped(Trap::CpuProfileViolation { .. })
            ));
        }
        for opcode in [0xc0, 0xc1, 0xd0, 0xd1, 0xd2, 0xd3] {
            let mut r = runtime_for(&[opcode, 0xf0, 0]);
            r.set_cpu_profile(crate::CpuProfile::I80286);
            r.step();
            assert!(matches!(
                r.state(),
                GuestState::Trapped(Trap::UnsupportedOpcode { .. })
            ));
        }
    }

    #[test]
    fn immediate_shift_memory_displacement_override_and_wrap() {
        // ES override, [BP-1], imm8; word straddles the 20-bit physical boundary.
        let mut r = runtime_for(&[0x26, 0xc1, 0x66, 0xff, 2]);
        r.set_cpu_profile(crate::CpuProfile::I80286);
        r.cpu.es = 0xffff;
        r.cpu.bp = 0x10;
        r.memory.write_u16(0xffff, 0xf, 0x4001);
        let before = r.cpu;
        r.step();
        assert_eq!(r.memory.read_u16(0xffff, 0xf), 4);
        assert_eq!(r.cpu.ip, 0x105);
        assert_eq!(r.cpu.bp, before.bp);
        assert_eq!(r.cpu.es, before.es);
        assert_eq!(r.cpu.flags & CpuState::FLAG_CF, CpuState::FLAG_CF);
        // Direct disp16 byte operand; all flags preserved at zero count.
        let mut r = runtime_for(&[0xc0, 0x26, 0x00, 0x40, 0]);
        r.set_cpu_profile(crate::CpuProfile::I80286);
        r.memory.write_u8(r.cpu.ds, 0x4000, 0x81);
        let flags = r.cpu.flags;
        r.step();
        assert_eq!(r.cpu.ip, 0x105);
        assert_eq!(r.cpu.flags, flags);
        assert_eq!(r.memory.read_u8(r.cpu.ds, 0x4000), 0x81);
    }

    #[test]
    fn guest_installs_queries_invokes_and_restores_multiplex_vector() {
        let mut r = runtime_for(&[
            0xba, 0x00, 0x02, // MOV DX,0200
            0xb8, 0x2f, 0x25, 0xcd, 0x21, // set DS:DX
            0xb8, 0x2f, 0x35, 0xcd, 0x21, // get ES:BX
            0xcd, 0x2f, 0xf4,
        ]);
        r.memory
            .write_slice(r.cpu.ds, 0x200, &[0xb9, 0x34, 0x12, 0xcf])
            .unwrap();
        let segment = r.cpu.ds;
        r.run_slice(20);
        assert_eq!(r.state(), &GuestState::Halted);
        assert_eq!((r.cpu.es, r.cpu.bx, r.cpu.cx), (segment, 0x200, 0x1234));
        // Execute DOS restoration, using a separate code location.
        r.memory
            .write_slice(segment, 0x300, &[0xb8, 0x2f, 0x25, 0xcd, 0x21])
            .unwrap();
        r.cpu.ip = 0x300;
        r.cpu.ds = 0x0800;
        r.cpu.dx = 0x0400;
        r.state = GuestState::Running;
        r.run_slice(2);
        assert_eq!(r.interrupt_vector(0x2f), (0x0800, 0x0400));
    }

    #[test]
    fn nested_guest_interrupts_and_far_call_chains_restore_stack_and_flags() {
        let mut r = runtime_for(&[0xcd, 0x60, 0xf4]);
        // PUSHF; CALL FAR predecessor; INT 61; IRET.
        r.memory
            .write_slice(
                0x2000,
                0x20,
                &[0x9c, 0x9a, 0x40, 0x00, 0x00, 0x20, 0xcd, 0x61, 0xcf],
            )
            .unwrap();
        r.memory.write_slice(0x2000, 0x40, &[0x43, 0xcf]).unwrap();
        r.memory.write_slice(0x2000, 0x60, &[0x41, 0xcf]).unwrap();
        // Direct IVT stores, independent of DOS AH=25.
        r.memory.write_u16(0, 0x180, 0x20);
        r.memory.write_u16(0, 0x182, 0x2000);
        r.memory.write_u16(0, 0x184, 0x60);
        r.memory.write_u16(0, 0x186, 0x2000);
        r.cpu.flags = 0x0303;
        let before = r.cpu;
        r.step();
        assert_eq!(r.memory.read_u16(before.ss, r.cpu.sp), before.ip + 2);
        assert_eq!(r.memory.read_u16(before.ss, r.cpu.sp + 2), before.cs);
        assert_eq!(r.memory.read_u16(before.ss, r.cpu.sp + 4), before.flags);
        r.run_slice(20);
        assert_eq!(r.state(), &GuestState::Halted);
        assert_eq!((r.cpu.bx, r.cpu.cx), (before.bx + 1, before.cx + 1));
        assert_eq!(
            (r.cpu.sp, r.cpu.cs, r.cpu.flags),
            (before.sp, before.cs, before.flags)
        );
    }

    #[test]
    fn bios_system_fallback_uses_the_guest_frame_and_preserves_registers() {
        for (ax, bx, flags) in [(0x1022, 0, 0x0fd6), (0x885a, 0xbeef, 0x0443)] {
            let mut r = runtime_for(&[0xcd, 0x15, 0xf4]);
            r.cpu.ax = ax;
            r.cpu.bx = bx;
            r.cpu.flags = flags;
            r.cpu.bp = 0x4567;
            r.cpu.ds = 0x3456;
            r.cpu.gpr_upper = [0x1234; 8];
            let before = r.cpu;
            let handler = r.interrupt_vector(0x15);
            assert_ne!(handler, (0, 0));
            // Writes through a segment alias cannot overwrite the predecessor.
            r.memory.write_u8(handler.0 - 1, handler.1 + 0x10, 0xf4);
            r.step();
            assert_eq!((r.cpu.cs, r.cpu.ip), handler);
            assert_eq!(r.cpu.sp, before.sp - 6);
            assert_eq!(
                r.cpu.flags,
                flags & !(CpuState::FLAG_IF | CpuState::FLAG_TF)
            );
            assert_eq!(r.memory.read_u16(before.ss, r.cpu.sp), before.ip + 2);
            assert_eq!(r.memory.read_u16(before.ss, r.cpu.sp + 2), before.cs);
            assert_eq!(r.memory.read_u16(before.ss, r.cpu.sp + 4), flags);
            r.run_slice(6);
            assert_eq!(
                r.cpu,
                CpuState {
                    ax: 0x8600 | (ax & 0xff),
                    ip: before.ip + 2,
                    flags: flags | CpuState::FLAG_CF,
                    ..before
                }
            );
            assert_eq!(r.instructions_retired(), 7);
        }
    }

    #[test]
    fn bios_system_hook_can_handle_or_chain_to_saved_predecessor() {
        for form in 0..3 {
            // Install the hook via DOS (MOV AX,2515h; INT 21h).
            let mut r = runtime_for(&[0xb8, 0x15, 0x25, 0xcd, 0x21, 0xcd, 0x15, 0xf4]);
            let (segment, offset) = r.interrupt_vector(0x15);
            let [lo, hi] = offset.to_le_bytes();
            let [seg_lo, seg_hi] = segment.to_le_bytes();
            r.cpu.ds = 0x2000;
            r.cpu.dx = 0x20;
            let hook = match form {
                0 => alloc::vec![0xb8, 0x34, 0x12, 0xcf], // private hook result
                1 => alloc::vec![0x41, 0xea, lo, hi, seg_lo, seg_hi], // INC CX; JMP FAR
                // PUSHF; CALL FAR; ADC DX,0 observes predecessor CF; IRET.
                _ => alloc::vec![0x9c, 0x9a, lo, hi, seg_lo, seg_hi, 0x83, 0xd2, 0, 0xcf],
            };
            r.memory.write_slice(0x2000, 0x20, &hook).unwrap();
            r.run_slice(2);
            assert_eq!(r.interrupt_vector(0x15), (0x2000, 0x20));
            r.cpu.ax = 0x195a;
            r.cpu.cx = 0;
            r.cpu.flags = 0x0202;
            let before = r.cpu;
            r.run_slice(20);
            assert_eq!(r.state(), &GuestState::Halted);
            assert_eq!(
                r.cpu,
                CpuState {
                    ax: if form == 0 { 0x1234 } else { 0x865a },
                    cx: u16::from(form == 1),
                    dx: before.dx + u16::from(form == 2),
                    flags: before.flags | u16::from(form == 1),
                    ip: before.ip + 3,
                    ..before
                }
            );
        }
    }

    #[test]
    fn bios_system_calls_reuse_the_stack_and_finish_within_the_slice_budget() {
        // INT 15h; LOOP back to INT; HLT.
        let mut r = runtime_for(&[0xcd, 0x15, 0xe2, 0xfc, 0xf4]);
        r.cpu.cx = 10_000;
        r.cpu.ax = 0xff55;
        let sp = r.cpu.sp;
        let vector = r.interrupt_vector(0x15);
        r.run_slice(80_001);
        assert_eq!(r.state(), &GuestState::Halted);
        assert_eq!(r.instructions_retired(), 80_001);
        assert_eq!((r.cpu.sp, r.cpu.cx, r.cpu.ax), (sp, 0, 0x8655));
        assert_eq!(r.interrupt_vector(0x15), vector);

        // A guest may deliberately recurse. It consumes the normal slice
        // budget; the host never recursively dispatches the interrupt.
        let mut r = runtime_for(&[0xcd, 0x15]);
        let entry = (r.cpu.cs, r.cpu.ip);
        r.set_interrupt_vector(0x15, entry.0, entry.1);
        r.run_slice(64);
        assert_eq!(r.state(), &GuestState::Running);
        assert_eq!(r.instructions_retired(), 64);
        assert_eq!(
            r.last_slice_stop_reason,
            crate::SliceStopReason::BudgetExhausted
        );
    }

    #[test]
    fn multiplex_predecessor_accepts_far_call_and_tail_jump() {
        for handler in [
            alloc::vec![0x9c, 0x9a, 0x00, 0x04, 0x00, 0x08, 0xcf],
            alloc::vec![0xea, 0x00, 0x04, 0x00, 0x08],
        ] {
            let mut r = runtime_for(&[0xcd, 0x2f, 0xf4]);
            r.memory.write_slice(0x2000, 0, &handler).unwrap();
            r.set_interrupt_vector(0x2f, 0x2000, 0);
            let before = r.cpu;
            r.run_slice(12);
            assert_eq!(r.state(), &GuestState::Halted);
            assert_eq!((r.cpu.sp, r.cpu.flags), (before.sp, before.flags));
        }
    }

    #[test]
    fn interrupt_pointer_and_stack_wrap_within_private_megabyte() {
        let mut r = runtime_for(&[0xcd, 0x60]);
        r.set_interrupt_vector(0x60, 0xffff, 0x0010); // wraps to IVT byte zero
        r.memory.write_u8(0, 0, 0xcf);
        r.cpu.ss = 0xffff;
        r.cpu.sp = 0x0004;
        let before = r.cpu;
        r.run_slice(2);
        assert_eq!(
            (r.cpu.cs, r.cpu.ip, r.cpu.sp),
            (before.cs, before.ip + 2, before.sp)
        );
        let mut r = runtime_for(&[0xcd, 0x60]);
        // Real mode has no invalid segment encoding: invalid code traps safely.
        r.set_interrupt_vector(0x60, 0xf000, 0);
        r.memory.write_u8(0xf000, 0, 0x0f);
        r.run_slice(2);
        assert!(matches!(r.state(), GuestState::Trapped(_)));
        let mut r = runtime_for(&[0xcd, 0x60]);
        r.step();
        assert!(matches!(
            r.state(),
            GuestState::Trapped(Trap::UnsupportedInterrupt {
                interrupt: 0x60,
                ..
            })
        ));
    }

    #[test]
    fn repeated_children_release_own_vectors_but_preserve_parent_handler() {
        let mut r = runtime_for(&[0x90]);
        r.resize_memory(r.current_psp(), 0x100).unwrap();
        let parent = r.current_psp();
        r.set_interrupt_vector(0x60, parent, 0x80);
        let path = r.drives.parse_path(b"C:\\HOOK.COM").unwrap();
        r.drives
            .add_base_file(
                crate::DosDrive::C,
                "HOOK.COM",
                alloc::vec![
                    0xba, 0x00, 0x01, 0xb8, 0x61, 0x25, 0xcd, 0x21, 0xb8, 0x00, 0x4c, 0xcd, 0x21,
                ],
            )
            .unwrap();
        for _ in 0..3 {
            r.exec(path.clone(), b"", 0).unwrap();
            r.run_slice(5);
            assert_eq!(r.current_psp(), parent);
            assert_eq!(r.interrupt_vector(0x60), (parent, 0x80));
            assert_eq!(r.interrupt_vector(0x61), (0, 0));
        }
    }

    #[test]
    fn released_and_shrunk_allocations_cannot_leave_live_vectors() {
        let mut r = runtime_for(&[0xb8, 0x00, 0x4c, 0xcd, 0x21]);
        let psp = r.current_psp();
        r.set_interrupt_vector(0x60, psp, 0x1800);
        r.resize_memory(psp, 0x100).unwrap();
        assert_eq!(r.interrupt_vector(0x60), (0, 0));
        let block = r.allocate_memory(0x20).unwrap();
        r.set_interrupt_vector(0x2f, block, 0);
        r.free_memory(block).unwrap();
        assert_eq!(r.interrupt_vector(0x2f), (0x0800, 0x0400));
        r.set_interrupt_vector(0x2f, psp, 0x100);
        r.run_slice(2);
        assert_eq!(r.state(), &GuestState::Exited { code: 0 });
        assert_eq!(r.interrupt_vector(0x2f), (0x0800, 0x0400));
    }

    #[test]
    fn installed_multiplex_handler_runs_in_guest_and_iret_restores_the_frame() {
        let mut runtime = runtime_for(&[0xcd, 0x2f, 0xf4]);
        runtime
            .memory
            .write_slice(0x2000, 0x20, &[0xbb, 0x4e, 0x44, 0xcf])
            .unwrap();
        runtime.set_interrupt_vector(0x2f, 0x2000, 0x20);
        assert_eq!(runtime.memory.read_u16(0, 0xbc), 0x20);
        assert_eq!(runtime.memory.read_u16(0, 0xbe), 0x2000);
        runtime.cpu.flags = CpuState::FLAG_IF | CpuState::FLAG_TF | CpuState::FLAG_CF | 2;
        let before = runtime.cpu;
        runtime.run_slice(1);
        assert_eq!((runtime.cpu.cs, runtime.cpu.ip), (0x2000, 0x20));
        assert_eq!(runtime.cpu.sp, before.sp.wrapping_sub(6));
        assert_eq!(
            runtime.cpu.flags & (CpuState::FLAG_IF | CpuState::FLAG_TF),
            0
        );
        assert_eq!(
            runtime.memory.read_u16(before.ss, runtime.cpu.sp),
            before.ip + 2
        );
        runtime.run_slice(3);
        assert_eq!(runtime.state(), &GuestState::Halted);
        assert_eq!(runtime.cpu.bx, 0x444e);
        assert_eq!(runtime.cpu.cs, before.cs);
        assert_eq!(runtime.cpu.sp, before.sp);
        assert_eq!(runtime.cpu.flags, before.flags);
        runtime.set_interrupt_vector(0x2f, 0, 0);
        assert_eq!(runtime.interrupt_vector(0x2f), (0, 0));
        assert_eq!(runtime.memory.read_u16(0, 0xbe), 0);
    }

    #[test]
    fn conditional_loops_decrement_cx_preserve_flags_and_wrap_zero() {
        for opcode in [0xe0, 0xe1] {
            for zero in [false, true] {
                for count in [0u16, 1, 2] {
                    // A taken branch skips MOV AL,1 and lands on HLT.
                    let mut runtime = runtime_for(&[opcode, 2, 0xb0, 1, 0xf4]);
                    runtime.cpu.cx = count;
                    runtime.cpu.flags = CpuState::FLAG_CF
                        | CpuState::FLAG_IF
                        | 2
                        | if zero { CpuState::FLAG_ZF } else { 0 };
                    let flags = runtime.cpu.flags;
                    runtime.run_slice(3);
                    let remaining = count.wrapping_sub(1);
                    let taken = remaining != 0 && (zero == (opcode == 0xe1));
                    assert_eq!(runtime.cpu.cx, remaining);
                    assert_eq!(runtime.cpu.al(), if taken { 0 } else { 1 });
                    assert_eq!(runtime.cpu.flags, flags);
                    assert_eq!(runtime.state(), &GuestState::Halted);
                }
            }
        }
    }

    #[test]
    fn software_fpu_interrupt_uses_the_authoritative_guest_ivt() {
        let mut runtime = runtime_for(&[0xcd, 0x37, 0xf4]);
        runtime.memory.write_u16(0, 0x37 * 4, 0x20);
        runtime.memory.write_u16(0, 0x37 * 4 + 2, 0x2000);
        runtime
            .memory
            .write_slice(0x2000, 0x20, &[0xb8, 0x34, 0x12, 0xcf])
            .unwrap();
        runtime.run_slice(4);
        assert_eq!(runtime.state(), &GuestState::Halted);
        assert_eq!(runtime.cpu.ax, 0x1234);
        assert_eq!(runtime.cpu.cs, PSP_SEGMENT);
    }

    #[test]
    fn sunlight_mines_bundle_loads_as_an_mz_guest() {
        let image = include_bytes!("../../SunlightMines.sunapp/Program/SUNMINE.EXE");
        let mut runtime = Runtime::from_program(image, &[]).unwrap();

        for _ in 0..1_000 {
            runtime.run_slice(4_096);
            if runtime.state() == &GuestState::YieldedUntilTimer {
                break;
            }
        }

        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(
            runtime.state(),
            &GuestState::YieldedUntilTimer,
            "cpu={:?} retired={}",
            runtime.cpu,
            runtime.instructions_retired()
        );
        assert!(runtime.cooperative_yielded_last_slice());
        assert_eq!(runtime.cursor_shape(), 0x2607);
        assert!(!runtime.text_cursor_visible());
        assert_eq!(runtime.mouse().ranges(), (0, 79, 0, 24));
        assert!(runtime.mouse().cursor_visible());
        let title: [u8; 14] = core::array::from_fn(|column| runtime.cell(29 + column, 0).character);
        assert_eq!(&title, b"SUNLIGHT MINES");
        assert_eq!(
            (runtime.cell(31, 3).character, runtime.cell(32, 3).character),
            (0xdb, 0xdb)
        );
    }

    fn run_to_palette_generation(runtime: &mut Runtime, generation: u64) {
        let mut instructions = 0usize;
        while runtime.palette_generation() < generation {
            runtime.step();
            instructions += 1;
            assert!(
                instructions < 1_000_000,
                "palette checkpoint {generation} was not reached: state={:?}",
                runtime.state()
            );
        }
        assert_eq!(runtime.palette_generation(), generation);
    }

    fn run_to_next_idle_hint(runtime: &mut Runtime) {
        let initial = runtime.cooperative_yield_count();
        runtime.wake_from_timer();
        for _ in 0..2_000 {
            runtime.run_slice(4_096);
            if runtime.cooperative_yield_count() != initial {
                return;
            }
        }
        panic!("guest did not reach INT 28h: state={:?}", runtime.state());
    }

    #[test]
    fn hello_chronos_regression_exits_with_its_original_output() {
        let mut runtime = runtime_for(HELLO_CHRONOS_COM);
        runtime.run_slice(128);
        let output: [u8; 19] = core::array::from_fn(|column| runtime.cell(column, 0).character);
        assert_eq!(&output, b"Hello from Chronos!");
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
    }

    #[test]
    fn environment_exposes_guest_comspec_and_shell_paths() {
        let mut runtime = runtime_for(&[0xf4]);
        runtime.set_executable_path(b"C:\\SUNSH.EXE");
        let segment = runtime.active_process.environment_segment;
        let mut environment = [0u8; 96];
        runtime
            .memory
            .read_slice(segment, 0, &mut environment)
            .unwrap();
        assert!(environment
            .windows(b"COMSPEC=C:\\SUNSH.EXE\0".len())
            .any(|entry| entry == b"COMSPEC=C:\\SUNSH.EXE\0"));
        assert!(environment
            .windows(b"SHELL=C:\\SUNSH.EXE\0".len())
            .any(|entry| entry == b"SHELL=C:\\SUNSH.EXE\0"));
    }

    #[test]
    fn no_8087_escapes_are_ignored_so_real_mode_programs_can_probe_for_an_fpu() {
        let mut runtime = runtime_for(&[0xb0, 0, 0xa2, 0, 2, 0xd9, 0x3e, 0, 2, 0xf4]);
        runtime.run_slice(4);
        assert_eq!(runtime.memory.read_u8(runtime.cpu.ds, 0x0200), 0);
        assert_eq!(runtime.state(), &GuestState::Halted);
    }

    #[test]
    fn repe_scasb_preserves_zero_flag_after_matching_a_bounded_sentinel_area() {
        let mut runtime = runtime_for(&[
            0xb8, 0x00, 0x20, 0x8e, 0xc0, 0x31, 0xff, 0xb9, 0x20, 0x00, 0xb0, 0x01, 0xfc, 0xf3,
            0xae, 0x74, 0x02, 0xb0, 0xff, 0xf4,
        ]);
        for offset in 0..32 {
            runtime.memory.write_u8(0x2000, offset, 1);
        }

        runtime.run_slice(64);

        assert_eq!(runtime.cpu.cx, 0);
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_ZF, 0);
        assert_eq!(runtime.cpu.al(), 1);
        assert_eq!(runtime.state(), &GuestState::Halted);
    }

    #[test]
    fn repne_scasb_stops_after_finding_a_matching_byte() {
        let mut runtime = runtime_for(&[
            0xb8, 0x00, 0x20, 0x8e, 0xc0, 0x31, 0xff, 0xb9, 0x20, 0x00, 0xb0, 0x7e, 0xfc, 0xf2,
            0xae, 0xf4,
        ]);
        runtime.memory.write_u8(0x2000, 9, 0x7e);

        runtime.run_slice(64);

        assert_eq!(runtime.cpu.cx, 22);
        assert_eq!(runtime.cpu.di, 10);
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_ZF, 0);
        assert_eq!(runtime.state(), &GuestState::Halted);
    }

    #[test]
    fn repe_cmpsw_stops_at_the_first_mismatch() {
        let mut runtime = runtime_for(&[
            0xb8, 0x00, 0x20, 0x8e, 0xc0, 0x31, 0xf6, 0x31, 0xff, 0xb9, 0x03, 0x00, 0xfc, 0xf3,
            0xa7, 0xf4,
        ]);
        runtime.memory.write_u16(PSP_SEGMENT, 0, 1);
        runtime.memory.write_u16(PSP_SEGMENT, 2, 2);
        runtime.memory.write_u16(PSP_SEGMENT, 4, 3);
        runtime.memory.write_u16(0x2000, 0, 1);
        runtime.memory.write_u16(0x2000, 2, 9);
        runtime.memory.write_u16(0x2000, 4, 3);

        runtime.run_slice(64);

        assert_eq!(runtime.cpu.cx, 1);
        assert_eq!((runtime.cpu.si, runtime.cpu.di), (4, 4));
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_ZF, 0);
        assert_eq!(runtime.state(), &GuestState::Halted);
    }

    #[test]
    fn free_pascal_sunshell_mz_starts_and_executes_a_command_without_host_interception() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut runtime = Runtime::from_program(image, b"/C VER").unwrap();
        let load_segment = runtime.cpu.cs;
        let data_segment = runtime.memory.read_u16(load_segment, 0x0001);
        assert_eq!(runtime.cpu.cs, load_segment);
        assert_ne!(data_segment, 0);
        let mut interrupts = Vec::new();
        for _ in 0..1_000_000 {
            if runtime.memory.read_u8(runtime.cpu.cs, runtime.cpu.ip) == 0xcd
                && runtime
                    .memory
                    .read_u8(runtime.cpu.cs, runtime.cpu.ip.wrapping_add(1))
                    == 0x21
            {
                interrupts.push((
                    runtime.cpu.ip,
                    runtime.cpu.ah(),
                    runtime.cpu.ax,
                    runtime.cpu.bx,
                ));
                if interrupts.len() > 32 {
                    interrupts.remove(0);
                }
            }
            runtime.step();
            if !matches!(runtime.state(), GuestState::Running | GuestState::Ready) {
                break;
            }
        }
        let data_prefix: [u8; 32] =
            core::array::from_fn(|offset| runtime.memory.read_u8(runtime.cpu.ds, offset as u16));
        let output: [u8; 22] = core::array::from_fn(|column| runtime.cell(column, 0).character);
        assert_eq!(
            runtime.state(),
            &GuestState::Exited { code: 0 },
            "output={output:?}, data_prefix={data_prefix:?}, interrupts={interrupts:?}, cpu={:?}",
            runtime.cpu,
        );
        assert_eq!(&output, b"Sunlight DOS Shell 0.1");
    }

    #[test]
    fn free_pascal_sunshell_interactive_mode_renders_the_cmd_prompt_before_waiting() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut runtime = Runtime::from_program(image, b"").unwrap();

        runtime.run_slice(10_000_000);

        assert_eq!(
            runtime.state(),
            &GuestState::WaitingForInput,
            "cpu={:?}, cursor=({}, {}), first_line={:?}",
            runtime.cpu,
            runtime.cursor_column(),
            runtime.cursor_row(),
            core::array::from_fn::<_, 80, _>(|column| runtime.cell(column, 0).character),
        );
        let prompt: [u8; 8] = core::array::from_fn(|column| runtime.cell(column, 4).character);
        assert_eq!(&prompt, b"CMD C:\\>");
    }

    #[test]
    fn free_pascal_sunshell_executes_help_after_one_enter_keypress() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut runtime = Runtime::from_program(image, b"").unwrap();
        runtime.run_slice(10_000_000);

        for ascii in [b'H', b'E', b'L', b'P', b'\r'] {
            runtime.inject_ascii(ascii);
            runtime.run_slice(10_000_000);
        }

        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        let help: [u8; 16] = core::array::from_fn(|column| runtime.cell(column, 5).character);
        assert_eq!(&help, b"CLS CD DIR ECHO ");
        let prompt: [u8; 8] = core::array::from_fn(|column| runtime.cell(column, 6).character);
        assert_eq!(&prompt, b"CMD C:\\>");
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (8, 6));
    }

    #[test]
    fn free_pascal_sunshell_runs_the_bundled_midterm_batch_through_dos_files() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let autoexec = include_bytes!("../../ChronosDosShell.sunapp/Program/AUTOEXEC.BAT");
        let midterm = include_bytes!("../../ChronosDosShell.sunapp/Program/MIDTERM.BAT");
        let mut runtime = Runtime::from_program(image, b"/C C:\\MIDTERM.BAT").unwrap();
        runtime
            .drives_mut()
            .add_base_file(crate::DosDrive::C, "AUTOEXEC.BAT", autoexec.to_vec())
            .unwrap();
        runtime
            .drives_mut()
            .add_base_file(crate::DosDrive::C, "MIDTERM.BAT", midterm.to_vec())
            .unwrap();

        runtime.run_slice(1_000_000);

        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
        let pass_line: [u8; 22] = core::array::from_fn(|column| runtime.cell(column, 10).character);
        assert_eq!(&pass_line, b"CHRONOS MIDTERM: PASS ");
        let temporary_root = runtime.drives().parse_path(b"T:\\MIDTERM").unwrap();
        assert!(runtime.drives().list(&temporary_root, b"*.*", 0).is_err());
    }

    #[test]
    fn inc_and_dec_byte_modrm_are_available_on_the_8086_profile() {
        let mut runtime = runtime_for(&[
            0xc6, 0x06, 0x00, 0x02, 41, 0xfe, 0x06, 0x00, 0x02, 0xfe, 0x0e, 0x00, 0x02,
        ]);
        runtime.run_slice(3);
        assert_eq!(runtime.memory.read_u8(runtime.cpu.ds, 0x0200), 41);
    }

    #[test]
    fn accumulator_immediate_logic_instructions_are_available_on_the_8086_profile() {
        let mut runtime = runtime_for(&[
            0xb8, 0xf0, 0x0f, 0x25, 0x0f, 0x0f, 0x0d, 0x00, 0x80, 0x35, 0xff, 0x00, 0xa9, 0xf0,
            0x0f,
        ]);
        runtime.run_slice(5);
        assert_eq!(runtime.cpu.ax, 0x8fff);
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_ZF, 0);

        let mut byte = runtime_for(&[0xb0, 0xf3, 0x24, 0x0f, 0x0c, 0x80, 0x34, 0x03, 0xa8, 0x80]);
        byte.run_slice(5);
        assert_eq!(byte.cpu.al(), 0x80);
        assert_eq!(byte.cpu.flags & CpuState::FLAG_ZF, 0);
    }

    #[test]
    fn all_modrm_effective_address_forms_and_default_segments_work() {
        let cases = [
            (0, 0x1111u16, false),
            (1, 0x1211, false),
            (2, 0x2111, true),
            (3, 0x2211, true),
            (4, 0x0111, false),
            (5, 0x0211, false),
            (7, 0x1000, false),
        ];
        for (rm, expected, uses_ss) in cases {
            let mut runtime = runtime_for(&[0x8a, 0x00 | rm]);
            runtime.cpu.bx = 0x1000;
            runtime.cpu.si = 0x0111;
            runtime.cpu.di = 0x0211;
            runtime.cpu.bp = 0x2000;
            runtime.cpu.ds = 0x1111;
            runtime.cpu.ss = 0x2222;
            runtime
                .memory
                .write_u8(if uses_ss { 0x2222 } else { 0x1111 }, expected, rm);
            runtime.step();
            assert_eq!(runtime.cpu.al(), rm);
        }
        let mut bp = runtime_for(&[0x8a, 0x46, 0x00]);
        bp.cpu.bp = 0x0303;
        bp.cpu.ss = 0x2222;
        bp.memory.write_u8(0x2222, 0x0303, 6);
        bp.step();
        assert_eq!(bp.cpu.al(), 6);
        let mut direct = runtime_for(&[0x8a, 0x06, 0x34, 0x12]);
        direct.memory.write_u8(direct.cpu.ds, 0x1234, 0xa5);
        direct.step();
        assert_eq!(direct.cpu.al(), 0xa5);
    }

    #[test]
    fn segment_override_wins_over_bp_default_segment() {
        let mut runtime = runtime_for(&[0x26, 0x8a, 0x46, 0x00]);
        runtime.cpu.bp = 0x0040;
        runtime.cpu.ss = 0x2222;
        runtime.cpu.es = 0x3333;
        runtime.memory.write_u8(0x2222, 0x0040, 1);
        runtime.memory.write_u8(0x3333, 0x0040, 2);
        runtime.step();
        assert_eq!(runtime.cpu.al(), 2);
    }

    #[test]
    fn mov_segment_lea_stack_and_near_call_work() {
        let mut runtime = runtime_for(&[
            0xb8, 0x34, 0x12, 0x8e, 0xd8, 0x8d, 0x5e, 0x02, 0x50, 0x58, 0xe8, 0x01, 0x00, 0xf4,
            0x43, 0xc3,
        ]);
        runtime.cpu.bp = 0x1000;
        runtime.run_slice(16);
        assert_eq!(runtime.cpu.ds, 0x1234);
        assert_eq!(runtime.cpu.bx, 0x1003);
        assert_eq!(runtime.cpu.ax, 0x1234);
        assert_eq!(runtime.cpu.ax, 0x1234);
    }

    #[test]
    fn arithmetic_flags_and_branches_are_not_approximated() {
        let mut runtime =
            runtime_for(&[0xb0, 0xff, 0x04, 0x01, 0x74, 0x02, 0xb3, 0x00, 0xb3, 0x01]);
        runtime.run_slice(4);
        assert_eq!(runtime.cpu.al(), 0);
        assert!(runtime.cpu.flags & CpuState::FLAG_CF != 0);
        assert!(runtime.cpu.flags & CpuState::FLAG_ZF != 0);
        assert_eq!(runtime.cpu.bl(), 1);
    }

    #[test]
    fn rep_stosw_is_sliced_and_writes_guest_video_memory() {
        let mut runtime = runtime_for(&[
            0xb8, 0x00, 0xb8, 0x8e, 0xc0, 0xbf, 0x00, 0x00, 0xb9, 0x03, 0x00, 0xb8, 0x41, 0x1f,
            0xf3, 0xab,
        ]);
        runtime.run_slice(6);
        assert_eq!(runtime.cpu.cx, 2);
        runtime.run_slice(8);
        assert_eq!(runtime.cpu.cx, 0);
        assert_eq!(runtime.cell(0, 0).character, b'A');
        assert_eq!(runtime.cell(2, 0).attribute, 0x1f);
    }

    #[test]
    fn video_byte_writes_and_bios_cursor_are_coherent() {
        let mut runtime = runtime_for(&[
            0xb8, 0x00, 0xb8, 0x8e, 0xc0, 0xbf, 0x00, 0x00, 0xb0, b'X', 0x26, 0x88, 0x05, 0xb0,
            0x4e, 0x26, 0x88, 0x45, 0x01,
        ]);
        runtime.run_slice(7);
        assert_eq!(runtime.cell(0, 0).character, b'X');
        assert_eq!(runtime.cell(0, 0).attribute, 0x4e);

        runtime.cpu.set_ah(0x02);
        runtime.cpu.set_dh(3);
        runtime.cpu.set_dl(4);
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        runtime.cpu.set_ah(0x03);
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!((runtime.cpu.dh(), runtime.cpu.dl()), (3, 4));
    }

    #[test]
    fn ega_configuration_guest_interrupt_preserves_state_and_memory() {
        for al in [0, 0x1a, 0xff] {
            for bh in [0, 0xaa, 0xff] {
                let mut r = runtime_for(&[0xcd, 0x10, 0xcd, 0x10]);
                r.cpu.ax = 0x1200 | al;
                r.cpu.bx = (bh << 8) | 0x10;
                r.cpu.cx = 0xffff;
                r.cpu.dx = 0x2345;
                r.cpu.si = 0x6789;
                r.cpu.di = 0xabcd;
                r.cpu.bp = 0xef01;
                r.cpu.flags = 0x0ed7;
                let before = r.cpu;
                let memory: Vec<_> = (0..0x100000)
                    .map(|p| r.memory.read_u8((p >> 4) as u16, (p & 15) as u16))
                    .collect();
                r.step();
                let mut expected = before;
                expected.bx = 0;
                expected.cx = 9;
                expected.ip += 2;
                assert_eq!(r.cpu, expected);
                assert_eq!(r.state(), &GuestState::Running);
                r.cpu.ax = before.ax;
                r.cpu.bx = before.bx;
                r.step();
                expected.ip += 2;
                assert_eq!(r.cpu, expected);
                for (p, byte) in memory.into_iter().enumerate() {
                    assert_eq!(r.memory.read_u8((p >> 4) as u16, (p & 15) as u16), byte);
                }
            }
        }
    }

    #[test]
    fn video_profile_bda_and_mode_queries_agree() {
        let mut r = runtime_for(&[0xcd, 0x10]);
        for (mode, columns, page_size, points) in [(3, 80, 0x1000, 16), (0x13, 40, 64000, 8)] {
            r.cpu.ip = 0x100;
            r.cpu.ax = mode;
            r.step();
            assert_eq!(r.memory.read_u8(0x40, 0x49), mode as u8);
            assert_eq!(r.memory.read_u16(0x40, 0x4a), columns);
            assert_eq!(r.memory.read_u16(0x40, 0x4c), page_size);
            assert_eq!(r.memory.read_u16(0x40, 0x4e), 0);
            assert_eq!(r.memory.read_u8(0x40, 0x62), 0);
            assert_eq!(r.memory.read_u16(0x40, 0x63), 0x3d4);
            assert_eq!(r.memory.read_u8(0x40, 0x84), 24);
            assert_eq!(r.memory.read_u16(0x40, 0x85), points);
            assert_eq!(r.memory.read_u8(0x40, 0x88), 9);
            r.cpu.ip = 0x100;
            r.cpu.ax = 0x0f00;
            r.cpu.bx = 0xaaff;
            r.step();
            assert_eq!(r.cpu.ax, (columns << 8) | mode);
            assert_eq!(r.cpu.bx, 0x00ff);
        }
    }

    #[test]
    fn bios_text_write_preserves_cursor_and_does_not_scroll() {
        let mut r = runtime_for(&[0xcd, 0x10]);
        r.set_cursor(79, 24);
        r.cpu.ax = 0x095a;
        r.cpu.bx = 0x004e;
        r.cpu.cx = 0xffff;
        r.step();
        assert_eq!(
            r.cell(79, 24),
            crate::TextCell {
                character: b'Z',
                attribute: 0x4e
            }
        );
        assert_eq!(r.cell(0, 0).character, b' ');
        assert_eq!((r.cursor_column(), r.cursor_row()), (79, 24));
        assert_eq!(r.memory.read_u16(0x40, 0x50), 0x184f);
        r.memory.write_u16(0xb800, 3998, 0x1f58);
        r.cpu.ip = 0x100;
        r.cpu.ax = 0x0800;
        r.step();
        assert_eq!(r.cpu.ax, 0x1f58);
    }

    #[test]
    fn bios_cursor_and_single_page_contract() {
        let mut r = runtime_for(&[0xcd, 0x10]);
        for (ax, bx, cx, dx) in [
            (0x0100, 0, 0x2007, 0),
            (0x0200, 0, 0, 0x0304),
            (0x0500, 0, 0, 0),
            (0x0300, 0x00ab, 0, 0),
        ] {
            r.cpu.ip = 0x100;
            r.cpu.ax = ax;
            r.cpu.bx = bx;
            r.cpu.cx = cx;
            r.cpu.dx = dx;
            r.step();
        }
        assert_eq!(r.cpu.bx, 0x00ab);
        assert_eq!(r.cpu.cx, 0x2007);
        assert_eq!(r.cpu.dx, 0x0304);
        assert_eq!(r.memory.read_u16(0x40, 0x50), 0x0304);
        assert_eq!(r.memory.read_u16(0x40, 0x60), 0x2007);
        assert!(!r.text_cursor_visible());
    }

    #[test]
    fn vga_decoder_controls_guest_memory_and_dac_without_changing_display() {
        let mut r = runtime_for(&[0xcd, 0x10, 0xef, 0xee, 0xee, 0xee, 0xec, 0xe4, 0x61]);
        r.memory.write_u16(0xb800, 0, 0x8f41);
        r.memory.write_u8(0xa000, 0, 4);
        r.memory.write_u8(0xb7ff, 15, 0x55);
        r.cpu.gpr_upper = [0x1234; 8];
        r.cpu.bx = 0xa532;
        r.cpu.cx = 0x4321;
        r.cpu.si = 0xabcd;
        r.cpu.di = 0xef01;
        let cell = r.cell(0, 0);
        let palette = r.palette_checksum();
        let generation = r.framebuffer_generation();
        let mode = r.video_mode();
        let attr = r.text_attribute_mode();
        let start = r.cpu.ip;
        // Valid requests are idempotent; reserved AL=12h cannot look like success.
        for (al, flags, status, enabled) in [
            (1, 0x0ed7, 0x12, false),
            (1, 2, 0x12, false),
            (0x12, 0x0202, 0, false),
            (0, 3, 0x12, true),
            (0, 0x0202, 0x12, true),
        ] {
            r.cpu.ip = start;
            r.cpu.ax = 0x1200 | al;
            r.cpu.flags = flags;
            let mut expected = r.cpu;
            expected.set_al(status);
            expected.ip += 2;
            r.step();
            assert_eq!(r.state(), &GuestState::Running);
            assert_eq!(r.cpu, expected);
            assert_eq!(r.memory.vga_addressing_enabled(), enabled);

            let mut bytes = [0; 2];
            r.memory.read_slice(0xb7ff, 15, &mut bytes).unwrap();
            assert_eq!(bytes, [0x55, if enabled { b'A' } else { 0xff }]);
            if !enabled {
                r.memory.write_u16(0xb800, 0, 0x2342);
                r.memory.write_slice(0xa000, 0, &[9]).unwrap();
                assert_eq!(r.memory.read_u16(0xb800, 0), 0xffff);
                assert_eq!(r.memory.read_u8(0xa000, 0), 0xff);
                assert!(!r.memory.range_is_ram(0xb8000, 2));
            } else {
                assert_eq!(r.memory.read_u16(0xb800, 0), 0x8f41);
                assert_eq!(r.memory.read_u8(0xa000, 0), 4);
            }

            // OUT DX,AX programs the write index and red component.
            // Finish a complete RGB triplet; a disconnected DAC must
            // ignore the entire sequence. Use the default RGB on enable.
            let entry = r.palette_entries()[4];
            r.cpu.dx = 0x3c8;
            r.cpu.ax = u16::from_le_bytes([4, if enabled { entry.red_6bit } else { 1 }]);
            r.step();
            r.cpu.dx = 0x3c9;
            r.cpu.set_al(if enabled { entry.green_6bit } else { 2 });
            r.step();
            r.cpu.set_al(if enabled { entry.blue_6bit } else { 3 });
            r.step();
            r.cpu.dx = 0x3c7;
            r.cpu.set_al(4);
            r.step();
            r.cpu.dx = 0x3c9;
            r.step(); // DAC data read
            assert_eq!(
                r.cpu.al(),
                if enabled {
                    r.palette_entries()[4].red_6bit
                } else {
                    0xff
                }
            );
            r.step(); // system control port B remains accessible
            assert_ne!(r.cpu.al(), 0xff);
            assert_eq!(r.cell(0, 0), cell);
            assert_eq!(r.video_mode(), mode);
            assert_eq!(r.text_attribute_mode(), attr);
            assert_eq!(r.framebuffer_index(0, 0), Some(4));
            assert_eq!(r.framebuffer_generation(), generation);
            assert_eq!(r.palette_checksum(), palette);
        }
    }

    #[test]
    fn vga_decoder_service_respects_guest_int10_hook() {
        let mut r = runtime_for(&[0xcd, 0x10, 0xf4]);
        r.memory.write_u16(0, 0x40, 0x20);
        r.memory.write_u16(0, 0x42, 0x2000);
        r.memory
            .write_slice(0x2000, 0x20, &[0xb8, 0x34, 0x12, 0xcf])
            .unwrap();
        r.cpu.ax = 0x1201;
        r.cpu.bx = 0x0032;
        r.cpu.flags = 0x0203;
        r.run_slice(4);
        assert_eq!(r.cpu.ax, 0x1234);
        assert_eq!(r.cpu.flags, 0x0203);
        assert!(r.memory.vga_addressing_enabled());
        assert_eq!(r.state(), &GuestState::Halted);
    }

    #[test]
    fn unsupported_video_requests_trap_without_changing_display() {
        for (ax, bx) in [
            (0x1200, 0x11),
            (0x12ff, 0xff),
            (0x1a00, 0),
            (0x0501, 0),
            (0x0300, 0x100),
            (0x0800, 0x100),
            (0x0900, 0x100),
            (0x0200, 0x100),
        ] {
            let mut r = runtime_for(&[0xcd, 0x10]);
            r.cpu.ax = ax;
            r.cpu.bx = bx;
            let generation = r.video_mode_generation();
            r.step();
            assert!(matches!(
                r.state(),
                GuestState::Trapped(Trap::UnsupportedInterrupt {
                    interrupt: 0x10,
                    ..
                })
            ));
            assert_eq!(r.video_mode_generation(), generation);
            assert_eq!(r.memory.read_u8(0x40, 0x62), 0);
            assert_eq!(r.cell(0, 0).character, b' ');
        }
    }

    fn video_bytes(runtime: &Runtime) -> Vec<u8> {
        (0..crate::VIDEO_BYTES)
            .map(|offset| runtime.memory.read_u8(0xb800, offset as u16))
            .collect()
    }

    #[test]
    fn blink_intensity_service_dispatches_through_guest_int_and_preserves_state() {
        // Authentic DN-9 input: AX=1003h, BL=00h, BH=00h.
        let mut r = runtime_for(&[0xcd, 0x10, 0xcd, 0x10]);
        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0000;
        r.cpu.cx = 0x1234;
        r.cpu.dx = 0x5678;
        r.cpu.si = 0x9abc;
        r.cpu.di = 0xdef0;
        r.cpu.bp = 0x1357;
        r.cpu.flags = 0x0ed7;
        let before = r.cpu;
        r.step();
        let mut expected = before;
        expected.ip += 2;
        assert_eq!(r.cpu, expected);
        assert_eq!(r.state(), &GuestState::Running);
        assert_eq!(
            r.text_attribute_mode(),
            TextAttributeMode::BackgroundIntensity
        );

        // BL=01h selects blinking again through a second guest INT 10h.
        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0001;
        let before = r.cpu;
        r.step();
        let mut expected = before;
        expected.ip += 2;
        assert_eq!(r.cpu, expected);
        assert_eq!(r.text_attribute_mode(), TextAttributeMode::Blink);
    }

    #[test]
    fn documented_bh_is_accepted_and_other_bl_values_leave_state_unchanged() {
        for bh in [0x00, 0xaa, 0xff] {
            let mut r = runtime_for(&[0xcd, 0x10]);
            r.cpu.ax = 0x1003;
            r.cpu.bx = (bh << 8) | 0x00;
            let before = r.cpu;
            r.step();
            let mut expected = before;
            expected.ip += 2;
            assert_eq!(r.cpu, expected);
            assert_eq!(
                r.text_attribute_mode(),
                TextAttributeMode::BackgroundIntensity
            );
        }

        for bl in [0x02, 0x7f, 0xff] {
            let mut r = runtime_for(&[0xcd, 0x10]);
            let generation = r.text_attribute_generation();
            r.cpu.ax = 0x1003;
            r.cpu.bx = 0x0000 | bl;
            let before = r.cpu;
            r.step();
            let mut expected = before;
            expected.ip += 2;
            assert_eq!(r.cpu, expected);
            assert_eq!(r.state(), &GuestState::Running);
            assert_eq!(r.text_attribute_mode(), TextAttributeMode::Blink);
            assert_eq!(r.text_attribute_generation(), generation);
        }
    }

    #[test]
    fn mode_switch_changes_interpretation_without_rewriting_b8000_bytes() {
        let mut r = runtime_for(&[0xcd, 0x10]);
        r.put_cell(0, 0, b'A', 0x8f);
        r.put_cell(1, 0, b'B', 0x4e);
        let before = video_bytes(&r);
        let blinking = r.text_attribute(0x8f);
        assert_eq!(blinking.background, 0x00);
        assert!(blinking.blink);

        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0000;
        r.step();

        assert_eq!(video_bytes(&r), before);
        assert_eq!(r.cell(0, 0).character, b'A');
        assert_eq!(r.cell(0, 0).attribute, 0x8f);
        let intense = r.text_attribute(0x8f);
        assert_eq!(intense.foreground, 0x0f);
        assert_eq!(intense.background, 0x08);
        assert!(!intense.blink);
        let unchanged = r.text_attribute(0x4e);
        assert_eq!(unchanged.foreground, 0x0e);
        assert_eq!(unchanged.background, 0x04);
    }

    #[test]
    fn bios_and_direct_guest_writes_use_the_shared_attribute_mode() {
        // BIOS AH=09h writes the raw attribute unchanged.
        let mut r = runtime_for(&[0xcd, 0x10]);
        r.cpu.ax = 0x095a;
        r.cpu.bx = 0x008f;
        r.cpu.cx = 1;
        r.step();
        assert_eq!(r.cell(0, 0).attribute, 0x8f);
        r.cpu.ip = 0x100;
        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0000;
        r.step();
        assert_eq!(r.cell(0, 0).attribute, 0x8f);
        assert_eq!(r.text_attribute(r.cell(0, 0).attribute).background, 0x08);

        // Direct guest video-memory writes through ES:B800 are equally raw.
        let mut r = runtime_for(&[
            0xb8, 0x00, 0xb8, // MOV AX,B800h
            0x8e, 0xc0, // MOV ES,AX
            0x26, 0xc6, 0x06, 0x01, 0x00, 0x8f, // MOV byte ES:[0001],8Fh
        ]);
        r.run_slice(4);
        assert_eq!(r.memory.read_u8(0xb800, 1), 0x8f);
        r.cpu.ip = 0x100;
        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0001;
        r.step();
        assert_eq!(r.memory.read_u8(0xb800, 1), 0x8f);
        assert_eq!(r.text_attribute(0x8f).background, 0x00);
        assert!(r.text_attribute(0x8f).blink);
    }

    #[test]
    fn attribute_mode_switch_reports_a_slice_change_and_keeps_page_and_cursor() {
        let mut r = runtime_for(&[0xcd, 0x10]);
        r.set_cursor(7, 3);
        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0000;
        let generation = r.text_attribute_generation();
        assert!(r.run_slice(1));
        assert_eq!(r.text_attribute_generation(), generation + 1);
        assert_eq!(r.memory.read_u8(0x40, 0x62), 0);
        assert_eq!((r.cursor_column(), r.cursor_row()), (7, 3));
        assert_eq!(r.memory.read_u16(0x40, 0x50), 0x0307);
        // A runnable guest that does not touch the mode reports no change.
        r.cpu.ip = 0x100;
        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0000;
        assert!(!r.run_slice(1));
    }

    #[test]
    fn video_mode_set_restores_the_blink_default_and_blink_scan_follows_mode() {
        let mut r = runtime_for(&[0xcd, 0x10, 0xcd, 0x10]);
        r.cpu.ax = 0x1003;
        r.cpu.bx = 0x0000;
        r.step();
        assert_eq!(
            r.text_attribute_mode(),
            TextAttributeMode::BackgroundIntensity
        );
        r.put_cell(0, 0, b'X', 0x8f);
        assert!(!r.has_blink_attributes());

        r.cpu.ip = 0x100;
        r.cpu.ax = 0x0003;
        r.step();
        assert_eq!(r.text_attribute_mode(), TextAttributeMode::Blink);
        assert_eq!(r.cell(0, 0).attribute, DEFAULT_ATTRIBUTE);
        assert!(!r.has_blink_attributes());

        r.put_cell(0, 0, b'X', 0x8f);
        assert!(r.has_blink_attributes());
        r.put_cell(0, 0, b'X', 0x7f);
        assert!(!r.has_blink_attributes());
    }

    #[test]
    fn bios_scroll_down_copies_backwards_and_clips_to_window() {
        let mut r = runtime_for(&[0xcd, 0x10]);
        for row in 0..4 {
            r.put_cell(1, row, b'A' + row as u8, 7);
        }
        r.cpu.ax = 0x0701;
        r.cpu.bx = 0x1f00;
        r.cpu.cx = 0x0101;
        r.cpu.dx = 0x0301;
        r.step();
        assert_eq!(r.cell(1, 0).character, b'A');
        assert_eq!(
            r.cell(1, 1),
            crate::TextCell {
                character: b' ',
                attribute: 0x1f
            }
        );
        assert_eq!(r.cell(1, 2).character, b'B');
        assert_eq!(r.cell(1, 3).character, b'C');
        r.cpu.ip = 0x100;
        r.cpu.ax = 0x0700;
        r.step();
        assert_eq!(r.cell(1, 3).character, b' ');
        r.cpu.ip = 0x100;
        r.cpu.cx = 0x0401;
        r.step();
        assert_eq!(r.state(), &GuestState::Trapped(Trap::InvalidVideoRectangle));
    }

    #[test]
    fn bios_data_area_and_video_calls_share_the_b8000_model() {
        let mut runtime = runtime_for(&[]);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x03);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4a), 80);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4c), 0x1000);

        runtime.cpu.set_ah(0x09);
        runtime.cpu.set_al(b'Z');
        runtime.cpu.set_bl(0x4e);
        runtime.cpu.cx = 3;
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.cell(0, 0).character, b'Z');
        assert_eq!(runtime.cell(2, 0).attribute, 0x4e);

        runtime.cpu.set_ah(0x06);
        runtime.cpu.set_al(0);
        runtime.cpu.set_bh(0x1f);
        runtime.cpu.set_ch(0);
        runtime.cpu.set_cl(0);
        runtime.cpu.set_dh(0);
        runtime.cpu.set_dl(2);
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.cell(0, 0).character, b' ');
        assert_eq!(runtime.cell(1, 0).attribute, 0x1f);

        runtime.cpu.set_ah(0x0f);
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.cpu.ax, 0x5003);
    }

    #[test]
    fn teletype_places_characters_before_advancing_the_cursor() {
        let mut runtime = runtime_for(&[]);
        runtime.set_cursor(0, 0);

        runtime.teletype(b'A', 0x1f);
        runtime.teletype(b'B', 0x1f);

        assert_eq!(runtime.cell(0, 0).character, b'A');
        assert_eq!(runtime.cell(1, 0).character, b'B');
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (2, 0));
    }

    #[test]
    fn teletype_cr_lf_and_backspace_keep_cursor_and_cells_in_sync() {
        let mut runtime = runtime_for(&[]);
        runtime.set_cursor(5, 3);

        runtime.teletype(b'\r', 0x07);
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (0, 3));
        runtime.teletype(b'\n', 0x07);
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (0, 4));

        runtime.teletype(b'X', 0x1e);
        runtime.teletype(b'Y', 0x1e);
        runtime.teletype(0x08, 0x1e);
        assert_eq!(runtime.cell(0, 4).character, b'X');
        assert_eq!(runtime.cell(1, 4).character, b' ');
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (1, 4));
    }

    #[test]
    fn teletype_wraps_only_after_writing_the_rightmost_cell() {
        let mut runtime = runtime_for(&[]);
        runtime.set_cursor(79, 0);

        runtime.teletype(b'Z', 0x2f);
        assert_eq!(runtime.cell(79, 0).character, b'Z');
        assert_eq!(runtime.cell(79, 0).attribute, 0x2f);
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (0, 1));

        runtime.teletype(b'N', 0x2f);
        assert_eq!(runtime.cell(0, 1).character, b'N');
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (1, 1));
    }

    #[test]
    fn teletype_scrolls_after_the_bottom_right_cell() {
        let mut runtime = runtime_for(&[]);
        runtime.put_cell(0, 1, b'K', 0x07);
        runtime.set_cursor(79, 24);

        runtime.teletype(b'Z', 0x07);

        assert_eq!(runtime.cell(0, 0).character, b'K');
        assert_eq!(runtime.cell(79, 23).character, b'Z');
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (0, 24));
    }

    #[test]
    fn test_and_xchg_follow_guest_visible_register_and_flag_rules() {
        let mut runtime = runtime_for(&[0xb8, 0xf0, 0x00, 0xbb, 0x0f, 0x00, 0x85, 0xd8, 0x93]);
        runtime.run_slice(4);
        assert!(runtime.cpu.flags & CpuState::FLAG_ZF != 0);
        assert_eq!(runtime.cpu.ax, 0x000f);
        assert_eq!(runtime.cpu.bx, 0x00f0);
    }

    #[test]
    fn keyboard_translation_and_blocking_bios_input_work() {
        assert_eq!(
            translate_key_press(HostKeyEvent {
                keycode: 0x48,
                extended: true,
                pressed: true,
                shift: false,
                ctrl: false,
                alt: false
            }),
            Some(BiosKey {
                ascii: 0xe0,
                scan_code: 0x48
            })
        );
        let mut runtime = runtime_for(&[0xb4, 0x00, 0xcd, 0x16]);
        runtime.run_slice(4);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert!(!runtime.run_slice(100));
        runtime.inject_key(BiosKey {
            ascii: b'Z',
            scan_code: 0x2c,
        });
        runtime.run_slice(1);
        assert_eq!(runtime.cpu.ax, 0x2c5a);
    }

    #[test]
    fn dos_buffered_input_edits_and_completes_without_spinning() {
        let mut runtime = runtime_for(&[0xba, 0x20, 0x01, 0xb4, 0x0a, 0xcd, 0x21]);
        runtime.memory.write_u8(PSP_SEGMENT, 0x0120, 8);
        runtime.run_slice(3);
        for ascii in [b'H', b'i', 0x08, b'!', b'\r'] {
            runtime.inject_ascii(ascii);
        }
        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0x0121), 2);
        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0x0122), b'H');
        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0x0123), b'!');
        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0x0124), b'\r');
    }

    #[test]
    fn interactive_guest_scripted_input_uses_guest_keyboard_and_exits() {
        let mut runtime = runtime_for(CHRONOS_INTERACTIVE_COM);
        runtime.run_slice(1024);
        let title: [u8; 27] = core::array::from_fn(|column| runtime.cell(column, 0).character);
        assert_eq!(&title, b"Chronos Interactive Console");
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        for ascii in [b'H', b'e', b'l', b'l', b'o', 0x08, b'!', b'\r', 0x1b] {
            runtime.inject_ascii(ascii);
            runtime.run_slice(64);
        }
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
        let mut echoed = None;
        for row in 0..25 {
            for column in 0..75 {
                if runtime.cell(column, row).character == b'H'
                    && runtime.cell(column + 1, row).character == b'e'
                    && runtime.cell(column + 2, row).character == b'l'
                    && runtime.cell(column + 3, row).character == b'l'
                    && runtime.cell(column + 4, row).character == b'!'
                {
                    echoed = Some((column, row));
                }
            }
        }
        assert!(echoed.is_some());
    }

    fn mz_program(image: &[u8]) -> Vec<u8> {
        let logical_size = 32 + image.len();
        assert!(logical_size <= 512);
        let mut program = vec![0u8; logical_size];
        program[..2].copy_from_slice(b"MZ");
        program[2..4].copy_from_slice(&(logical_size as u16).to_le_bytes());
        program[4..6].copy_from_slice(&1u16.to_le_bytes());
        program[8..10].copy_from_slice(&2u16.to_le_bytes());
        program[0x18..0x1a].copy_from_slice(&0x1cu16.to_le_bytes());
        program[12..14].copy_from_slice(&1u16.to_le_bytes());
        program[32..].copy_from_slice(image);
        program
    }

    #[test]
    fn far_control_flow_and_iret_restore_full_machine_state() {
        let mut runtime = runtime_for(&[0x9a, 0x00, 0x02, 0x00, 0x20, 0xf4]);
        runtime
            .memory
            .write_slice(0x2000, 0x0200, &[0xb8, 0x34, 0x12, 0xcb])
            .unwrap();
        runtime.run_slice(4);
        assert_eq!(runtime.cpu.ax, 0x1234);
        assert_eq!(runtime.cpu.cs, PSP_SEGMENT);

        let mut iret = runtime_for(&[0xcf]);
        iret.cpu.ss = PSP_SEGMENT;
        iret.cpu.sp = 0xfff8;
        iret.memory.write_u16(PSP_SEGMENT, 0xfff8, 0x0200);
        iret.memory.write_u16(PSP_SEGMENT, 0xfffa, 0x2222);
        iret.memory.write_u16(PSP_SEGMENT, 0xfffc, 0x0243);
        iret.step();
        assert_eq!(
            (iret.cpu.cs, iret.cpu.ip, iret.cpu.flags),
            (0x2222, 0x0200, 0x0243)
        );
    }

    #[test]
    fn mz_child_exec_restores_parent_and_records_exit_code() {
        let parent = mz_program(&[0x90, 0xf4]);
        let mut runtime = Runtime::from_program(&parent, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_file(
                crate::DosDrive::C,
                "CHILD.COM",
                vec![0xb8, 0x2a, 0x4c, 0xcd, 0x21],
            )
            .unwrap();
        let path = runtime.drives().parse_path(b"C:\\CHILD.COM").unwrap();
        let parent_psp = runtime.current_psp();
        runtime.exec(path, b"/from-parent", 0).unwrap();
        assert_ne!(runtime.current_psp(), parent_psp);
        runtime.run_slice(16);
        assert_eq!(runtime.current_psp(), parent_psp);
        assert_eq!(
            runtime.active_process.child_result,
            Some(super::ChildResult {
                code: 42,
                termination: super::TerminationType::Normal
            })
        );
    }

    #[test]
    fn final_dos_exit_releases_parent_after_child_recovery_and_retains_status() {
        for code in [0, 42] {
            let parent = mz_program(&[
                0xb4, 0x4d, 0xcd, 0x21, // get child exit status
                0xb4, 0x00, 0xcd, 0x16, // leave the session open
                0xb8, code, 0x4c, 0xcd, 0x21, // terminate the last process
            ]);
            let mut runtime = Runtime::from_program(&parent, b"").unwrap();
            runtime
                .drives_mut()
                .add_base_file(DosDrive::C, "CHILD.COM", vec![0xb8, 7, 0x4c, 0xcd, 0x21])
                .unwrap();
            let path = runtime.drives().parse_path(b"C:\\CHILD.COM").unwrap();
            let parent_psp = runtime.current_psp();
            runtime.exec(path, b"", 0).unwrap();
            let child_psp = runtime.current_psp();
            assert_ne!(parent_psp, child_psp);
            runtime.run_slice(32);
            assert_eq!(runtime.current_psp(), parent_psp);
            assert!(runtime.parent_process.is_none());
            assert_eq!(*runtime.state(), GuestState::WaitingForInput);
            assert_eq!(runtime.last_delivered_child_result().unwrap().code, 7);
            assert!(runtime
                .arena
                .blocks()
                .iter()
                .any(|block| block.owner_psp == Some(parent_psp)));
            assert!(runtime
                .arena
                .blocks()
                .iter()
                .all(|block| block.owner_psp != Some(child_psp)));

            runtime.inject_ascii(b'x');
            runtime.run_slice(32);
            assert_eq!(*runtime.state(), GuestState::Exited { code });
            assert!(runtime
                .arena
                .blocks()
                .iter()
                .all(|block| block.owner_psp.is_none()));
            let retired = runtime.instructions_retired();
            runtime.run_slice(32);
            assert_eq!(runtime.instructions_retired(), retired);
            assert_eq!(*runtime.state(), GuestState::Exited { code });
        }
    }

    #[test]
    fn multiply_divide_shifts_and_profile_gates_are_guest_visible() {
        let mut multiply = runtime_for(&[0xb0, 0x06, 0xb3, 0x07, 0xf6, 0xe3]);
        multiply.run_slice(3);
        assert_eq!(multiply.cpu.ax, 42);

        let mut divide = runtime_for(&[0xb8, 0x2a, 0x00, 0xb3, 0x05, 0xf6, 0xf3]);
        divide.run_slice(3);
        assert_eq!((divide.cpu.al(), divide.cpu.ah()), (8, 2));

        let mut shift = runtime_for(&[0xb0, 0x81, 0xd0, 0xe0]);
        shift.run_slice(2);
        assert_eq!(shift.cpu.al(), 2);

        let mut logical_right = runtime_for(&[0xb9, 0x46, 0x02, 0xd1, 0xe9]);
        logical_right.run_slice(2);
        assert_eq!(logical_right.cpu.cx, 0x0123);

        let mut gated = runtime_for(&[0x68, 0x34, 0x12]);
        gated.step();
        assert!(matches!(
            gated.state(),
            GuestState::Trapped(super::Trap::CpuProfileViolation { .. })
        ));
        gated.set_cpu_profile(super::CpuProfile::I80186);
        let mut allowed = runtime_for(&[0x68, 0x34, 0x12, 0x58]);
        allowed.set_cpu_profile(super::CpuProfile::I80186);
        allowed.run_slice(2);
        assert_eq!(allowed.cpu.ax, 0x1234);
    }

    #[test]
    fn bios_mode_switching_updates_internal_bda_and_cursor_state() {
        let mut runtime = runtime_for(&[]);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x03);
        assert!(runtime.text_cursor_visible());

        runtime.cpu.ax = 0x0013;
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x13);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4a), 40);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4c), 0xfa00);
        assert_eq!(runtime.memory.read_u8(0x40, 0x62), 0);
        assert_eq!(runtime.memory.read_u16(0x40, 0x60), 0x2000);
        assert!(!runtime.text_cursor_visible());
        assert!(runtime
            .take_graphics_dirty_rows()
            .iter()
            .all(|dirty| *dirty));

        runtime.cpu.set_ah(0x0f);
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.cpu.ax, 0x2813);

        runtime.cpu.ax = 0x0003;
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x03);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4a), 80);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4c), 0x1000);
        assert_eq!(runtime.cursor_shape(), 0x0607);
        assert!(runtime.text_cursor_visible());
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (0, 0));

        runtime.cpu.set_ah(0x01);
        runtime.cpu.cx = 0x2607;
        super::dos::dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.cursor_shape(), 0x2607);
        assert_eq!(runtime.memory.read_u16(0x40, 0x60), 0x2607);
        assert!(!runtime.text_cursor_visible());
    }

    #[test]
    fn text_mode_mouse_delivery_uses_guest_selected_ranges() {
        let mut runtime = runtime_for(&[0xf4]);
        runtime.cpu.ax = 0;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        runtime.cpu.ax = 1;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        runtime.cpu.ax = 7;
        runtime.cpu.cx = 0;
        runtime.cpu.dx = 79;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        runtime.cpu.ax = 8;
        runtime.cpu.cx = 0;
        runtime.cpu.dx = 24;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();

        let viewport = MouseViewport::new(10, 20, 800, 400);
        assert!(runtime.inject_mouse_motion(viewport, 410, 220));
        assert_eq!(runtime.mouse().position(), (40, 12));
        assert!(runtime.inject_mouse_button(viewport, 410, 220, 0, true));
        assert_eq!(runtime.mouse().buttons().bits(), 1);
        assert!(runtime.inject_mouse_button(viewport, 410, 220, 0, false));
        assert_eq!(runtime.mouse().buttons().bits(), 0);
    }

    #[test]
    fn int33_core_functions_report_validate_clamp_and_remain_per_runtime() {
        let mut runtime = runtime_for(&[0xf4]);
        runtime.cpu.ax = 0;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert_eq!((runtime.cpu.ax, runtime.cpu.bx), (0xffff, 3));
        assert_eq!(runtime.mouse().ranges(), (0, 639, 0, 199));
        assert_eq!(runtime.mouse().position(), (319, 99));
        assert!(!runtime.mouse().cursor_visible());

        runtime.cpu.ax = 1;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        runtime.cpu.ax = 1;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert_eq!(runtime.mouse().visibility_counter(), 1);
        runtime.cpu.ax = 2;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert!(runtime.mouse().cursor_visible());
        runtime.cpu.ax = 2;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert!(!runtime.mouse().cursor_visible());

        runtime.cpu.ax = 7;
        runtime.cpu.cx = 0;
        runtime.cpu.dx = 319;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        runtime.cpu.ax = 8;
        runtime.cpu.cx = 0;
        runtime.cpu.dx = 199;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        runtime.cpu.ax = 4;
        runtime.cpu.cx = u16::MAX;
        runtime.cpu.dx = u16::MAX;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        runtime.cpu.ax = 3;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert_eq!(
            (runtime.cpu.bx, runtime.cpu.cx, runtime.cpu.dx),
            (0, 319, 199)
        );
        assert_eq!(runtime.mouse().int33_state_query_count(), 1);
        assert_eq!(runtime.mouse().last_state_query(), (0, 319, 199));

        runtime.cpu.ax = 7;
        runtime.cpu.cx = 200;
        runtime.cpu.dx = 100;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(
            runtime.cpu.ax,
            crate::mouse::DOS_MOUSE_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(runtime.mouse().ranges(), (0, 319, 0, 199));

        runtime.cpu.ax = 0x0005;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(runtime.cpu.ax, crate::mouse::DOS_MOUSE_ERROR_UNSUPPORTED);
        runtime.cpu.ax = 0x0006;
        super::dos::dispatch(&mut runtime, 0x33).unwrap();
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(runtime.cpu.ax, crate::mouse::DOS_MOUSE_ERROR_UNSUPPORTED);
        assert_eq!(runtime.mouse().int33_function_count(0), 1);
        assert_eq!(runtime.mouse().int33_function_count(1), 2);
        assert_eq!(runtime.mouse().int33_function_count(2), 2);
        assert_eq!(runtime.mouse().int33_function_count(3), 1);
        assert_eq!(runtime.mouse().int33_function_count(4), 1);
        assert_eq!(runtime.mouse().int33_function_count(5), 1);
        assert_eq!(runtime.mouse().int33_function_count(6), 1);
        assert_eq!(runtime.mouse().int33_function_count(7), 2);
        assert_eq!(runtime.mouse().int33_function_count(8), 1);
        assert_eq!(runtime.mouse().int33_function_count(9), 0);

        let mut other = runtime_for(&[0xf4]);
        other.cpu.ax = 0;
        super::dos::dispatch(&mut other, 0x33).unwrap();
        assert_eq!(runtime.mouse().ranges(), (0, 319, 0, 199));
        assert_eq!(other.mouse().ranges(), (0, 639, 0, 199));

        other.set_mouse_enabled(false);
        other.cpu.ax = 0;
        super::dos::dispatch(&mut other, 0x33).unwrap();
        assert_eq!((other.cpu.ax, other.cpu.bx), (0, 0));
    }

    #[test]
    fn int28_yields_the_slice_without_clobbering_registers() {
        let mut runtime = runtime_for(&[
            0xb8, 0x34, 0x12, 0xbb, 0x78, 0x56, 0xb9, 0xbc, 0x9a, 0xba, 0xf0, 0xde, 0xcd, 0x28,
            0xf4,
        ]);
        runtime.run_slice(100);
        assert_eq!(runtime.state(), &GuestState::YieldedUntilTimer);
        assert!(runtime.cooperative_yielded_last_slice());
        assert_eq!(runtime.cooperative_yield_count(), 1);
        assert_eq!((runtime.cpu.ax, runtime.cpu.bx), (0x1234, 0x5678));
        assert_eq!((runtime.cpu.cx, runtime.cpu.dx), (0x9abc, 0xdef0));
        assert!(runtime.wake_from_timer());
        runtime.run_slice(1);
        assert_eq!(runtime.state(), &GuestState::Halted);
    }

    #[test]
    fn sunpaint_mouse_unavailable_path_restores_text_and_exits_nonzero() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        runtime.set_mouse_enabled(false);
        runtime.run_slice(10_000);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.state(), &GuestState::Exited { code: 1 });
        let text: [u8; 27] = core::array::from_fn(|index| runtime.cell(index, 0).character);
        assert_eq!(&text, b"SUNPAINT: mouse unavailable");
    }

    #[test]
    fn sunpaint_real_guest_paints_erases_selects_clears_and_exits() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        run_to_next_idle_hint(&mut runtime);
        let viewport = MouseViewport::new(0, 0, 320, 200);

        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.mouse().ranges(), (0, 319, 0, 199));
        assert_eq!(runtime.mouse().position(), (160, 100));
        assert!(runtime.mouse().cursor_visible());
        assert!(runtime.cooperative_yielded_last_slice());
        assert_eq!(runtime.framebuffer_index(0, 0), Some(4));
        assert_eq!(runtime.framebuffer_index(40, 0), Some(6));
        assert_eq!(runtime.framebuffer_index(80, 0), Some(10));
        assert_eq!(runtime.framebuffer_index(0, 16), Some(0));
        let initial_checksum = runtime.framebuffer_checksum();
        assert_eq!(initial_checksum, 0x0fe7_2cec_a639_10da);
        let palette_checksum = runtime.palette_checksum();

        runtime.inject_mouse_motion(viewport, 20, 30);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 20, 30, 0, true);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_motion(viewport, 50, 45);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 50, 45, 0, false);
        run_to_next_idle_hint(&mut runtime);
        for x in 20..=50 {
            let y = 30 + (x - 20) / 2;
            assert!(
                runtime.framebuffer_index(x, y) == Some(12)
                    || runtime.framebuffer_index(x, y + 1) == Some(12),
                "guest Bresenham stroke has a gap near ({x},{y})"
            );
        }
        let stroke_checksum = runtime.framebuffer_checksum();
        assert_eq!(stroke_checksum, 0x6c02_e1e3_952e_ed46);
        assert_ne!(stroke_checksum, initial_checksum);
        assert_eq!(runtime.mouse().buttons().bits(), 0);
        assert_eq!(runtime.palette_checksum(), palette_checksum);

        runtime.inject_mouse_button(viewport, 40, 40, 1, true);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_motion(viewport, 45, 42);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 45, 42, 1, false);
        run_to_next_idle_hint(&mut runtime);
        assert_eq!(runtime.framebuffer_index(40, 40), Some(0));

        runtime.inject_mouse_button(viewport, 100, 5, 0, true);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 100, 5, 0, false);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_motion(viewport, 70, 70);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 70, 70, 0, true);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 70, 70, 0, false);
        run_to_next_idle_hint(&mut runtime);
        assert_eq!(runtime.framebuffer_index(70, 70), Some(10));

        runtime.inject_ascii(b'3');
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_motion(viewport, 80, 80);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 80, 80, 0, true);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 80, 80, 0, false);
        run_to_next_idle_hint(&mut runtime);
        assert_eq!(runtime.framebuffer_index(80, 80), Some(10));

        runtime.inject_ascii(b'C');
        run_to_next_idle_hint(&mut runtime);
        let clear_checksum = runtime.framebuffer_checksum();
        assert_eq!(clear_checksum, 0x74f1_cf81_d4ed_f3ec);
        for y in 16..200 {
            for x in 0..320 {
                if x >= 308 && y >= 188 {
                    continue;
                }
                assert_eq!(runtime.framebuffer_index(x, y), Some(0));
            }
        }
        assert_eq!(runtime.framebuffer_index(80, 0), Some(10));
        assert_eq!(runtime.framebuffer_index(80, 80), Some(0));

        runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        runtime.run_slice(100_000);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
        assert!(!runtime.mouse().cursor_visible());
        assert!(!runtime.mouse().captured());
    }

    #[test]
    fn sunpaint_focus_loss_and_high_frequency_motion_never_stick_input() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        run_to_next_idle_hint(&mut runtime);
        let viewport = MouseViewport::new(0, 0, 320, 200);

        runtime.inject_mouse_motion(viewport, 30, 30);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 30, 30, 0, true);
        run_to_next_idle_hint(&mut runtime);
        assert_eq!(runtime.mouse().buttons().bits(), 1);
        runtime.mouse_focus_changed(false);
        assert_eq!(runtime.mouse().buttons().bits(), 0);
        assert!(!runtime.mouse().captured());
        assert!(!runtime.inject_mouse_button(viewport, 300, 180, 0, false));
        runtime.mouse_focus_changed(true);
        run_to_next_idle_hint(&mut runtime);
        let generation = runtime.framebuffer_generation();
        runtime.inject_mouse_motion(viewport, 60, 60);
        run_to_next_idle_hint(&mut runtime);
        assert_eq!(runtime.framebuffer_generation(), generation);

        let mouse_generation = runtime.mouse().generation();
        runtime.inject_mouse_button(viewport, 40, 40, 0, true);
        for index in 0..5_000i32 {
            runtime.inject_mouse_motion(
                viewport,
                40 + index.rem_euclid(260),
                20 + (index * 7).rem_euclid(170),
            );
        }
        assert!(runtime.mouse().generation() > mouse_generation);
        assert!(runtime.mouse().captured());
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 319, 199, 0, false);
        run_to_next_idle_hint(&mut runtime);
        assert_eq!(runtime.mouse().position(), (319, 199));
        assert_eq!(runtime.mouse().buttons().bits(), 0);
        assert!(!runtime.mouse().captured());
        assert!(runtime.cooperative_yielded_last_slice());
    }

    #[test]
    fn mouse_graphics_child_trap_releases_capture_cursor_and_restores_one_shell_prompt() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut runtime = Runtime::from_program(shell, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_file(
                DosDrive::C,
                "BADMOUSE.COM",
                vec![
                    0xb8, 0x13, 0x00, 0xcd, 0x10, // graphics
                    0x31, 0xc0, 0xcd, 0x33, // reset mouse
                    0xb8, 0x01, 0x00, 0xcd, 0x33, // show
                    0xcd, 0x28, // yield so native capture can begin
                    0x0f, // unsupported opcode
                ],
            )
            .unwrap();
        runtime.run_slice(10_000_000);
        let shell_psp = runtime.current_psp();
        for ascii in b"BADMOUSE\r" {
            runtime.inject_ascii(*ascii);
            runtime.run_slice(10_000_000);
        }
        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert!(runtime.mouse().cursor_visible());
        runtime.inject_mouse_button(MouseViewport::new(0, 0, 320, 200), 50, 50, 0, true);
        assert!(runtime.mouse().captured());
        runtime.run_slice(1);

        assert_eq!(runtime.current_psp(), shell_psp);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.mouse().buttons().bits(), 0);
        assert!(!runtime.mouse().captured());
        assert!(!runtime.mouse().cursor_visible());
        assert!(matches!(
            runtime.take_recovered_child_trap(),
            Some(Trap::UnsupportedOpcode { .. })
        ));
        runtime.run_slice(10_000_000);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        let text: [u8; 2000] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert_eq!(
            text.windows(b"CMD C:\\>".len())
                .filter(|window| *window == b"CMD C:\\>")
                .count(),
            1
        );
    }

    #[test]
    fn shell_runs_real_sunpaint_stroke_then_escape_and_dir_with_one_restored_prompt() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let sunpaint = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM");
        let mut runtime = Runtime::from_program(shell, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_directory(DosDrive::C, "TESTS")
            .unwrap();
        runtime
            .drives_mut()
            .add_base_file(DosDrive::C, "TESTS/SUNPAINT.COM", sunpaint.to_vec())
            .unwrap();
        runtime.run_slice(10_000_000);
        let shell_psp = runtime.current_psp();

        for ascii in b"SUNPAINT\r" {
            runtime.inject_ascii(*ascii);
            runtime.run_slice(10_000_000);
        }
        run_to_next_idle_hint(&mut runtime);
        let child_psp = runtime.current_psp();
        assert_ne!(child_psp, shell_psp);
        let viewport = MouseViewport::new(0, 0, 320, 200);
        runtime.inject_mouse_motion(viewport, 25, 25);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 25, 25, 0, true);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_motion(viewport, 45, 35);
        run_to_next_idle_hint(&mut runtime);
        runtime.inject_mouse_button(viewport, 45, 35, 0, false);
        run_to_next_idle_hint(&mut runtime);
        assert_eq!(runtime.framebuffer_index(25, 25), Some(12));
        assert_eq!(runtime.framebuffer_index(45, 35), Some(12));

        runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        runtime.run_slice(100_000);
        assert_eq!(runtime.current_psp(), shell_psp);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert!(runtime.parent_process.is_none());
        assert!(runtime
            .arena
            .blocks()
            .iter()
            .all(|block| block.owner_psp != Some(child_psp)));
        let text: [u8; 2000] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert_eq!(
            text.windows(b"CMD C:\\>".len())
                .filter(|window| *window == b"CMD C:\\>")
                .count(),
            1
        );

        for ascii in b"DIR\r" {
            runtime.inject_ascii(*ascii);
            runtime.run_slice(10_000_000);
        }
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        let text: [u8; 2000] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert_eq!(
            text.windows(b"CMD C:\\>".len())
                .filter(|window| *window == b"CMD C:\\>")
                .count(),
            2,
            "DIR must complete and return exactly its next shell prompt"
        );
    }

    #[test]
    fn sunpaint_bounded_interaction_soak_has_no_stuck_state_or_arena_growth() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        run_to_next_idle_hint(&mut runtime);
        let viewport = MouseViewport::new(0, 0, 320, 200);
        let arena_blocks = runtime.arena.blocks().len();
        let framebuffer_len = runtime.memory.len();

        for cycle in 0..100i32 {
            let button = if cycle & 1 == 0 { 0 } else { 1 };
            let start_x = 10 + (cycle * 13).rem_euclid(280);
            let start_y = 20 + (cycle * 17).rem_euclid(160);
            runtime.inject_mouse_button(viewport, start_x, start_y, button, true);
            run_to_next_idle_hint(&mut runtime);
            for sample in 0..30i32 {
                runtime.inject_mouse_motion(
                    viewport,
                    (start_x + sample * 11).rem_euclid(320),
                    16 + (start_y + sample * 7).rem_euclid(184),
                );
            }
            run_to_next_idle_hint(&mut runtime);
            let (x, y) = runtime.mouse().position();
            runtime.inject_mouse_button(viewport, i32::from(x), i32::from(y), button, false);
            run_to_next_idle_hint(&mut runtime);
            if cycle % 10 == 0 {
                runtime.mouse_focus_changed(false);
                runtime.mouse_focus_changed(true);
            }
            if cycle % 20 == 0 {
                runtime.inject_ascii(b'C');
                run_to_next_idle_hint(&mut runtime);
            }
        }

        assert_eq!(runtime.memory.len(), framebuffer_len);
        assert_eq!(runtime.arena.blocks().len(), arena_blocks);
        assert_eq!(runtime.mouse().buttons().bits(), 0);
        assert!(!runtime.mouse().captured());
        assert!(runtime.cooperative_yield_count() > 200);
        runtime.inject_ascii(0x1b);
        runtime.run_slice(100_000);
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
    }

    #[test]
    fn unsupported_bios_video_modes_are_explicit_and_do_not_change_state() {
        let mut runtime = runtime_for(&[]);
        runtime.cpu.ax = 0x0012;
        assert_eq!(
            super::dos::dispatch(&mut runtime, 0x10),
            Err(Trap::UnsupportedVideoMode { mode: 0x12 })
        );
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x03);
    }

    #[test]
    fn rep_stosb_and_stosw_fill_authoritative_mode13_memory_cooperatively() {
        let mut byte_fill = runtime_for(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, 0xb8, 0x00, 0xa0, 0x8e, 0xc0, 0x31, 0xff, 0xb0, 0x2a,
            0xb9, 0x00, 0xfa, 0xfc, 0xf3, 0xaa, 0xb4, 0x00, 0xcd, 0x16,
        ]);
        byte_fill.run_slice(9);
        assert_eq!(byte_fill.cpu.cx, 0xf9ff, "REP must remain slice-resumable");
        byte_fill.run_slice(100_000);
        assert_eq!(byte_fill.state(), &GuestState::WaitingForInput);
        assert_eq!(byte_fill.framebuffer_index(0, 0), Some(0x2a));
        assert_eq!(byte_fill.framebuffer_index(319, 199), Some(0x2a));
        assert_eq!(byte_fill.framebuffer_generation(), 64_000);

        let mut word_fill = runtime_for(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, 0xb8, 0x00, 0xa0, 0x8e, 0xc0, 0x31, 0xff, 0xb8, 0x01,
            0x02, 0xb9, 0x00, 0x7d, 0xfc, 0xf3, 0xab, 0xb4, 0x00, 0xcd, 0x16,
        ]);
        word_fill.run_slice(100_000);
        assert_eq!(word_fill.state(), &GuestState::WaitingForInput);
        assert_eq!(word_fill.framebuffer_index(0, 0), Some(1));
        assert_eq!(word_fill.framebuffer_index(1, 0), Some(2));
        assert_eq!(word_fill.framebuffer_index(319, 199), Some(2));
    }

    #[test]
    fn vgalab_guest_generates_the_static_image_and_waits_without_spinning() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        runtime.run_slice(1_000_000);

        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(runtime.framebuffer_index(0, 0), Some(0));
        assert_eq!(runtime.framebuffer_index(0, 32), Some(16));
        assert_eq!(runtime.framebuffer_index(319, 32), Some(79));
        assert_eq!(runtime.framebuffer_index(24, 64), Some(11));
        assert_eq!(runtime.framebuffer_index(8, 56), Some(14));
        assert_eq!(runtime.framebuffer_index(16, 64), Some(12));
        assert_eq!(runtime.framebuffer_index(303, 64), Some(10));
        assert_eq!(runtime.framebuffer_index(140, 84), Some(6));
        assert_eq!(runtime.framebuffer_index(146, 90), Some(14));
        assert_eq!(runtime.framebuffer_index(152, 98), Some(15));
        assert_eq!(runtime.framebuffer_index(0, 199), Some(1));
        assert_eq!(runtime.framebuffer_checksum(), 0xe775_e5c1_2250_d879);

        let cpu = runtime.cpu;
        let generation = runtime.framebuffer_generation();
        assert!(!runtime.run_slice(1_000_000));
        assert_eq!(runtime.cpu, cpu);
        assert_eq!(runtime.framebuffer_generation(), generation);
    }

    #[test]
    fn vgalab_completes_static_drawing_under_small_execution_slices() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        let mut slices = 0usize;
        while matches!(runtime.state(), GuestState::Ready | GuestState::Running) {
            runtime.run_slice(128);
            slices += 1;
            assert!(slices < 2_000, "guest drawing exceeded bounded slice limit");
        }
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(slices, 831);
    }

    #[test]
    fn shell_command_path_runs_vgalab_and_escape_returns_one_clean_prompt() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let vgalab = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM");
        let mut runtime = Runtime::from_program(shell, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_directory(DosDrive::C, "TESTS")
            .unwrap();
        runtime
            .drives_mut()
            .add_base_file(DosDrive::C, "TESTS/VGALAB.COM", vgalab.to_vec())
            .unwrap();
        runtime.run_slice(10_000_000);
        let shell_psp = runtime.current_psp();

        for ascii in [b'V', b'G', b'A', b'L', b'A', b'B', b'\r'] {
            runtime.inject_ascii(ascii);
            runtime.run_slice(10_000_000);
        }
        let screen: [u8; 400] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert_ne!(
            runtime.current_psp(),
            shell_psp,
            "state={:?} screen={screen:?}",
            runtime.state()
        );
        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(runtime.framebuffer_checksum(), 0xe775_e5c1_2250_d879);

        runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        runtime.run_slice(10_000_000);
        assert_eq!(runtime.current_psp(), shell_psp);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(
            runtime.last_delivered_child_result(),
            Some(super::ChildResult {
                code: 0,
                termination: TerminationType::Normal,
            })
        );
        let text: [u8; 2000] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert_eq!(
            text.windows(b"CMD C:\\>".len())
                .filter(|window| *window == b"CMD C:\\>")
                .count(),
            1
        );
        runtime.inject_ascii(b'X');
        runtime.run_slice(10_000);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        runtime.inject_ascii(0x08);
        runtime.run_slice(10_000);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
    }

    #[test]
    fn trapped_graphics_child_restores_text_parent_generically() {
        let parent = mz_program(&[0xeb, 0xfe]);
        let mut runtime = Runtime::from_program(&parent, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_file(
                DosDrive::C,
                "BROKEN.COM",
                vec![0xb8, 0x13, 0x00, 0xcd, 0x10, 0x0f],
            )
            .unwrap();
        let path = runtime.drives().parse_path(b"C:\\BROKEN.COM").unwrap();
        let parent_psp = runtime.current_psp();
        runtime.exec(path, b"", 0).unwrap();
        runtime.run_slice(3);

        assert_eq!(runtime.current_psp(), parent_psp);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x03);
        assert_eq!(
            runtime.active_process.child_result,
            Some(super::ChildResult {
                code: 1,
                termination: TerminationType::RuntimeTrap,
            })
        );
        assert!(matches!(
            runtime.take_recovered_child_trap(),
            Some(Trap::UnsupportedOpcode { .. })
        ));
        assert_eq!(runtime.state(), &GuestState::Running);
    }

    #[test]
    fn graphics_child_exit_without_mode_restore_falls_back_to_text_parent() {
        let parent = mz_program(&[0xeb, 0xfe]);
        let mut runtime = Runtime::from_program(&parent, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_file(
                DosDrive::C,
                "NORESTORE.COM",
                vec![0xb8, 0x13, 0x00, 0xcd, 0x10, 0xb8, 0x07, 0x4c, 0xcd, 0x21],
            )
            .unwrap();
        let path = runtime.drives().parse_path(b"C:\\NORESTORE.COM").unwrap();
        runtime.exec(path, b"", 0).unwrap();
        runtime.run_slice(4);

        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x03);
        assert_eq!(
            runtime.active_process.child_result,
            Some(super::ChildResult {
                code: 7,
                termination: TerminationType::Normal,
            })
        );
        assert_eq!(runtime.state(), &GuestState::Running);
    }

    #[test]
    fn shell_recovers_from_a_trapped_graphics_child_and_remains_interactive() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let mut runtime = Runtime::from_program(shell, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_file(
                DosDrive::C,
                "BROKEN.COM",
                vec![0xb8, 0x13, 0x00, 0xcd, 0x10, 0x0f],
            )
            .unwrap();
        runtime.run_slice(10_000_000);

        for ascii in [b'B', b'R', b'O', b'K', b'E', b'N', b'\r'] {
            runtime.inject_ascii(ascii);
            runtime.run_slice(10_000_000);
        }

        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert_eq!(
            runtime.last_delivered_child_result(),
            Some(super::ChildResult {
                code: 1,
                termination: TerminationType::RuntimeTrap,
            })
        );
        assert!(matches!(
            runtime.take_recovered_child_trap(),
            Some(Trap::UnsupportedOpcode { .. })
        ));
        let text: [u8; 2000] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert_eq!(
            text.windows(b"CMD C:\\>".len())
                .filter(|window| *window == b"CMD C:\\>")
                .count(),
            1
        );
        runtime.inject_ascii(b'X');
        runtime.run_slice(10_000);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
    }

    #[test]
    fn runtime_decodes_every_8086_in_out_form_with_correct_ip_and_register_policy() {
        for opcode in [0xe4, 0xe5, 0xe6, 0xe7] {
            let mut runtime = runtime_for(&[opcode, 0xc9]);
            runtime.cpu.ax = 0x1234;
            runtime.cpu.dx = 0x5678;
            runtime.cpu.flags = 0x0ad7;
            runtime.set_cpu_profile(super::CpuProfile::I8086);
            runtime.step();
            let GuestState::Trapped(Trap::UnsupportedIoPort {
                port,
                cs,
                ip,
                active_executable,
                application_id,
                ..
            }) = runtime.state()
            else {
                panic!("immediate I/O opcode {opcode:02x} did not trap structurally");
            };
            assert_eq!(*port, 0x00c9, "imm8 ports are zero-extended");
            assert_eq!((*cs, *ip), (PSP_SEGMENT, 0x0100));
            assert_eq!(runtime.cpu.ip, 0x0102);
            assert_eq!(runtime.cpu.dx, 0x5678);
            assert_eq!(runtime.cpu.flags, 0x0ad7);
            assert_eq!(active_executable, b"C:\\PROGRAM.COM");
            assert_eq!(application_id, b"org.sunlight.chronos");
        }

        let mut out_byte = runtime_for(&[0xee]);
        out_byte.cpu.dx = 0x03c8;
        out_byte.cpu.ax = 0x7f20;
        out_byte.cpu.flags = 0x0ad7;
        out_byte.step();
        assert_eq!(out_byte.cpu.ip, 0x0101);
        assert_eq!(out_byte.cpu.dx, 0x03c8);
        assert_eq!(out_byte.cpu.flags, 0x0ad7);

        let mut out_word = runtime_for(&[0xef]);
        out_word.cpu.dx = 0x03c7;
        out_word.cpu.ax = 0x2020;
        out_word.step();
        assert_eq!(out_word.cpu.ip, 0x0101);
        assert_eq!(out_word.cpu.dx, 0x03c7);

        let mut in_byte = runtime_for(&[0xec]);
        in_byte.cpu.dx = 0x03c9;
        in_byte.cpu.ax = 0xaa55;
        in_byte.cpu.flags = 0x0ad7;
        in_byte.step();
        assert_eq!(in_byte.cpu.ip, 0x0101);
        assert_eq!(in_byte.cpu.ax, 0xaa00);
        assert_eq!(in_byte.cpu.dx, 0x03c9);
        assert_eq!(in_byte.cpu.flags, 0x0ad7);

        let mut in_word = runtime_for(&[0xed]);
        in_word.cpu.dx = 0x03c9;
        in_word.cpu.ax = 0xaa55;
        in_word.step();
        assert_eq!(in_word.cpu.ip, 0x0101);
        assert_eq!(in_word.cpu.ax, 0xaa55);
        assert!(matches!(
            in_word.state(),
            GuestState::Trapped(Trap::UnsupportedIoPort {
                operation: IoOperation::Read,
                port: 0x03c9,
                width: IoWidth::Word,
                value: None,
                ..
            })
        ));
    }

    #[test]
    fn palette_only_updates_reconvert_pixels_without_faking_framebuffer_damage() {
        let mut runtime = runtime_for(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, 0xb8, 0x00, 0xa0, 0x8e, 0xc0, 0x31, 0xff, 0xb0, 32, 0xaa,
            0xba, 0xc8, 0x03, 0xb0, 32, 0xee, 0x42, 0xb0, 63, 0xee, 0x30, 0xc0, 0xee, 0xee, 0xf4,
        ]);
        runtime.run_slice(7);
        assert_eq!(runtime.framebuffer_index(0, 0), Some(32));
        runtime.take_graphics_dirty_rows();
        let framebuffer_generation = runtime.framebuffer_generation();
        runtime.run_slice(64);
        assert_eq!(runtime.palette_generation(), 2);
        assert!(!runtime
            .take_graphics_dirty_rows()
            .iter()
            .any(|dirty| *dirty));
        assert_eq!(runtime.framebuffer_generation(), framebuffer_generation);

        let mut converted = alloc::vec![Rgb8::default(); VGA_FRAMEBUFFER_BYTES];
        let rows = [true; VGA_HEIGHT];
        assert!(runtime.convert_graphics_rows(&rows, &mut converted));
        assert_eq!(converted[0], Rgb8::new(255, 0, 0));
        assert_eq!(runtime.framebuffer_index(0, 0), Some(32));
    }

    #[test]
    fn palcycle_guest_has_deterministic_palette_checkpoints_and_static_framebuffer() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM");
        let mut runtime = Runtime::from_com(image).unwrap();

        run_to_palette_generation(&mut runtime, 33);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.framebuffer_checksum(), 0x2911_9770_0f65_2b25);
        assert_eq!(runtime.framebuffer_generation(), 51_840);
        assert_eq!(runtime.palette_checksum(), 0x055f_8c3d_d0a4_e6de);
        assert_eq!(runtime.framebuffer_index(0, 20), Some(32));
        let framebuffer_checksum = runtime.framebuffer_checksum();
        let framebuffer_generation = runtime.framebuffer_generation();

        let mut pixels = alloc::vec![Rgb8::default(); VGA_FRAMEBUFFER_BYTES];
        assert!(runtime.convert_graphics_rows(&[true; VGA_HEIGHT], &mut pixels));
        let checkpoint_zero_pixel = pixels[20 * 320];

        run_to_palette_generation(&mut runtime, 65);
        assert_eq!(runtime.palette_checksum(), 0xac08_f70b_ac57_1700);
        assert_eq!(runtime.framebuffer_checksum(), framebuffer_checksum);
        assert_eq!(runtime.framebuffer_generation(), framebuffer_generation);
        assert!(runtime.convert_graphics_rows(&[true; VGA_HEIGHT], &mut pixels));
        assert_ne!(pixels[20 * 320], checkpoint_zero_pixel);

        run_to_palette_generation(&mut runtime, 33 + 32 * 32);
        assert_eq!(runtime.palette_checksum(), 0x055f_8c3d_d0a4_e6de);
        assert_eq!(runtime.framebuffer_checksum(), framebuffer_checksum);
        assert_eq!(runtime.framebuffer_generation(), framebuffer_generation);
    }

    #[test]
    fn palcycle_escape_restores_default_dac_and_text_mode() {
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        run_to_palette_generation(&mut runtime, 33);
        runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        runtime.run_slice(20_000);
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.palette(), &DEFAULT_VGA_PALETTE);
    }

    #[test]
    fn incomplete_dac_entry_is_never_committed() {
        let mut runtime = runtime_for(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, 0xba, 0xc8, 0x03, 0xb0, 32, 0xee, 0x42, 0xb0, 63, 0xee,
            0xee, 0xb8, 0x03, 0x00, 0xcd, 0x10, 0xb8, 0x00, 0x4c, 0xcd, 0x21,
        ]);
        let original = runtime.palette_entries()[32];
        runtime.run_slice(64);
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
        assert_eq!(runtime.palette_generation(), 1);
        assert_eq!(runtime.palette_entries()[32], original);
    }

    #[test]
    fn repeated_mode13_selection_restores_default_dac_and_resets_sequences() {
        let mut runtime = runtime_for(&[
            0xb8, 0x13, 0x00, 0xcd, 0x10, 0xba, 0xc8, 0x03, 0xb0, 32, 0xee, 0x42, 0xb0, 63, 0xee,
            0x30, 0xc0, 0xee, 0xee, 0xb8, 0x13, 0x00, 0xcd, 0x10, 0xf4,
        ]);
        runtime.run_slice(64);
        assert_eq!(runtime.state(), &GuestState::Halted);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.palette_generation(), 3);
        assert_eq!(runtime.palette(), &DEFAULT_VGA_PALETTE);
        assert_eq!(runtime.palette_entries(), &crate::default_vga_dac_entries());
    }

    #[test]
    fn unsupported_io_child_trap_restores_parent_and_carries_guest_identity() {
        let parent = mz_program(&[0xeb, 0xfe]);
        let mut runtime = Runtime::from_program(&parent, b"").unwrap();
        runtime.set_application_id(b"org.sunlight.io-test");
        runtime
            .drives_mut()
            .add_base_file(
                DosDrive::C,
                "BADIO.COM",
                alloc::vec![0xba, 0x34, 0x12, 0xb0, 7, 0xee],
            )
            .unwrap();
        let path = runtime.drives().parse_path(b"C:\\BADIO.COM").unwrap();
        let parent_psp = runtime.current_psp();
        runtime.exec(path, b"", 0).unwrap();
        runtime.run_slice(3);
        assert_eq!(runtime.current_psp(), parent_psp);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert!(matches!(
            runtime.take_recovered_child_trap(),
            Some(Trap::UnsupportedIoPort {
                operation: IoOperation::Write,
                port: 0x1234,
                width: IoWidth::Byte,
                value: Some(7),
                active_executable,
                application_id,
                ..
            }) if active_executable == b"C:\\BADIO.COM" && application_id == b"org.sunlight.io-test"
        ));
    }

    #[test]
    fn shell_runs_palcycle_escape_then_ver_and_dir_with_one_restored_prompt() {
        let shell = include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE");
        let palcycle = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM");
        let mut runtime = Runtime::from_program(shell, b"").unwrap();
        runtime
            .drives_mut()
            .add_base_directory(DosDrive::C, "TESTS")
            .unwrap();
        runtime
            .drives_mut()
            .add_base_file(DosDrive::C, "TESTS/PALCYCLE.COM", palcycle.to_vec())
            .unwrap();
        runtime.run_slice(10_000_000);
        let shell_psp = runtime.current_psp();

        for ascii in b"PALCYCLE" {
            runtime.inject_ascii(*ascii);
            runtime.run_slice(10_000_000);
        }
        runtime.inject_ascii(b'\r');
        run_to_palette_generation(&mut runtime, 33);
        assert_ne!(runtime.current_psp(), shell_psp);
        let child_psp = runtime.current_psp();
        assert_eq!(runtime.video_mode(), GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.framebuffer_checksum(), 0x2911_9770_0f65_2b25);

        runtime.inject_key(BiosKey {
            ascii: 0x1b,
            scan_code: 0x01,
        });
        runtime.run_slice(20_000);
        assert_eq!(runtime.current_psp(), shell_psp);
        assert_eq!(runtime.video_mode(), GuestVideoMode::Text80x25Color);
        assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        assert!(runtime.parent_process.is_none());
        assert!(runtime
            .arena
            .blocks()
            .iter()
            .all(|block| block.owner_psp != Some(child_psp)));
        assert_eq!(
            runtime.last_delivered_child_result(),
            Some(super::ChildResult {
                code: 0,
                termination: TerminationType::Normal,
            })
        );
        let text: [u8; 2000] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert_eq!(
            text.windows(b"CMD C:\\>".len())
                .filter(|window| *window == b"CMD C:\\>")
                .count(),
            1
        );

        for command in [b"VER\r".as_slice(), b"DIR\r".as_slice()] {
            for ascii in command {
                runtime.inject_ascii(*ascii);
                runtime.run_slice(10_000_000);
            }
            assert_eq!(runtime.state(), &GuestState::WaitingForInput);
        }
        let text: [u8; 2000] =
            core::array::from_fn(|index| runtime.cell(index % 80, index / 80).character);
        assert!(text
            .windows(b"Sunlight DOS Shell 0.1".len())
            .any(|window| window == b"Sunlight DOS Shell 0.1"));
        assert!(text
            .windows(b"item(s)".len())
            .any(|window| window == b"item(s)"));
    }

    #[test]
    #[ignore = "bounded three-minute-equivalent PALCYCLE soak"]
    fn palcycle_three_minute_equivalent_soak_has_no_growth_or_framebuffer_churn() {
        const FRAMES: u64 = 30 * 60 * 3;
        let image = include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM");
        let mut runtime = Runtime::from_com(image).unwrap();
        run_to_palette_generation(&mut runtime, 33);
        let framebuffer_checksum = runtime.framebuffer_checksum();
        let framebuffer_generation = runtime.framebuffer_generation();
        let process_psp = runtime.current_psp();
        let arena_blocks = runtime.arena.blocks().to_vec();
        let open_handles = (0..=u16::MAX)
            .filter(|handle| runtime.handles.get(*handle).is_ok())
            .count();
        let target_generation = 33 + FRAMES * 32;
        let mut slices = 0usize;
        while runtime.palette_generation() < target_generation {
            runtime.run_slice(4096);
            slices += 1;
            assert!(slices < 10_000, "bounded soak exceeded its slice cap");
        }
        assert_eq!(runtime.current_psp(), process_psp);
        assert!(runtime.parent_process.is_none());
        assert_eq!(runtime.arena.blocks(), arena_blocks.as_slice());
        assert_eq!(
            (0..=u16::MAX)
                .filter(|handle| runtime.handles.get(*handle).is_ok())
                .count(),
            open_handles
        );
        assert_eq!(runtime.framebuffer_checksum(), framebuffer_checksum);
        assert_eq!(runtime.framebuffer_generation(), framebuffer_generation);
        assert!(runtime.palette_generation() >= target_generation);
        assert_eq!(runtime.unsupported_io_attempts(), 0);
    }
    #[test]
    fn dos_memory_interrupts_enforce_owner_and_report_growth_limit() {
        let mut r = runtime_for(&[0xcd, 0x21]);
        let psp = r.current_psp();
        let saved = r.arena.blocks().to_vec();
        let call = |r: &mut Runtime, ax, bx, es| {
            r.cpu.cs = psp;
            r.cpu.ip = 0x100;
            r.cpu.ax = ax;
            r.cpu.bx = bx;
            r.cpu.es = es;
            r.step();
        };
        call(&mut r, 0x4800, 0x20, 0);
        assert_eq!(r.cpu.flags & CpuState::FLAG_CF, 0);
        let first = r.cpu.ax;
        call(&mut r, 0x4800, 0x20, 0);
        let second = r.cpu.ax;
        call(&mut r, 0x4900, 0, second);
        assert_eq!(r.cpu.flags & CpuState::FLAG_CF, 0);
        call(&mut r, 0x4a00, 0x10, first);
        assert_eq!(r.memory.read_u16(first - 1, 3), 0x10);
        call(&mut r, 0x4a00, 0x30, first);
        assert_eq!(r.cpu.flags & CpuState::FLAG_CF, 0);
        let before = r.arena.blocks().to_vec();
        let limit = 0xa000 - first;
        call(&mut r, 0x4a00, 0xffff, first);
        assert_eq!((r.cpu.ax, r.cpu.bx), (8, limit));
        assert_ne!(r.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(r.arena.blocks(), before.as_slice());
        let foreign = r.arena.allocate(1, 0x2222).unwrap();
        call(&mut r, 0x4900, 0, foreign);
        assert_eq!(r.cpu.ax, 9);
        call(&mut r, 0x4a00, 1, foreign);
        assert_eq!(r.cpu.ax, 9);
        assert_ne!(r.cpu.flags & CpuState::FLAG_CF, 0);
        r.arena.free_owner(0x2222);
        call(&mut r, 0x4900, 0, first);
        call(&mut r, 0x4900, 0, first);
        assert_eq!(r.cpu.ax, 9);
        assert_ne!(r.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(r.arena.blocks(), saved.as_slice());
        call(&mut r, 0x4800, 0xffff, 0);
        assert_eq!((r.cpu.ax, r.cpu.bx), (8, r.arena.largest_available()));
    }

    #[test]
    fn maximum_mz_child_releases_environment_and_process_on_repeated_exit() {
        let mut r = Runtime::from_program(&mz_program(&[0x90, 0xf4]), b"").unwrap();
        let mut child = mz_program(&[0xb8, 0, 0x4c, 0xcd, 0x21]);
        child[12..14].copy_from_slice(&0xffffu16.to_le_bytes());
        r.drives_mut()
            .add_base_file(DosDrive::C, "MAX.EXE", child)
            .unwrap();
        let saved = r.arena.blocks().to_vec();
        r.drives_mut()
            .add_base_file(DosDrive::C, "BAD.EXE", vec![b'M', b'Z'])
            .unwrap();
        let bad = r.drives().parse_path(b"C:/BAD.EXE").unwrap();
        assert!(r.exec(bad, b"", 0).is_err());
        assert_eq!(r.arena.blocks(), saved.as_slice());
        for _ in 0..16 {
            let path = r.drives().parse_path(b"C:/MAX.EXE").unwrap();
            r.exec(path, b"", 0).unwrap();
            let psp = r.current_psp();
            let env = r.memory.read_u16(psp, 0x2c);
            assert!(r.arena.contains_allocated(env, psp));
            assert_eq!(r.memory.read_u16(psp, 2), 0xa000);
            assert_eq!(r.arena.largest_available(), 0);
            r.run_slice(2);
            assert_eq!(r.arena.blocks(), saved.as_slice());
        }
        r.exit(0).unwrap();
        assert_eq!(r.arena.blocks().len(), 1);
        assert_eq!(r.arena.largest_available(), 0x9000);
    }
}
