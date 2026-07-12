use crate::{CpuState, GuestMemory, TextModeSurface, Trap};

pub const DOS_VERSION_MAJOR: u8 = 5;
pub const DOS_VERSION_MINOR: u8 = 0;
const MAX_DOS_STRING_BYTES: usize = 0x1_0000;

pub enum InterruptResult {
    Continue,
    Exit(u8),
}

pub fn dispatch(
    interrupt: u8,
    cpu: &mut CpuState,
    memory: &GuestMemory,
    text: &mut TextModeSurface,
) -> Result<InterruptResult, Trap> {
    match interrupt {
        0x20 => Ok(InterruptResult::Exit(0)),
        0x21 => dispatch_dos(cpu, memory, text),
        0x10 => dispatch_bios(cpu, text),
        _ => Err(Trap::UnsupportedInterrupt {
            interrupt,
            function: cpu.ah(),
        }),
    }
}

fn dispatch_dos(
    cpu: &mut CpuState,
    memory: &GuestMemory,
    text: &mut TextModeSurface,
) -> Result<InterruptResult, Trap> {
    match cpu.ah() {
        0x02 => {
            text.write_byte(cpu.dl());
            Ok(InterruptResult::Continue)
        }
        0x09 => {
            for offset in 0..MAX_DOS_STRING_BYTES {
                let guest_offset = cpu.dx.wrapping_add(offset as u16);
                let byte = memory.read_u8(cpu.ds, guest_offset);
                if byte == b'$' {
                    return Ok(InterruptResult::Continue);
                }
                text.write_byte(byte);
            }
            Err(Trap::UnterminatedDosString {
                segment: cpu.ds,
                offset: cpu.dx,
                maximum: MAX_DOS_STRING_BYTES,
            })
        }
        0x30 => {
            cpu.set_al(DOS_VERSION_MAJOR);
            cpu.set_ah(DOS_VERSION_MINOR);
            Ok(InterruptResult::Continue)
        }
        0x4c => Ok(InterruptResult::Exit(cpu.al())),
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x21,
            function,
        }),
    }
}

fn dispatch_bios(cpu: &CpuState, text: &mut TextModeSurface) -> Result<InterruptResult, Trap> {
    match cpu.ah() {
        0x0e => {
            text.write_byte(cpu.al());
            Ok(InterruptResult::Continue)
        }
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x10,
            function,
        }),
    }
}
