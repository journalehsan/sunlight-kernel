use alloc::{collections::VecDeque, vec::Vec};

use crate::{
    dos, load_com_with_command_tail, load_program, ArenaError, CpuState, DosHandleTable,
    DosMemoryArena, DosPath, DriveTable, ExecutableFormat, GuestMemory, GuestVideoMode,
    IoOperation, IoTrap, IoWidth, LoadedProgram, LoaderError, Rgb8, TextCell, TextModeSurface,
    VgaDacEntry, DEFAULT_ATTRIBUTE, TEXT_COLUMNS, TEXT_ROWS, VGA_FRAMEBUFFER_BYTES, VGA_HEIGHT,
    VIDEO_SEGMENT,
};

const BDA_SEGMENT: u16 = 0x0040;
const BDA_VIDEO_MODE: u16 = 0x0049;
const BDA_COLUMNS: u16 = 0x004a;
const BDA_PAGE_SIZE: u16 = 0x004c;
const BDA_CURSOR_POSITIONS: u16 = 0x0050;
const BDA_CURSOR_SHAPE: u16 = 0x0060;
const BDA_ACTIVE_PAGE: u16 = 0x0062;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostKeyEvent {
    pub keycode: u8,
    pub pressed: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Translate SunlightOS PS/2-set-1 key events into BIOS key pairs. Printable
/// text should be injected through `Event::Key`; this path is for controls and
/// extended keys, preventing duplicate printable events.
pub fn translate_key_press(event: HostKeyEvent) -> Option<BiosKey> {
    if !event.pressed {
        return None;
    }
    let scan_code = event.keycode;
    let ascii = match scan_code {
        0x01 => 0x1b,
        0x0e => 0x08,
        0x0f => b'\t',
        0x1c => b'\r',
        0x39 => b' ',
        0x47 | 0x48 | 0x49 | 0x4b | 0x4d | 0x4f | 0x50 | 0x51 | 0x52 | 0x53 | 0x3b..=0x44 => 0,
        _ => return None,
    };
    Some(BiosKey { ascii, scan_code })
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum InputMode {
    Bios,
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
    pending_input: Option<InputMode>,
    pub(crate) cursor_column: usize,
    pub(crate) cursor_row: usize,
    pub(crate) cursor_shape: u16,
    video_mode: GuestVideoMode,
    video_mode_generation: u64,
    io: crate::io::GuestIoDispatcher,
    pub(crate) mouse: crate::DosMouse,
    cooperative_yield_requested: bool,
    cooperative_yielded_last_slice: bool,
    cooperative_yield_count: u64,
    instructions_retired: u64,
    last_slice_stop_reason: SliceStopReason,
    last_wake_source: Option<GuestWakeSource>,
    wake_generation: u64,
    shift: bool,
    ctrl: bool,
    alt: bool,
    pub(crate) drives: DriveTable,
    pub(crate) handles: DosHandleTable,
    pub(crate) dta_segment: u16,
    pub(crate) dta_offset: u16,
    pub(crate) searches: Vec<crate::DirectoryEntry>,
    pub(crate) search_index: usize,
    pub(crate) arena: DosMemoryArena,
    pub(crate) active_process: DosProcess,
    pub(crate) parent_process: Option<DosProcess>,
    recovered_child_trap: Option<Trap>,
    pub(crate) last_delivered_child_result: Option<ChildResult>,
    pub(crate) cpu_profile: CpuProfile,
    guest_unix_time: u64,
    environment_app_id: Vec<u8>,
    executable_path: Vec<u8>,
    interrupt_vectors: [(u16, u16); 256],
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
            pending_input: None,
            cursor_column: 0,
            cursor_row: 0,
            cursor_shape: 0x0607,
            video_mode: GuestVideoMode::Text80x25Color,
            video_mode_generation: 0,
            io: crate::io::GuestIoDispatcher::default(),
            mouse: crate::DosMouse::default(),
            cooperative_yield_requested: false,
            cooperative_yielded_last_slice: false,
            cooperative_yield_count: 0,
            instructions_retired: 0,
            last_slice_stop_reason: SliceStopReason::NotRun,
            last_wake_source: None,
            wake_generation: 0,
            shift: false,
            ctrl: false,
            alt: false,
            drives: DriveTable::new(),
            handles: DosHandleTable::new(),
            dta_segment: crate::PSP_SEGMENT,
            dta_offset: 0x0080,
            searches: Vec::new(),
            search_index: 0,
            arena: DosMemoryArena::new(),
            active_process,
            parent_process: None,
            recovered_child_trap: None,
            last_delivered_child_result: None,
            cpu_profile: CpuProfile::I8086,
            guest_unix_time: 0,
            environment_app_id: b"org.sunlight.chronos".to_vec(),
            executable_path: b"C:\\PROGRAM.COM".to_vec(),
            interrupt_vectors: [(0, 0); 256],
        };
        let _ = runtime.arena.allocate(0x1000, crate::PSP_SEGMENT);
        runtime.initialize_environment();
        runtime.reset_video();
        Ok(runtime)
    }

    pub fn from_program(image: &[u8], command_tail: &[u8]) -> Result<Self, LoaderError> {
        let mut runtime = Self::from_com_with_command_tail(&[], &[])?;
        runtime.memory.clear();
        runtime.arena = DosMemoryArena::new();
        runtime.drives = DriveTable::new();
        runtime.handles = DosHandleTable::new();
        runtime.keyboard.clear();
        runtime.pending_input = None;
        runtime.searches.clear();
        runtime.search_index = 0;
        runtime.parent_process = None;
        runtime.recovered_child_trap = None;
        runtime.last_delivered_child_result = None;
        runtime.interrupt_vectors = [(0, 0); 256];
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
        runtime.install_initial_program(program);
        runtime.active_process.environment_segment = environment_segment;
        runtime.active_process.environment_owned = true;
        runtime.reset_video();
        Ok(runtime)
    }

    fn install_initial_program(&mut self, program: LoadedProgram) {
        self.cpu = program.cpu;
        self.active_process = DosProcess {
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

    pub(crate) fn interrupt_vector(&self, interrupt: u8) -> (u16, u16) {
        self.interrupt_vectors[interrupt as usize]
    }

    pub(crate) fn set_interrupt_vector(&mut self, interrupt: u8, segment: u16, offset: u16) {
        self.interrupt_vectors[interrupt as usize] = (segment, offset);
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
        self.arena
            .allocate(paragraphs, self.active_process.psp_segment)
    }

    pub(crate) fn free_memory(&mut self, segment: u16) -> Result<(), ArenaError> {
        self.arena.free(segment, self.active_process.psp_segment)
    }

    pub(crate) fn resize_memory(
        &mut self,
        segment: u16,
        paragraphs: u16,
    ) -> Result<(), ArenaError> {
        self.arena
            .resize(segment, paragraphs, self.active_process.psp_segment)
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
        let program = load_program(
            &mut self.memory,
            &mut self.arena,
            &image,
            command_tail,
            Some(parent_psp),
            0,
        )?;
        let environment_segment = match self.write_environment(
            program.psp_segment,
            &inherited_environment,
            path.display().as_bytes(),
        ) {
            Ok(segment) => segment,
            Err(error) => {
                self.arena.free_owner(program.psp_segment);
                return Err(error);
            }
        };
        self.memory
            .write_u16(program.psp_segment, 0x002c, environment_segment);
        self.active_process.cpu = self.cpu;
        self.active_process.cpu.flags &= !CpuState::FLAG_CF;
        self.active_process.dta_segment = self.dta_segment;
        self.active_process.dta_offset = self.dta_offset;
        self.active_process.handles = self.handles.clone();
        self.active_process.current_drive = self.drives.current_drive;
        let mut child = DosProcess {
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
        self.searches.clear();
        self.search_index = 0;
        self.pending_input = None;
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
        self.arena.free_owner(child.psp_segment);
        self.active_process.child_result = Some(result);
        self.cpu = self.active_process.cpu;
        self.dta_segment = self.active_process.dta_segment;
        self.dta_offset = self.active_process.dta_offset;
        self.handles = self.active_process.handles.clone();
        self.drives.current_drive = self.active_process.current_drive;
        self.searches.clear();
        self.search_index = 0;
        self.pending_input = None;
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
        Some(self.memory.read_u8(
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
                .read_u8(crate::VGA_FRAMEBUFFER_SEGMENT, offset as u16) as u64;
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

    pub fn inject_ascii(&mut self, ascii: u8) -> bool {
        self.inject_key(BiosKey {
            ascii,
            scan_code: 0,
        })
    }

    pub fn update_modifiers(&mut self, shift: bool, ctrl: bool, alt: bool) {
        self.shift = shift;
        self.ctrl = ctrl;
        self.alt = alt;
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
        let opcode = loop {
            let opcode = self.fetch_u8();
            match opcode {
                0x26 => segment_override = Some(self.cpu.es),
                0x2e => segment_override = Some(self.cpu.cs),
                0x36 => segment_override = Some(self.cpu.ss),
                0x3e => segment_override = Some(self.cpu.ds),
                0xf2 => repeat = Some(false),
                0xf3 => repeat = Some(true),
                _ => break opcode,
            }
            if self.cpu.ip.wrapping_sub(instruction_ip) > 5 {
                self.state = GuestState::Trapped(Trap::MalformedPrefix {
                    cs: instruction_cs,
                    ip: instruction_ip,
                });
                return;
            }
        };
        let result = self.execute(opcode, segment_override, repeat, instruction_ip);
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

    fn execute(
        &mut self,
        opcode: u8,
        override_segment: Option<u16>,
        repeat: Option<bool>,
        start_ip: u16,
    ) -> Result<(), Trap> {
        match opcode {
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
            0xe2 => {
                let displacement = self.fetch_u8() as i8;
                self.cpu.cx = self.cpu.cx.wrapping_sub(1);
                if self.cpu.cx != 0 {
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
                crate::io::execute_io_instruction(
                    &mut self.io,
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
            0xd0..=0xd3 => self.group_shift(opcode, override_segment, start_ip)?,
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
            4 => {
                if word {
                    let product = self.cpu.ax as u32 * value;
                    self.cpu.ax = product as u16;
                    self.cpu.dx = (product >> 16) as u16;
                    self.set_flag(CpuState::FLAG_CF, self.cpu.dx != 0);
                    self.set_flag(CpuState::FLAG_OF, self.cpu.dx != 0);
                } else {
                    let product = self.cpu.al() as u16 * value as u16;
                    self.cpu.ax = product;
                    self.set_flag(CpuState::FLAG_CF, product > 0xff);
                    self.set_flag(CpuState::FLAG_OF, product > 0xff);
                }
            }
            5 => {
                if word {
                    let product = self.cpu.ax as i16 as i32 * value as u16 as i16 as i32;
                    self.cpu.ax = product as u16;
                    self.cpu.dx = (product >> 16) as u16;
                    let fits = product == self.cpu.ax as i16 as i32;
                    self.set_flag(CpuState::FLAG_CF, !fits);
                    self.set_flag(CpuState::FLAG_OF, !fits);
                } else {
                    let product = self.cpu.al() as i8 as i16 * value as u8 as i8 as i16;
                    self.cpu.ax = product as u16;
                    let fits = product == self.cpu.al() as i8 as i16;
                    self.set_flag(CpuState::FLAG_CF, !fits);
                    self.set_flag(CpuState::FLAG_OF, !fits);
                }
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
                    let quotient = dividend / divisor;
                    if !(i16::MIN as i32..=i16::MAX as i32).contains(&quotient) {
                        return Err(Trap::DivideError {
                            cs: self.cpu.cs,
                            ip: start_ip,
                        });
                    }
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
    ) -> Result<(), Trap> {
        let operand = self.decode_modrm(override_segment)?;
        let count = match opcode {
            0xd0 | 0xd1 => 1,
            _ => self.cpu.cl() & 0x1f,
        };
        if count == 0 {
            return Ok(());
        }
        let word = opcode & 1 != 0;
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

    fn shift8(&mut self, kind: u8, mut value: u8, count: u8, ip: u16) -> Result<u8, Trap> {
        if kind > 7 {
            return Err(self.bad_opcode(ip));
        }
        for _ in 0..count {
            let carry = match kind {
                0 => {
                    let carry = value & 0x80 != 0;
                    value = value.rotate_left(1);
                    carry
                }
                1 => {
                    let carry = value & 1 != 0;
                    value = value.rotate_right(1);
                    carry
                }
                2 => {
                    let carry = value & 0x80 != 0;
                    let old_carry = self.cpu.flags & CpuState::FLAG_CF != 0;
                    value = (value << 1) | old_carry as u8;
                    carry
                }
                3 => {
                    let carry = value & 1 != 0;
                    let old_carry = self.cpu.flags & CpuState::FLAG_CF != 0;
                    value = (value >> 1) | ((old_carry as u8) << 7);
                    carry
                }
                4 => {
                    let carry = value & 0x80 != 0;
                    value <<= 1;
                    carry
                }
                5 | 6 => {
                    let carry = value & 1 != 0;
                    value >>= 1;
                    carry
                }
                7 => {
                    let carry = value & 1 != 0;
                    value = (value as i8 >> 1) as u8;
                    carry
                }
                _ => unreachable!("shift kind is constrained to 0..=7"),
            };
            self.set_flag(CpuState::FLAG_CF, carry);
        }
        if !matches!(kind, 0..=3) {
            self.flags8(value);
        }
        if count == 1 {
            let overflow = match kind {
                0 => ((value >> 7) ^ (value & 1)) != 0,
                1 => value & 0x80 != 0,
                2 | 4 => (value & 0x80 != 0) ^ (self.cpu.flags & CpuState::FLAG_CF != 0),
                3 => value & 0x80 != 0,
                _ => false,
            };
            self.set_flag(CpuState::FLAG_OF, overflow);
        }
        Ok(value)
    }

    fn shift16(&mut self, kind: u8, mut value: u16, count: u8, ip: u16) -> Result<u16, Trap> {
        if kind > 7 {
            return Err(self.bad_opcode(ip));
        }
        for _ in 0..count {
            let carry = match kind {
                0 => {
                    let carry = value & 0x8000 != 0;
                    value = value.rotate_left(1);
                    carry
                }
                1 => {
                    let carry = value & 1 != 0;
                    value = value.rotate_right(1);
                    carry
                }
                2 => {
                    let carry = value & 0x8000 != 0;
                    let old_carry = self.cpu.flags & CpuState::FLAG_CF != 0;
                    value = (value << 1) | old_carry as u16;
                    carry
                }
                3 => {
                    let carry = value & 1 != 0;
                    let old_carry = self.cpu.flags & CpuState::FLAG_CF != 0;
                    value = (value >> 1) | ((old_carry as u16) << 15);
                    carry
                }
                4 => {
                    let carry = value & 0x8000 != 0;
                    value <<= 1;
                    carry
                }
                5 | 6 => {
                    let carry = value & 1 != 0;
                    value >>= 1;
                    carry
                }
                7 => {
                    let carry = value & 1 != 0;
                    value = (value as i16 >> 1) as u16;
                    carry
                }
                _ => unreachable!("shift kind is constrained to 0..=7"),
            };
            self.set_flag(CpuState::FLAG_CF, carry);
        }
        if !matches!(kind, 0..=3) {
            self.flags16(value);
        }
        if count == 1 {
            let overflow = match kind {
                0 => ((value >> 15) ^ (value & 1)) != 0,
                1 => value & 0x8000 != 0,
                2 | 4 => (value & 0x8000 != 0) ^ (self.cpu.flags & CpuState::FLAG_CF != 0),
                3 => value & 0x8000 != 0,
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
        if self.parent_process.is_some() {
            self.restore_parent(ChildResult {
                code,
                termination: TerminationType::Normal,
            });
        } else {
            self.state = GuestState::Exited { code };
        }
        Ok(())
    }
    pub(crate) fn has_key(&self) -> bool {
        !self.keyboard.is_empty()
    }
    pub(crate) fn peek_key(&self) -> Option<BiosKey> {
        self.keyboard.front().copied()
    }
    pub(crate) fn pop_key(&mut self) -> Option<BiosKey> {
        self.keyboard.pop_front()
    }
    pub(crate) fn clear_keys(&mut self) {
        self.keyboard.clear();
    }
    pub(crate) fn shift_flags(&self) -> u8 {
        (self.shift as u8) | ((self.ctrl as u8) << 2) | ((self.alt as u8) << 3)
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
        let Some(mode) = self.pending_input else {
            return;
        };
        let Some(key) = self.pop_key() else {
            self.state = GuestState::WaitingForInput;
            return;
        };
        match mode {
            InputMode::Bios => {
                self.cpu.ax = u16::from_le_bytes([key.ascii, key.scan_code]);
                self.pending_input = None;
                self.state = GuestState::Running;
            }
            InputMode::Dos { echo } => {
                self.cpu.set_al(key.ascii);
                if echo {
                    self.teletype(key.ascii, DEFAULT_ATTRIBUTE);
                }
                self.pending_input = None;
                self.state = GuestState::Running;
            }
            InputMode::ConsoleRead { segment, offset } => {
                self.memory.write_u8(segment, offset, key.ascii);
                if key.ascii == b'\r' {
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
                    self.teletype(key.ascii, DEFAULT_ATTRIBUTE);
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
                if key.ascii == b'\r' {
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
                } else if key.ascii == 0x08 {
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
                } else if key.ascii >= 0x20 && length < maximum {
                    self.memory.write_u8(
                        segment,
                        offset.wrapping_add(2).wrapping_add(length as u16),
                        key.ascii,
                    );
                    length += 1;
                    self.memory
                        .write_u8(segment, offset.wrapping_add(1), length);
                    self.teletype(key.ascii, DEFAULT_ATTRIBUTE);
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
        self.video_mode = mode;
        self.video_mode_generation = self.video_mode_generation.wrapping_add(1);
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.memory
            .write_u8(BDA_SEGMENT, BDA_VIDEO_MODE, mode.bios_mode());
        self.memory.write_u8(BDA_SEGMENT, BDA_ACTIVE_PAGE, 0);
        match mode {
            GuestVideoMode::Text80x25Color => {
                self.mouse.leave_graphics_mode();
                TextModeSurface::clear(&mut self.memory, DEFAULT_ATTRIBUTE);
                self.cursor_shape = 0x0607;
                self.memory
                    .write_u16(BDA_SEGMENT, BDA_COLUMNS, TEXT_COLUMNS as u16);
                self.memory.write_u16(
                    BDA_SEGMENT,
                    BDA_PAGE_SIZE,
                    (TEXT_COLUMNS * TEXT_ROWS * 2) as u16,
                );
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
        TerminationType, Trap, CHRONOS_INTERACTIVE_COM, DEFAULT_VGA_PALETTE, HELLO_CHRONOS_COM,
        PSP_SEGMENT, VGA_FRAMEBUFFER_BYTES, VGA_HEIGHT,
    };

    fn runtime_for(bytes: &[u8]) -> Runtime {
        Runtime::from_com(bytes).unwrap()
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
    fn bios_data_area_and_video_calls_share_the_b8000_model() {
        let mut runtime = runtime_for(&[]);
        assert_eq!(runtime.memory.read_u8(0x40, 0x49), 0x03);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4a), 80);
        assert_eq!(runtime.memory.read_u16(0x40, 0x4c), 4000);

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
                pressed: true,
                shift: false,
                ctrl: false,
                alt: false
            }),
            Some(BiosKey {
                ascii: 0,
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
        assert_eq!(runtime.memory.read_u16(0x40, 0x4c), 4000);
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
}
