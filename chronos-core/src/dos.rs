use crate::{CpuState, Runtime, Trap, DEFAULT_ATTRIBUTE, TEXT_COLUMNS};

pub const DOS_VERSION_MAJOR: u8 = 5;
pub const DOS_VERSION_MINOR: u8 = 0;
const MAX_DOS_STRING_BYTES: usize = 0x1_0000;

pub fn dispatch(runtime: &mut Runtime, interrupt: u8) -> Result<(), Trap> {
    match interrupt {
        0x20 => runtime.exit(0),
        0x10 => dispatch_video(runtime),
        0x16 => dispatch_keyboard(runtime),
        0x21 => dispatch_dos(runtime),
        _ => Err(Trap::UnsupportedInterrupt {
            interrupt,
            function: runtime.cpu.ah(),
        }),
    }
}

fn dispatch_video(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.cpu.ah() {
        0x00 if runtime.cpu.al() == 0x03 => {
            runtime.reset_video();
            Ok(())
        }
        0x00 => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x10,
            function: runtime.cpu.ah(),
        }),
        0x02 => {
            if runtime.cpu.bh() != 0 {
                return Err(Trap::UnsupportedInterrupt {
                    interrupt: 0x10,
                    function: runtime.cpu.ah(),
                });
            }
            let column = runtime.cpu.dl().min(79) as usize;
            let row = runtime.cpu.dh().min(24) as usize;
            runtime.set_cursor(column, row);
            Ok(())
        }
        0x03 => {
            runtime.cpu.set_bh(0);
            runtime.cpu.set_dh(runtime.cursor_row as u8);
            runtime.cpu.set_dl(runtime.cursor_column as u8);
            runtime.cpu.cx = runtime.cursor_shape;
            Ok(())
        }
        0x06 => {
            let top = runtime.cpu.ch() as usize;
            let left = runtime.cpu.cl() as usize;
            let bottom = runtime.cpu.dh() as usize;
            let right = runtime.cpu.dl() as usize;
            let lines = runtime.cpu.al() as usize;
            let attribute = runtime.cpu.bh();
            runtime.scroll_up(top, left, bottom, right, lines, attribute)?;
            Ok(())
        }
        0x08 => {
            let cell = runtime.cell(runtime.cursor_column, runtime.cursor_row);
            runtime.cpu.set_al(cell.character);
            runtime.cpu.set_ah(cell.attribute);
            Ok(())
        }
        0x09 => {
            if runtime.cpu.bh() != 0 {
                return Err(Trap::UnsupportedInterrupt {
                    interrupt: 0x10,
                    function: runtime.cpu.ah(),
                });
            }
            let count = runtime.cpu.cx;
            let character = runtime.cpu.al();
            let attribute = runtime.cpu.bl();
            for _ in 0..count {
                runtime.put_cell(
                    runtime.cursor_column,
                    runtime.cursor_row,
                    character,
                    attribute,
                );
                runtime.advance_cursor();
            }
            Ok(())
        }
        0x0e => {
            runtime.teletype(runtime.cpu.al(), DEFAULT_ATTRIBUTE);
            Ok(())
        }
        0x0f => {
            runtime.cpu.set_al(0x03);
            runtime.cpu.set_ah(TEXT_COLUMNS as u8);
            runtime.cpu.set_bh(0);
            Ok(())
        }
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x10,
            function,
        }),
    }
}

fn dispatch_keyboard(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.cpu.ah() {
        0x00 => runtime.wait_for_key(crate::runtime::InputMode::Bios),
        0x01 => {
            if let Some(key) = runtime.peek_key() {
                runtime.cpu.ax = u16::from_le_bytes([key.ascii, key.scan_code]);
                runtime.cpu.flags &= !CpuState::FLAG_ZF;
            } else {
                runtime.cpu.flags |= CpuState::FLAG_ZF;
            }
            Ok(())
        }
        0x02 => {
            runtime.cpu.set_al(runtime.shift_flags());
            Ok(())
        }
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x16,
            function,
        }),
    }
}

fn dispatch_dos(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.cpu.ah() {
        0x01 => runtime.wait_for_key(crate::runtime::InputMode::Dos { echo: true }),
        0x02 => {
            runtime.teletype(runtime.cpu.dl(), DEFAULT_ATTRIBUTE);
            Ok(())
        }
        0x06 if runtime.cpu.dl() != 0xff => {
            runtime.teletype(runtime.cpu.dl(), DEFAULT_ATTRIBUTE);
            Ok(())
        }
        0x06 => {
            if let Some(key) = runtime.pop_key() {
                runtime.cpu.set_al(key.ascii);
                runtime.cpu.flags &= !CpuState::FLAG_ZF;
            } else {
                runtime.cpu.flags |= CpuState::FLAG_ZF;
            }
            Ok(())
        }
        0x07 | 0x08 => runtime.wait_for_key(crate::runtime::InputMode::Dos { echo: false }),
        0x09 => {
            for offset in 0..MAX_DOS_STRING_BYTES {
                let guest_offset = runtime.cpu.dx.wrapping_add(offset as u16);
                let byte = runtime.memory.read_u8(runtime.cpu.ds, guest_offset);
                if byte == b'$' {
                    return Ok(());
                }
                runtime.teletype(byte, DEFAULT_ATTRIBUTE);
            }
            Err(Trap::UnterminatedDosString {
                segment: runtime.cpu.ds,
                offset: runtime.cpu.dx,
                maximum: MAX_DOS_STRING_BYTES,
            })
        }
        0x0a => {
            let maximum = runtime.memory.read_u8(runtime.cpu.ds, runtime.cpu.dx);
            runtime
                .memory
                .write_u8(runtime.cpu.ds, runtime.cpu.dx.wrapping_add(1), 0);
            runtime.wait_for_key(crate::runtime::InputMode::Line {
                segment: runtime.cpu.ds,
                offset: runtime.cpu.dx,
                maximum,
                length: 0,
            })
        }
        0x0b => {
            runtime.cpu.set_al(if runtime.has_key() { 0xff } else { 0 });
            Ok(())
        }
        0x0c => {
            runtime.clear_keys();
            let function = runtime.cpu.al();
            runtime.cpu.set_ah(function);
            dispatch_dos(runtime)
        }
        0x30 => {
            runtime.cpu.set_al(DOS_VERSION_MAJOR);
            runtime.cpu.set_ah(DOS_VERSION_MINOR);
            Ok(())
        }
        0x4c => runtime.exit(runtime.cpu.al()),
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x21,
            function,
        }),
    }
}
