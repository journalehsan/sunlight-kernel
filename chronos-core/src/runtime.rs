use crate::{
    dos::{self, InterruptResult},
    load_com, CpuState, GuestMemory, LoaderError, TextModeSurface,
};

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
}

impl Trap {
    pub const fn summary(&self) -> &'static str {
        match self {
            Self::UnsupportedOpcode { .. } => "Unsupported guest instruction",
            Self::UnsupportedInterrupt { .. } => "Unsupported DOS or BIOS interrupt",
            Self::UnterminatedDosString { .. } => "Unterminated DOS string",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestState {
    Runnable,
    Exited { code: u8 },
    Halted,
    Trapped(Trap),
}

/// A complete runnable Chronos guest with its private memory and text output.
pub struct Runtime {
    pub cpu: CpuState,
    pub memory: GuestMemory,
    pub text: TextModeSurface,
    state: GuestState,
}

impl Runtime {
    pub fn from_com(image: &[u8]) -> Result<Self, LoaderError> {
        let mut memory = GuestMemory::new();
        let cpu = load_com(&mut memory, image)?;
        Ok(Self {
            cpu,
            memory,
            text: TextModeSurface::new(),
            state: GuestState::Runnable,
        })
    }

    pub fn state(&self) -> &GuestState {
        &self.state
    }

    /// Runs no more than `budget` guest instructions and returns whether the
    /// state or text surface changed. Call this from a native UI tick.
    pub fn run_slice(&mut self, budget: usize) -> bool {
        if !matches!(self.state, GuestState::Runnable) {
            return false;
        }

        self.text.take_dirty();
        let state_before = self.state.clone();
        for _ in 0..budget {
            if !matches!(self.state, GuestState::Runnable) {
                break;
            }
            self.step();
        }
        self.text.take_dirty() || self.state != state_before
    }

    pub fn step(&mut self) {
        if !matches!(self.state, GuestState::Runnable) {
            return;
        }

        let instruction_cs = self.cpu.cs;
        let instruction_ip = self.cpu.ip;
        let opcode = self.fetch_u8();
        match opcode {
            0x90 => {}
            0xb0..=0xb7 => {
                let immediate = self.fetch_u8();
                self.cpu.set_reg8(opcode - 0xb0, immediate);
            }
            0xb8..=0xbf => {
                let immediate = self.fetch_u16();
                self.cpu.set_reg16(opcode - 0xb8, immediate);
            }
            0xcd => {
                let interrupt = self.fetch_u8();
                match dos::dispatch(interrupt, &mut self.cpu, &self.memory, &mut self.text) {
                    Ok(InterruptResult::Continue) => {}
                    Ok(InterruptResult::Exit(code)) => {
                        self.state = GuestState::Exited { code };
                    }
                    Err(trap) => self.state = GuestState::Trapped(trap),
                }
            }
            0xeb => {
                let displacement = self.fetch_u8() as i8;
                self.cpu.ip = self.cpu.ip.wrapping_add(displacement as i16 as u16);
            }
            0xe9 => {
                let displacement = self.fetch_u16() as i16;
                self.cpu.ip = self.cpu.ip.wrapping_add(displacement as u16);
            }
            0xf4 => self.state = GuestState::Halted,
            _ => {
                self.state = GuestState::Trapped(Trap::UnsupportedOpcode {
                    cs: instruction_cs,
                    ip: instruction_ip,
                    bytes: self.instruction_bytes(instruction_ip),
                    cpu: self.cpu,
                });
            }
        }
    }

    fn fetch_u8(&mut self) -> u8 {
        let value = self.memory.read_u8(self.cpu.cs, self.cpu.ip);
        self.cpu.ip = self.cpu.ip.wrapping_add(1);
        value
    }

    fn fetch_u16(&mut self) -> u16 {
        let lo = self.fetch_u8();
        let hi = self.fetch_u8();
        u16::from_le_bytes([lo, hi])
    }

    fn instruction_bytes(&self, ip: u16) -> [u8; 4] {
        [
            self.memory.read_u8(self.cpu.cs, ip),
            self.memory.read_u8(self.cpu.cs, ip.wrapping_add(1)),
            self.memory.read_u8(self.cpu.cs, ip.wrapping_add(2)),
            self.memory.read_u8(self.cpu.cs, ip.wrapping_add(3)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{GuestState, Runtime, Trap};
    use crate::{CpuState, HELLO_CHRONOS_COM, PSP_SEGMENT};

    fn runtime_for(bytes: &[u8]) -> Runtime {
        Runtime::from_com(bytes).unwrap()
    }

    #[test]
    fn immediate_mov_instructions_assign_registers_and_advance_ip() {
        let mut runtime = runtime_for(&[0xb4, 0x09, 0xba, 0x34, 0x12]);
        runtime.step();
        assert_eq!(runtime.cpu.ah(), 0x09);
        assert_eq!(runtime.cpu.ip, 0x0102);
        runtime.step();
        assert_eq!(runtime.cpu.dx, 0x1234);
        assert_eq!(runtime.cpu.ip, 0x0105);
    }

    #[test]
    fn short_and_near_jumps_use_the_ip_after_the_instruction() {
        let mut short = runtime_for(&[0xeb, 0x02, 0x90, 0x90, 0xb0, 0x7f]);
        short.step();
        short.step();
        assert_eq!(short.cpu.al(), 0x7f);

        let mut near = runtime_for(&[0xe9, 0x02, 0x00, 0x90, 0x90, 0xb3, 0x42]);
        near.step();
        near.step();
        assert_eq!(near.cpu.bl(), 0x42);
    }

    #[test]
    fn dos_character_string_and_exit_services_work() {
        let mut character = runtime_for(&[0xba, b'X' as u8, 0x00, 0xb4, 0x02, 0xcd, 0x21]);
        character.run_slice(3);
        assert_eq!(character.text.cell(0, 0).character, b'X');

        let mut string = runtime_for(&[
            0xba, 0x0c, 0x01, 0xb4, 0x09, 0xcd, 0x21, 0xb8, 0x07, 0x4c, 0xcd, 0x21, b'O', b'K',
            b'$',
        ]);
        string.run_slice(16);
        assert_eq!(string.text.cell(0, 0).character, b'O');
        assert_eq!(string.text.cell(1, 0).character, b'K');
        assert_eq!(string.state(), &GuestState::Exited { code: 7 });
    }

    #[test]
    fn bios_teletype_controls_are_routed_to_the_text_surface() {
        let mut runtime = runtime_for(&[0xb8, b'Q' as u8, 0x0e, 0xcd, 0x10]);
        runtime.run_slice(3);
        assert_eq!(runtime.text.cell(0, 0).character, b'Q');
    }

    #[test]
    fn unsupported_opcode_has_diagnostics_instead_of_panicking() {
        let mut runtime = runtime_for(&[0xff]);
        runtime.step();
        assert!(matches!(
            runtime.state(),
            GuestState::Trapped(Trap::UnsupportedOpcode {
                cs: PSP_SEGMENT,
                ip: 0x0100,
                bytes: [0xff, ..],
                ..
            })
        ));
    }

    #[test]
    fn unterminated_dos_string_is_bounded_and_traps() {
        let mut runtime = runtime_for(&[0xba, 0x00, 0x02, 0xb4, 0x09, 0xcd, 0x21]);
        runtime.run_slice(3);
        assert!(matches!(
            runtime.state(),
            GuestState::Trapped(Trap::UnterminatedDosString { .. })
        ));
    }

    #[test]
    fn bundled_hello_program_executes_as_guest_code() {
        let mut runtime = runtime_for(HELLO_CHRONOS_COM);
        runtime.run_slice(128);

        let output: [u8; 19] = core::array::from_fn(|index| runtime.text.cell(index, 0).character);
        assert_eq!(&output, b"Hello from Chronos!");
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
    }

    #[test]
    fn dos_version_returns_conservative_compatibility_value() {
        let mut runtime = runtime_for(&[0xb4, 0x30, 0xcd, 0x21]);
        runtime.run_slice(2);
        assert_eq!(runtime.cpu.ax, 0x0005);
    }

    #[test]
    fn int_20_terminates_successfully() {
        let mut runtime = runtime_for(&[0xcd, 0x20]);
        runtime.run_slice(1);
        assert_eq!(runtime.state(), &GuestState::Exited { code: 0 });
    }

    #[test]
    fn interrupt_fetch_uses_cs_ip() {
        let mut runtime = runtime_for(&[0x90]);
        runtime.cpu = CpuState {
            cs: PSP_SEGMENT,
            ip: 0x0100,
            ..runtime.cpu
        };
        runtime.step();
        assert_eq!(runtime.cpu.ip, 0x0101);
    }
}
