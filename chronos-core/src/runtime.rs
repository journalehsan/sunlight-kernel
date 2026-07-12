use alloc::{collections::VecDeque, vec::Vec};

use crate::{
    dos, load_com_with_command_tail, CpuState, DosHandleTable, DriveTable, GuestMemory,
    LoaderError, TextCell, TextModeSurface, DEFAULT_ATTRIBUTE, TEXT_COLUMNS, TEXT_ROWS,
    VIDEO_SEGMENT,
};

const BDA_SEGMENT: u16 = 0x0040;
const BDA_VIDEO_MODE: u16 = 0x0049;
const BDA_COLUMNS: u16 = 0x004a;
const BDA_PAGE_SIZE: u16 = 0x004c;
const BDA_CURSOR_POSITIONS: u16 = 0x0050;
const BDA_CURSOR_SHAPE: u16 = 0x0060;
const BDA_ACTIVE_PAGE: u16 = 0x0062;

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
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestState {
    Ready,
    Running,
    WaitingForInput,
    Exited { code: u8 },
    Halted,
    Trapped(Trap),
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
    shift: bool,
    ctrl: bool,
    alt: bool,
    pub(crate) drives: DriveTable,
    pub(crate) handles: DosHandleTable,
    pub(crate) dta_segment: u16,
    pub(crate) dta_offset: u16,
    pub(crate) searches: Vec<crate::DirectoryEntry>,
    pub(crate) search_index: usize,
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
            shift: false,
            ctrl: false,
            alt: false,
            drives: DriveTable::new(),
            handles: DosHandleTable::new(),
            dta_segment: crate::PSP_SEGMENT,
            dta_offset: 0x0080,
            searches: Vec::new(),
            search_index: 0,
        };
        runtime.reset_video();
        Ok(runtime)
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

    pub fn inject_key(&mut self, key: BiosKey) -> bool {
        self.keyboard.push_back(key);
        if matches!(self.state, GuestState::WaitingForInput) {
            self.complete_pending_input();
            return true;
        }
        false
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
        }
        if !matches!(self.state, GuestState::Running) {
            return false;
        }
        let state_before = self.state.clone();
        for _ in 0..budget {
            if !matches!(self.state, GuestState::Running) {
                break;
            }
            self.step();
        }
        self.memory.take_video_dirty().iter().any(|dirty| *dirty) || self.state != state_before
    }

    pub fn step(&mut self) {
        if matches!(self.state, GuestState::Ready) {
            self.state = GuestState::Running;
        }
        if !matches!(self.state, GuestState::Running) {
            return;
        }
        let instruction_cs = self.cpu.cs;
        let instruction_ip = self.cpu.ip;
        let mut segment_override = None;
        let mut repeat = false;
        let opcode = loop {
            let opcode = self.fetch_u8();
            match opcode {
                0x26 => segment_override = Some(self.cpu.es),
                0x2e => segment_override = Some(self.cpu.cs),
                0x36 => segment_override = Some(self.cpu.ss),
                0x3e => segment_override = Some(self.cpu.ds),
                0xf3 => repeat = true,
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
            self.state = GuestState::Trapped(trap);
        }
    }

    fn execute(
        &mut self,
        opcode: u8,
        override_segment: Option<u16>,
        repeat: bool,
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
            0xe8 => {
                let displacement = self.fetch_u16() as i16;
                self.push_u16(self.cpu.ip);
                self.cpu.ip = self.cpu.ip.wrapping_add(displacement as u16);
            }
            0xc3 => self.cpu.ip = self.pop_u16(),
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
            0x00..=0x03
            | 0x28..=0x2b
            | 0x38..=0x3b
            | 0x30..=0x33
            | 0x20..=0x23
            | 0x08..=0x0b
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
            0xfc => self.cpu.flags &= !CpuState::FLAG_DF,
            0xfd => self.cpu.flags |= CpuState::FLAG_DF,
            0xf8 => self.cpu.flags &= !CpuState::FLAG_CF,
            0xf9 => self.cpu.flags |= CpuState::FLAG_CF,
            0xfa => self.cpu.flags &= !CpuState::FLAG_IF,
            0xfb => self.cpu.flags |= CpuState::FLAG_IF,
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
            0xa4 | 0xa5 | 0xaa | 0xab | 0xac | 0xad => {
                self.string_op(opcode, override_segment, repeat, start_ip)
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

    fn string_op(
        &mut self,
        opcode: u8,
        override_segment: Option<u16>,
        repeat: bool,
        start_ip: u16,
    ) {
        if repeat && self.cpu.cx == 0 {
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
            0xaa | 0xab => {
                if word {
                    self.memory.write_u16(self.cpu.es, self.cpu.di, self.cpu.ax);
                } else {
                    self.memory
                        .write_u8(self.cpu.es, self.cpu.di, self.cpu.al());
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
        if repeat {
            self.cpu.cx = self.cpu.cx.wrapping_sub(1);
            if self.cpu.cx != 0 {
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
    fn dec16(&mut self, value: u16) -> u16 {
        let carry = self.cpu.flags & CpuState::FLAG_CF;
        let result = self.sub16(value, 1);
        self.cpu.flags = (self.cpu.flags & !CpuState::FLAG_CF) | carry;
        result
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
        self.state = GuestState::Exited { code };
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
        TextModeSurface::clear(&mut self.memory, DEFAULT_ATTRIBUTE);
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.cursor_shape = 0x0607;
        self.memory.write_u8(BDA_SEGMENT, BDA_VIDEO_MODE, 0x03);
        self.memory
            .write_u16(BDA_SEGMENT, BDA_COLUMNS, TEXT_COLUMNS as u16);
        self.memory.write_u16(
            BDA_SEGMENT,
            BDA_PAGE_SIZE,
            (TEXT_COLUMNS * TEXT_ROWS * 2) as u16,
        );
        self.memory.write_u8(BDA_SEGMENT, BDA_ACTIVE_PAGE, 0);
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

#[cfg(test)]
mod tests {
    use super::{translate_key_press, BiosKey, GuestState, HostKeyEvent, Runtime};
    use crate::{CpuState, CHRONOS_INTERACTIVE_COM, HELLO_CHRONOS_COM, PSP_SEGMENT};

    fn runtime_for(bytes: &[u8]) -> Runtime {
        Runtime::from_com(bytes).unwrap()
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
}
