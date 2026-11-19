use alloc::{vec, vec::Vec};

use crate::{
    CpuState, DosDrive, DosError, DosHandle, DosPath, GuestVideoMode, OpenMode,
    Runtime, TextAttributeMode, Trap, DEFAULT_ATTRIBUTE, TEXT_COLUMNS,
};

pub const DOS_VERSION_MAJOR: u8 = 5;
pub const DOS_VERSION_MINOR: u8 = 0;
const MAX_DOS_STRING_BYTES: usize = 0x1_0000;
const DOS_PATH_BYTES: usize = 241;
const DTA_RESULT_SIZE: usize = 43;

pub fn dispatch(runtime: &mut Runtime, interrupt: u8) -> Result<(), Trap> {
    // DOS multiplex and software-FPU hooks execute as guest code (IRET).
    // BIOS/DOS built-ins retain their existing dispatch until ROM chaining
    // trampolines are modeled. The saved IP is already past INT.
    let (segment, offset) = runtime.interrupt_vector(interrupt);
    let host_service = matches!(interrupt, 0x10 | 0x11 | 0x16 | 0x20 | 0x21 | 0x28 | 0x33);
    // The new video-addressing service permits guest interception. Older
    // INT 10h services retain their built-in routing until ROM chaining exists.
    let video_addressing =
        interrupt == 0x10 && runtime.cpu.ah() == 0x12 && runtime.cpu.bl() == 0x32;
    if (!host_service || video_addressing) && (segment != 0 || offset != 0) {
        runtime.enter_guest_interrupt(segment, offset);
        return Ok(());
    }
    match interrupt {
        0x20 => runtime.exit(0),
        0x10 => dispatch_video(runtime),
        0x11 => {
            // Virtual equipment: 80-column color display, no disk controllers,
            // serial/parallel devices, or numeric coprocessor.
            runtime.cpu.ax = runtime.memory.read_u16(0x0040, 0x0010);
            Ok(())
        }
        0x16 => dispatch_keyboard(runtime),
        0x21 => dispatch_dos(runtime),
        0x28 => {
            runtime.cooperative_yield();
            Ok(())
        }
        0x33 => crate::mouse::dispatch(runtime),
        _ => Err(Trap::UnsupportedInterrupt {
            interrupt,
            function: runtime.cpu.ah(),
        }),
    }
}

fn dispatch_video(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.cpu.ah() {
        0x00 => match runtime.cpu.al() {
            0x03 => {
                runtime.set_video_mode(GuestVideoMode::Text80x25Color);
                Ok(())
            }
            0x13 => {
                runtime.set_video_mode(GuestVideoMode::Vga320x200x256);
                Ok(())
            }
            mode => Err(Trap::UnsupportedVideoMode { mode }),
        },
        0x01 => {
            runtime.set_cursor_shape(runtime.cpu.cx);
            Ok(())
        }
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
            if runtime.cpu.bh() != 0 {
                return Err(Trap::UnsupportedInterrupt {
                    interrupt: 0x10,
                    function: 0x03,
                });
            }
            runtime.cpu.set_dh(runtime.cursor_row as u8);
            runtime.cpu.set_dl(runtime.cursor_column as u8);
            runtime.cpu.cx = runtime.cursor_shape;
            Ok(())
        }
        0x05 if runtime.cpu.al() == 0 => Ok(()),
        0x06 | 0x07 => {
            let top = runtime.cpu.ch() as usize;
            let left = runtime.cpu.cl() as usize;
            let bottom = runtime.cpu.dh() as usize;
            let right = runtime.cpu.dl() as usize;
            let lines = runtime.cpu.al() as usize;
            let attribute = runtime.cpu.bh();
            if runtime.cpu.ah() == 0x06 {
                runtime.scroll_up(top, left, bottom, right, lines, attribute)?;
            } else {
                runtime.scroll_down(top, left, bottom, right, lines, attribute)?;
            }
            Ok(())
        }
        0x08 => {
            if runtime.cpu.bh() != 0 {
                return Err(Trap::UnsupportedInterrupt {
                    interrupt: 0x10,
                    function: 0x08,
                });
            }
            let offset = ((runtime.cursor_row * TEXT_COLUMNS + runtime.cursor_column) * 2) as u16;
            runtime.cpu.ax = runtime.memory.read_u16(crate::VIDEO_SEGMENT, offset);
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
            // BIOS AH=09 writes sequential cells without moving or scrolling
            // the cursor. Clip at the end of the sole supported text page.
            let start = runtime.cursor_row * TEXT_COLUMNS + runtime.cursor_column;
            let end = (start + count as usize).min(TEXT_COLUMNS * crate::TEXT_ROWS);
            for index in start..end {
                runtime.put_cell(
                    index % TEXT_COLUMNS,
                    index / TEXT_COLUMNS,
                    character,
                    attribute,
                );
            }
            Ok(())
        }
        0x0e => {
            runtime.teletype(runtime.cpu.al(), DEFAULT_ATTRIBUTE);
            Ok(())
        }
        0x0f => {
            runtime.cpu.set_al(runtime.video_mode().bios_mode());
            runtime.cpu.set_ah(match runtime.video_mode() {
                GuestVideoMode::Text80x25Color => TEXT_COLUMNS as u8,
                GuestVideoMode::Vga320x200x256 => 40,
            });
            runtime.cpu.set_bh(0);
            Ok(())
        }
        0x10 if runtime.cpu.al() == 0x03 => {
            // INT 10h/AX=1003h selects text attribute bit-7 semantics:
            // BL=00h enables background intensity, BL=01h enables blinking.
            // BH is documented as 00h and is not an output; it is preserved.
            // Other BL values are undefined input and leave the state unchanged.
            let mode = match runtime.cpu.bl() {
                0x00 => Some(TextAttributeMode::BackgroundIntensity),
                0x01 => Some(TextAttributeMode::Blink),
                _ => None,
            };
            if let Some(mode) = mode {
                runtime.set_text_attribute_mode(mode);
            }
            Ok(())
        }
        0x11 if runtime.cpu.al() == 0x30 => crate::font::get_information(runtime),
        0x12 if runtime.cpu.bl() == 0x10 => {
            // IBM PS/2 and PC BIOS Interface Technical Reference, INT 10h
            // Alternate Select. AL is not an input or an output for BL=10h.
            // Conservative virtual subset: 64 KiB video aperture, color,
            // no feature connector, enhanced color display switch setting.
            runtime.cpu.bx = crate::video::EGA_MEMORY_CODE as u16;
            runtime.cpu.cx = u16::from_le_bytes([
                crate::video::EGA_SWITCH_SETTINGS,
                crate::video::EGA_FEATURE_BITS,
            ]);
            Ok(())
        }
        0x12 if runtime.cpu.bl() == 0x32 => {
            // IBM VGA/Phoenix: select CPU video address decoding. AL=12h
            // acknowledges a defined request; no carry status is specified.
            match runtime.cpu.al() {
                state @ (0 | 1) => {
                    runtime.memory.set_vga_addressing_enabled(state == 0);
                    runtime.cpu.set_al(0x12);
                }
                // The references do not specify invalid-input status.
                // Return no support signature and leave decoding unchanged.
                _ => runtime.cpu.set_al(0),
            }
            Ok(())
        }
        0x1c => dispatch_video_state(runtime),
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x10,
            function,
        }),
    }
}

/// Largest save buffer this VGA profile can produce. Chronos models the BIOS
/// data area only, so the documented 20h-byte header is followed by the
/// 96-byte video BDA range.
const VIDEO_STATE_BUFFER_BYTES: usize =
    crate::video::VIDEO_STATE_HEADER_BYTES + crate::video::VIDEO_STATE_BIOS_DATA_BYTES;

/// INT 10h/AH=1Ch: VGA save/restore video state.
///
/// `CX` bits select the state areas. Chronos supports bit 1 only and rejects
/// every other mask with `AL=00h` rather than advertising state it cannot
/// reproduce. `AL` values above 02h return `AL=00h` with `AH` preserved.
/// Unaffected registers and flags are left untouched.
fn dispatch_video_state(runtime: &mut Runtime) -> Result<(), Trap> {
    let subfunction = runtime.cpu.al();
    let mask = runtime.cpu.cx;
    if subfunction > 0x02 {
        runtime.cpu.set_al(0);
        return Ok(());
    }
    let Some(buffer_bytes) = crate::video::video_state_buffer_bytes(mask) else {
        runtime.cpu.set_al(0);
        return Ok(());
    };
    debug_assert_eq!(buffer_bytes, VIDEO_STATE_BUFFER_BYTES);
    match subfunction {
        0x00 => {
            runtime.cpu.bx = match crate::video::video_state_buffer_blocks(mask) {
                Some(blocks) => blocks,
                None => {
                    runtime.cpu.set_al(0);
                    return Ok(());
                }
            };
            runtime.cpu.set_al(0x1c);
            Ok(())
        }
        0x01 => {
            let (segment, offset) = (runtime.cpu.es, runtime.cpu.bx);
            let Some(start) =
                crate::GuestMemory::checked_buffer_start(segment, offset, VIDEO_STATE_BUFFER_BYTES)
            else {
                runtime.cpu.set_al(0);
                return Ok(());
            };
            if !runtime.memory.range_is_ram(start, VIDEO_STATE_BUFFER_BYTES) {
                runtime.cpu.set_al(0);
                return Ok(());
            }
            let mut buffer = [0u8; VIDEO_STATE_BUFFER_BYTES];
            let bda_offset = crate::video::VIDEO_STATE_BIOS_DATA_OFFSET;
            buffer[2..4].copy_from_slice(&(bda_offset as u16).to_le_bytes());
            let mut bda = [0u8; crate::video::VIDEO_STATE_BIOS_DATA_BYTES];
            if !runtime.read_video_bda(&mut bda) {
                runtime.cpu.set_al(0);
                return Ok(());
            }
            buffer[bda_offset..].copy_from_slice(&bda);
            if runtime
                .memory
                .write_slice(segment, offset, &buffer)
                .is_err()
            {
                runtime.cpu.set_al(0);
                return Ok(());
            }
            runtime.cpu.set_al(0x1c);
            Ok(())
        }
        _ => {
            let (segment, offset) = (runtime.cpu.es, runtime.cpu.bx);
            let Some(start) =
                crate::GuestMemory::checked_buffer_start(segment, offset, VIDEO_STATE_BUFFER_BYTES)
            else {
                runtime.cpu.set_al(0);
                return Ok(());
            };
            if !runtime.memory.range_is_ram(start, VIDEO_STATE_BUFFER_BYTES) {
                runtime.cpu.set_al(0);
                return Ok(());
            }
            let mut buffer = [0u8; VIDEO_STATE_BUFFER_BYTES];
            if runtime
                .memory
                .read_slice(segment, offset, &mut buffer)
                .is_err()
            {
                runtime.cpu.set_al(0);
                return Ok(());
            }
            let header = |index: usize| u16::from_le_bytes([buffer[index], buffer[index + 1]]);
            let bda_offset = crate::video::VIDEO_STATE_BIOS_DATA_OFFSET;
            if header(0) != 0 || header(2) != bda_offset as u16 || header(4) != 0 || header(6) != 0
            {
                runtime.cpu.set_al(0);
                return Ok(());
            }
            let mut bda = [0u8; crate::video::VIDEO_STATE_BIOS_DATA_BYTES];
            bda.copy_from_slice(&buffer[bda_offset..]);
            if !runtime.restore_video_bda(&bda) {
                runtime.cpu.set_al(0);
                return Ok(());
            }
            // The BDA is always writable guest RAM; this cannot fail after the
            // range validation above.
            let _ = runtime.memory.write_slice(0x0040, 0x0049, &bda);
            runtime.cpu.set_al(0x1c);
            Ok(())
        }
    }
}

fn dispatch_keyboard(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.cpu.ah() {
        // The current input pipeline stores unified BIOS key events (ASCII
        // plus scan code), including ASCII-zero extended keys.  It therefore
        // uses the same queue for legacy and enhanced reads and status calls.
        0x00 | 0x10 => runtime.wait_for_key(crate::runtime::InputMode::Bios),
        0x01 | 0x11 => {
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
            dos_success(runtime)
        }
        0x38 => country_information(runtime),
        0x33 => match runtime.cpu.al() {
            0 => {
                runtime.cpu.set_dl(u8::from(runtime.break_check_enabled));
                Ok(())
            }
            1 => {
                runtime.break_check_enabled = runtime.cpu.dl() != 0;
                Ok(())
            }
            _ => {
                runtime.cpu.set_al(0xff);
                Ok(())
            }
        },
        0x0e => {
            let Some(drive) = DosDrive::from_number(runtime.cpu.dl()) else {
                return dos_error(runtime, DosError::InvalidDrive);
            };
            match runtime.drives.select(drive) {
                Ok(()) => {
                    runtime.cpu.set_al(runtime.drives.mounted_count());
                    dos_success(runtime)
                }
                Err(error) => dos_error(runtime, error),
            }
        }
        0x19 => {
            runtime.cpu.set_al(runtime.drives.current_drive.number());
            dos_success(runtime)
        }
        0x36 => {
            let drive = match runtime.cpu.dl().checked_sub(1) {
                None => Some(runtime.drives.current_drive),
                Some(number) => DosDrive::from_number(number),
            };
            match drive
                .and_then(|drive| runtime.drives.drive_space(drive))
                .and_then(crate::DriveSpace::dos_registers)
            {
                Some(registers) => {
                    (runtime.cpu.ax, runtime.cpu.bx, runtime.cpu.cx, runtime.cpu.dx) = registers;
                }
                None => runtime.cpu.ax = u16::MAX,
            }
            Ok(())
        }
        0x25 => {
            runtime.set_interrupt_vector(runtime.cpu.al(), runtime.cpu.ds, runtime.cpu.dx);
            dos_success(runtime)
        }
        0x35 => {
            let (segment, offset) = runtime.interrupt_vector(runtime.cpu.al());
            runtime.cpu.es = segment;
            runtime.cpu.bx = offset;
            dos_success(runtime)
        }
        0x1a => {
            runtime.dta_segment = runtime.cpu.ds;
            runtime.dta_offset = runtime.cpu.dx;
            runtime.active_process.dta_segment = runtime.dta_segment;
            runtime.active_process.dta_offset = runtime.dta_offset;
            dos_success(runtime)
        }
        0x2f => {
            runtime.cpu.es = runtime.dta_segment;
            runtime.cpu.bx = runtime.dta_offset;
            dos_success(runtime)
        }
        0x29 => parse_filename_into_fcb(runtime),
        0x2a => get_date(runtime),
        0x2c => get_time(runtime),
        0x39 => {
            let path = read_dos_path(runtime);
            match path.and_then(|path| runtime.drives.mkdir(&path)) {
                Ok(()) => dos_success(runtime),
                Err(error) => dos_error(runtime, error),
            }
        }
        0x3a => {
            let path = read_dos_path(runtime);
            match path.and_then(|path| runtime.drives.rmdir(&path)) {
                Ok(()) => dos_success(runtime),
                Err(error) => dos_error(runtime, error),
            }
        }
        0x3b => {
            let path = read_dos_path(runtime);
            match path.and_then(|path| runtime.drives.change_directory(&path)) {
                Ok(()) => dos_success(runtime),
                Err(error) => dos_error(runtime, error),
            }
        }
        0x3c => create_file(runtime),
        0x3d => open_file(runtime),
        0x3e => match runtime.handles.close(runtime.cpu.bx) {
            Ok(()) => dos_success(runtime),
            Err(error) => dos_error(runtime, error),
        },
        0x3f => read_handle(runtime),
        0x40 => write_handle(runtime),
        0x41 => {
            let path = read_dos_path(runtime);
            match path.and_then(|path| runtime.drives.delete_file(&path)) {
                Ok(()) => dos_success(runtime),
                Err(error) => dos_error(runtime, error),
            }
        }
        0x42 => seek_handle(runtime),
        0x43 => attributes(runtime),
        0x44 => ioctl(runtime),
        0x45 => duplicate_handle(runtime),
        0x46 => force_duplicate_handle(runtime),
        0x47 => get_current_directory(runtime),
        0x48 => allocate_memory(runtime),
        0x49 => free_memory(runtime),
        0x4a => resize_memory(runtime),
        0x4b => exec(runtime),
        0x4d => child_result(runtime),
        0x50 => {
            if runtime.set_current_psp(runtime.cpu.bx) {
                dos_success(runtime)
            } else {
                dos_error(runtime, DosError::InvalidMemoryBlock)
            }
        }
        0x51 | 0x62 => {
            runtime.cpu.bx = runtime.current_psp();
            dos_success(runtime)
        }
        0x52 => {
            // Minimal DOS 3+ list-of-lists / SFT capacity view. Reserved low
            // guest RAM is outside the process arena, IVT, and BIOS data area.
            // Only the SFT chain and capacity are currently modeled, not DOS
            // internal device/DPB tables or live native file descriptors.
            const SEGMENT: u16 = 0x0800;
            const SFT_OFFSET: u16 = 0x0100;
            runtime.memory.write_u16(SEGMENT, 4, SFT_OFFSET);
            runtime.memory.write_u16(SEGMENT, 6, SEGMENT);
            runtime.memory.write_u16(SEGMENT, SFT_OFFSET, 0xffff);
            runtime.memory.write_u16(SEGMENT, SFT_OFFSET + 2, 0xffff);
            runtime
                .memory
                .write_u16(SEGMENT, SFT_OFFSET + 4, crate::fs::MAX_OPEN_HANDLES as u16);
            runtime.cpu.es = SEGMENT;
            runtime.cpu.bx = 0;
            Ok(())
        }
        0x71 => dos_error(runtime, DosError::InvalidFunction),
        0x4e => find_first(runtime),
        0x4f => find_next(runtime),
        0x56 => rename_file(runtime),
        0x59 => {
            runtime.cpu.ax = 0;
            runtime.cpu.set_bh(0);
            runtime.cpu.set_bl(0);
            runtime.cpu.set_ch(0);
            dos_success(runtime)
        }
        0x00 => runtime.exit(0),
        0x4c => runtime.exit(runtime.cpu.al()),
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x21,
            function,
        }),
    }
}

/// DOS AH=29h filename parser used by conventional runtimes while searching
/// PATH. This fills the drive/name/extension portion of an unopened FCB and
/// leaves DS:SI at the first delimiter after the parsed 8.3 name.
fn parse_filename_into_fcb(runtime: &mut Runtime) -> Result<(), Trap> {
    let control = runtime.cpu.al();
    let segment = runtime.cpu.ds;
    let mut source = runtime.cpu.si;
    let destination_segment = runtime.cpu.es;
    let destination = runtime.cpu.di;
    let mut remaining = DOS_PATH_BYTES;

    if control & 0x01 != 0 {
        while remaining != 0 {
            let byte = runtime.memory.read_u8(segment, source);
            if matches!(byte, b' ' | b'\t' | b';' | b',' | b'=' | b'+') {
                source = source.wrapping_add(1);
                remaining -= 1;
            } else {
                break;
            }
        }
    }

    let first = runtime.memory.read_u8(segment, source);
    let second = runtime.memory.read_u8(segment, source.wrapping_add(1));
    if second == b':' {
        if first.is_ascii_alphabetic() {
            runtime.memory.write_u8(
                destination_segment,
                destination,
                first.to_ascii_uppercase() - b'A' + 1,
            );
            source = source.wrapping_add(2);
            remaining = remaining.saturating_sub(2);
        } else {
            runtime.cpu.set_al(0xff);
            runtime.cpu.si = source;
            return Ok(());
        }
    } else if control & 0x02 == 0 {
        runtime.memory.write_u8(destination_segment, destination, 0);
    }

    if control & 0x04 == 0 {
        for index in 0..8 {
            runtime.memory.write_u8(
                destination_segment,
                destination.wrapping_add(1 + index),
                b' ',
            );
        }
    }
    if control & 0x08 == 0 {
        for index in 0..3 {
            runtime.memory.write_u8(
                destination_segment,
                destination.wrapping_add(9 + index),
                b' ',
            );
        }
    }

    let mut wildcard = false;
    let mut name_index = 0u16;
    while name_index < 8 && remaining != 0 {
        let byte = runtime.memory.read_u8(segment, source);
        if byte == b'*' {
            wildcard = true;
            while name_index < 8 {
                runtime.memory.write_u8(
                    destination_segment,
                    destination.wrapping_add(1 + name_index),
                    b'?',
                );
                name_index += 1;
            }
            source = source.wrapping_add(1);
            remaining -= 1;
            break;
        }
        if byte == b'?' {
            wildcard = true;
        }
        if byte == b'.' || is_fcb_delimiter(byte) {
            break;
        }
        runtime.memory.write_u8(
            destination_segment,
            destination.wrapping_add(1 + name_index),
            byte.to_ascii_uppercase(),
        );
        name_index += 1;
        source = source.wrapping_add(1);
        remaining -= 1;
    }
    // Consume overlong name characters deterministically instead of allowing
    // them to spill into the extension field.
    while remaining != 0 && {
        let byte = runtime.memory.read_u8(segment, source);
        byte != b'.' && !is_fcb_delimiter(byte)
    } {
        source = source.wrapping_add(1);
        remaining -= 1;
    }

    if remaining != 0 && runtime.memory.read_u8(segment, source) == b'.' {
        source = source.wrapping_add(1);
        remaining -= 1;
        let mut extension_index = 0u16;
        while extension_index < 3 && remaining != 0 {
            let byte = runtime.memory.read_u8(segment, source);
            if byte == b'*' {
                wildcard = true;
                while extension_index < 3 {
                    runtime.memory.write_u8(
                        destination_segment,
                        destination.wrapping_add(9 + extension_index),
                        b'?',
                    );
                    extension_index += 1;
                }
                source = source.wrapping_add(1);
                remaining -= 1;
                break;
            }
            if byte == b'?' {
                wildcard = true;
            }
            if is_fcb_delimiter(byte) || byte == b'.' {
                break;
            }
            runtime.memory.write_u8(
                destination_segment,
                destination.wrapping_add(9 + extension_index),
                byte.to_ascii_uppercase(),
            );
            extension_index += 1;
            source = source.wrapping_add(1);
            remaining -= 1;
        }
        while remaining != 0 && !is_fcb_delimiter(runtime.memory.read_u8(segment, source)) {
            source = source.wrapping_add(1);
            remaining -= 1;
        }
    }

    runtime.cpu.si = source;
    runtime.cpu.set_al(if wildcard { 1 } else { 0 });
    Ok(())
}

fn is_fcb_delimiter(byte: u8) -> bool {
    byte == 0
        || byte <= b' '
        || matches!(
            byte,
            b'"' | b'/' | b'\\' | b'[' | b']' | b':' | b';' | b',' | b'=' | b'+'
        )
}

fn allocate_memory(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.allocate_memory(runtime.cpu.bx) {
        Ok(segment) => {
            runtime.cpu.ax = segment;
            dos_success(runtime)
        }
        Err(crate::ArenaError::InsufficientMemory { largest }) => {
            runtime.cpu.ax = DosError::InsufficientMemory.code();
            runtime.cpu.bx = largest;
            runtime.cpu.flags |= CpuState::FLAG_CF;
            Ok(())
        }
        Err(_) => {
            runtime.cpu.ax = DosError::InsufficientMemory.code();
            runtime.cpu.bx = runtime.largest_available_memory();
            runtime.cpu.flags |= CpuState::FLAG_CF;
            Ok(())
        }
    }
}

fn free_memory(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.free_memory(runtime.cpu.es) {
        Ok(()) => dos_success(runtime),
        Err(_) => dos_error(runtime, DosError::InvalidMemoryBlock),
    }
}

fn resize_memory(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.resize_memory(runtime.cpu.es, runtime.cpu.bx) {
        Ok(()) => dos_success(runtime),
        Err(crate::ArenaError::InsufficientMemory { largest }) => {
            runtime.cpu.ax = DosError::InsufficientMemory.code();
            runtime.cpu.bx = largest;
            runtime.cpu.flags |= CpuState::FLAG_CF;
            Ok(())
        }
        Err(_) => dos_error(runtime, DosError::InvalidMemoryBlock),
    }
}

fn exec(runtime: &mut Runtime) -> Result<(), Trap> {
    if runtime.cpu.al() != 0 {
        return dos_error(runtime, DosError::AccessDenied);
    }
    if !runtime
        .arena
        .owns_range(runtime.current_psp(), runtime.cpu.es, runtime.cpu.bx, 14)
    {
        return dos_error(runtime, DosError::AccessDenied);
    }
    let parameter = runtime.cpu.bx;
    let environment = runtime.memory.read_u16(runtime.cpu.es, parameter);
    let tail_offset = runtime
        .memory
        .read_u16(runtime.cpu.es, parameter.wrapping_add(2));
    let tail_segment = runtime
        .memory
        .read_u16(runtime.cpu.es, parameter.wrapping_add(4));
    let path = match read_dos_path(runtime) {
        Ok(path) => path,
        Err(error) => return dos_error(runtime, error),
    };
    let command_tail = match read_command_tail(runtime, tail_segment, tail_offset) {
        Ok(value) => value,
        Err(error) => return dos_error(runtime, error),
    };
    match runtime.exec(path, &command_tail, environment) {
        Ok(()) => dos_success(runtime),
        Err(crate::LoaderError::InsufficientMemory { largest, .. }) => {
            runtime.cpu.ax = DosError::InsufficientMemory.code();
            runtime.cpu.bx = largest;
            runtime.cpu.flags |= CpuState::FLAG_CF;
            Ok(())
        }
        Err(_) => dos_error(runtime, DosError::FileNotFound),
    }
}

fn read_command_tail(runtime: &Runtime, segment: u16, offset: u16) -> Result<Vec<u8>, DosError> {
    if segment == 0 && offset == 0 {
        return Ok(Vec::new());
    }
    if !runtime
        .arena
        .owns_range(runtime.current_psp(), segment, offset, 1)
    {
        return Err(DosError::AccessDenied);
    }
    let length = runtime.memory.read_u8(segment, offset) as usize;
    if length > 126
        || !runtime
            .arena
            .owns_range(runtime.current_psp(), segment, offset, length + 2)
    {
        return Err(DosError::AccessDenied);
    }
    let mut tail = vec![0; length];
    runtime
        .memory
        .read_slice(segment, offset.wrapping_add(1), &mut tail)
        .map_err(|_| DosError::AccessDenied)?;
    Ok(tail)
}

fn child_result(runtime: &mut Runtime) -> Result<(), Trap> {
    let result = runtime
        .active_process
        .child_result
        .take()
        .unwrap_or(crate::ChildResult {
            code: 0,
            termination: crate::TerminationType::Normal,
        });
    runtime.last_delivered_child_result = Some(result);
    runtime.cpu.set_al(result.code);
    runtime.cpu.set_ah(result.termination as u8);
    dos_success(runtime)
}

// DOS 5 country 1 / OEM CP437. Layout: RBIL INT 21/AH=38, table 01399.
fn country_information(runtime: &mut Runtime) -> Result<(), Trap> {
    let country = match runtime.cpu.al() {
        0 => 1,
        0xff => runtime.cpu.bx,
        value => u16::from(value),
    };
    if country != 1 {
        return dos_error(runtime, DosError::FileNotFound); // DOS error 02: unknown country
    }
    // DX=FFFF selects the set-country operation, without a buffer write.
    if runtime.cpu.dx == 0xffff {
        return dos_success(runtime);
    }
    // DOS-owned low memory, below the conventional arena; separate from SFT.
    const SEGMENT: u16 = 0x0800;
    const CASEMAP: u16 = 0x0200;
    // FAR-callable AL mapping, preserving flags and all other registers.
    let mut code = vec![0x9c]; // PUSHF
    for &(lower, upper) in CP437_UPPERCASE {
        code.extend_from_slice(&[0x3c, lower, 0x75, 4, 0xb0, upper, 0x9d, 0xcb]);
    }
    code.extend_from_slice(&[0x9d, 0xcb]); // POPF; RETF
    let _ = runtime.memory.write_slice(SEGMENT, CASEMAP, &code);
    let mut info = [0u8; 34];
    info[2] = b'$';
    info[7] = b',';
    info[9] = b'.';
    info[11] = b'/';
    info[13] = b':';
    info[16] = 2;
    info[18..20].copy_from_slice(&CASEMAP.to_le_bytes());
    info[20..22].copy_from_slice(&SEGMENT.to_le_bytes());
    info[22] = b',';
    let _ = runtime
        .memory
        .write_slice(runtime.cpu.ds, runtime.cpu.dx, &info);
    runtime.cpu.bx = 1;
    dos_success(runtime)
}

const CP437_UPPERCASE: &[(u8, u8)] = &[
    (0x81, 0x9a),
    (0x82, 0x90),
    (0x84, 0x8e),
    (0x86, 0x8f),
    (0x87, 0x80),
    (0x91, 0x92),
    (0x94, 0x99),
    (0xa4, 0xa5),
    (0xe5, 0xe4),
    (0xed, 0xe8),
];

fn dos_success(runtime: &mut Runtime) -> Result<(), Trap> {
    runtime.cpu.flags &= !CpuState::FLAG_CF;
    Ok(())
}

fn dos_error(runtime: &mut Runtime, error: DosError) -> Result<(), Trap> {
    runtime.cpu.flags |= CpuState::FLAG_CF;
    runtime.cpu.ax = error.code();
    Ok(())
}

fn read_dos_path(runtime: &Runtime) -> Result<DosPath, DosError> {
    let mut bytes = vec![0u8; DOS_PATH_BYTES];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = runtime
            .memory
            .read_u8(runtime.cpu.ds, runtime.cpu.dx.wrapping_add(index as u16));
        if *byte == 0 {
            return runtime.drives.parse_path(&bytes[..index]);
        }
    }
    Err(DosError::PathNotFound)
}

fn read_dos_path_at(runtime: &Runtime, segment: u16, offset: u16) -> Result<DosPath, DosError> {
    let mut bytes = vec![0u8; DOS_PATH_BYTES];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = runtime
            .memory
            .read_u8(segment, offset.wrapping_add(index as u16));
        if *byte == 0 {
            return runtime.drives.parse_path(&bytes[..index]);
        }
    }
    Err(DosError::PathNotFound)
}

fn create_file(runtime: &mut Runtime) -> Result<(), Trap> {
    let attributes = runtime.cpu.cx as u8;
    match read_dos_path(runtime).and_then(|path| {
        runtime.drives.create_file(&path, attributes)?;
        runtime.handles.open(path, OpenMode::ReadWrite)
    }) {
        Ok(handle) => {
            runtime.cpu.ax = handle;
            dos_success(runtime)
        }
        Err(error) => dos_error(runtime, error),
    }
}

fn open_file(runtime: &mut Runtime) -> Result<(), Trap> {
    let mode = match OpenMode::from_dos(runtime.cpu.al()) {
        Ok(mode) => mode,
        Err(error) => return dos_error(runtime, error),
    };
    match read_dos_path(runtime).and_then(|path| {
        if mode.can_write() {
            runtime.drives.open_for_write(&path)?;
        } else {
            runtime.drives.read_file(&path)?;
        }
        runtime.handles.open(path, mode)
    }) {
        Ok(handle) => {
            runtime.cpu.ax = handle;
            dos_success(runtime)
        }
        Err(error) => dos_error(runtime, error),
    }
}

fn read_handle(runtime: &mut Runtime) -> Result<(), Trap> {
    let handle = runtime.cpu.bx;
    let requested = runtime.cpu.cx as usize;
    let descriptor = match runtime.handles.get(handle) {
        Ok(value) => value.clone(),
        Err(error) => return dos_error(runtime, error),
    };
    match descriptor {
        DosHandle::ConsoleInput => {
            if requested == 0 {
                runtime.cpu.ax = 0;
            } else {
                return runtime.wait_for_console_read(runtime.cpu.ds, runtime.cpu.dx);
            }
            dos_success(runtime)
        }
        DosHandle::ConsoleOutput => dos_error(runtime, DosError::AccessDenied),
        DosHandle::File {
            path,
            position,
            mode,
        } => {
            if !mode.can_read() {
                return dos_error(runtime, DosError::AccessDenied);
            }
            let data = match runtime.drives.read_file(&path) {
                Ok(data) => data,
                Err(error) => return dos_error(runtime, error),
            };
            let start = position.min(data.len());
            let count = requested.min(data.len() - start);
            if runtime
                .memory
                .write_slice(runtime.cpu.ds, runtime.cpu.dx, &data[start..start + count])
                .is_err()
            {
                return dos_error(runtime, DosError::InsufficientMemory);
            }
            if let Ok(DosHandle::File { position, .. }) = runtime.handles.get_mut(handle) {
                *position = start + count;
            }
            runtime.cpu.ax = count as u16;
            dos_success(runtime)
        }
    }
}

fn write_handle(runtime: &mut Runtime) -> Result<(), Trap> {
    let handle = runtime.cpu.bx;
    let requested = runtime.cpu.cx as usize;
    let mut bytes = vec![0u8; requested];
    if runtime
        .memory
        .read_slice(runtime.cpu.ds, runtime.cpu.dx, &mut bytes)
        .is_err()
    {
        return dos_error(runtime, DosError::InsufficientMemory);
    }
    let descriptor = match runtime.handles.get(handle) {
        Ok(value) => value.clone(),
        Err(error) => return dos_error(runtime, error),
    };
    match descriptor {
        DosHandle::ConsoleInput => dos_error(runtime, DosError::AccessDenied),
        DosHandle::ConsoleOutput => {
            for byte in bytes {
                runtime.teletype(byte, DEFAULT_ATTRIBUTE);
            }
            runtime.cpu.ax = requested as u16;
            dos_success(runtime)
        }
        DosHandle::File {
            path,
            position,
            mode,
        } => {
            if !mode.can_write() {
                return dos_error(runtime, DosError::AccessDenied);
            }
            let written = match runtime
                .drives
                .write_file(&path, position, &bytes, requested == 0)
            {
                Ok(value) => value,
                Err(error) => return dos_error(runtime, error),
            };
            if let Ok(DosHandle::File { position, .. }) = runtime.handles.get_mut(handle) {
                *position = position.saturating_add(written);
            }
            runtime.cpu.ax = written as u16;
            dos_success(runtime)
        }
    }
}

fn seek_handle(runtime: &mut Runtime) -> Result<(), Trap> {
    let handle = runtime.cpu.bx;
    let origin = runtime.cpu.al();
    let offset = i32::from_be_bytes([
        runtime.cpu.cx.to_be_bytes()[0],
        runtime.cpu.cx.to_be_bytes()[1],
        runtime.cpu.dx.to_be_bytes()[0],
        runtime.cpu.dx.to_be_bytes()[1],
    ]) as i64;
    let descriptor = match runtime.handles.get(handle) {
        Ok(value) => value.clone(),
        Err(error) => return dos_error(runtime, error),
    };
    let DosHandle::File {
        path,
        position,
        mode: _,
    } = descriptor
    else {
        return dos_error(runtime, DosError::InvalidHandle);
    };
    let base = match origin {
        0 => 0i64,
        1 => position.min(i64::MAX as usize) as i64,
        2 => match runtime.drives.file_len(&path) {
            Ok(value) => value.min(i64::MAX as usize) as i64,
            Err(error) => return dos_error(runtime, error),
        },
        _ => return dos_error(runtime, DosError::AccessDenied),
    };
    let Some(new_position) = base.checked_add(offset) else {
        return dos_error(runtime, DosError::AccessDenied);
    };
    if new_position < 0 {
        return dos_error(runtime, DosError::AccessDenied);
    }
    if let Ok(DosHandle::File { position, .. }) = runtime.handles.get_mut(handle) {
        *position = new_position as usize;
    }
    let output = new_position as u32;
    runtime.cpu.dx = (output >> 16) as u16;
    runtime.cpu.ax = output as u16;
    dos_success(runtime)
}

fn duplicate_handle(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.handles.duplicate(runtime.cpu.bx) {
        Ok(handle) => {
            runtime.cpu.ax = handle;
            dos_success(runtime)
        }
        Err(error) => dos_error(runtime, error),
    }
}

fn force_duplicate_handle(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime
        .handles
        .force_duplicate(runtime.cpu.bx, runtime.cpu.cx)
    {
        Ok(()) => dos_success(runtime),
        Err(error) => dos_error(runtime, error),
    }
}

fn ioctl(runtime: &mut Runtime) -> Result<(), Trap> {
    match runtime.cpu.al() {
        0x00 => {
            let descriptor = match runtime.handles.get(runtime.cpu.bx) {
                Ok(value) => value,
                Err(error) => return dos_error(runtime, error),
            };
            runtime.cpu.dx = match descriptor {
                DosHandle::ConsoleInput | DosHandle::ConsoleOutput => 0x0080,
                DosHandle::File { .. } => 0,
            };
            dos_success(runtime)
        }
        _ => dos_error(runtime, DosError::InvalidFunction),
    }
}

fn get_date(runtime: &mut Runtime) -> Result<(), Trap> {
    let date = runtime.guest_date();
    runtime.cpu.set_al(date.weekday);
    runtime.cpu.cx = date.year;
    runtime.cpu.set_dh(date.month);
    runtime.cpu.set_dl(date.day);
    dos_success(runtime)
}

fn get_time(runtime: &mut Runtime) -> Result<(), Trap> {
    let time = runtime.guest_time();
    runtime.cpu.set_ch(time.hour);
    runtime.cpu.set_cl(time.minute);
    runtime.cpu.set_dh(time.second);
    runtime.cpu.set_dl(time.hundredths);
    dos_success(runtime)
}

fn attributes(runtime: &mut Runtime) -> Result<(), Trap> {
    let subfunction = runtime.cpu.al();
    let path = read_dos_path(runtime);
    match (subfunction, path) {
        (0, Ok(path)) => match runtime.drives.get_attributes(&path) {
            Ok(attributes) => {
                runtime.cpu.cx = attributes as u16;
                dos_success(runtime)
            }
            Err(error) => dos_error(runtime, error),
        },
        (1, Ok(path)) => match runtime.drives.set_attributes(&path, runtime.cpu.cx as u8) {
            Ok(()) => dos_success(runtime),
            Err(error) => dos_error(runtime, error),
        },
        (_, _) => dos_error(runtime, DosError::AccessDenied),
    }
}

fn get_current_directory(runtime: &mut Runtime) -> Result<(), Trap> {
    let drive = if runtime.cpu.dl() == 0 {
        runtime.drives.current_drive
    } else {
        match DosDrive::from_number(runtime.cpu.dl() - 1) {
            Some(drive) => drive,
            None => return dos_error(runtime, DosError::InvalidDrive),
        }
    };
    let directory = match runtime.drives.current_directory(drive) {
        Ok(value) => value.as_bytes(),
        Err(error) => return dos_error(runtime, error),
    };
    if directory.len() + 1 > 64 {
        return dos_error(runtime, DosError::InsufficientMemory);
    }
    for (index, byte) in directory.iter().enumerate() {
        runtime.memory.write_u8(
            runtime.cpu.ds,
            runtime.cpu.si.wrapping_add(index as u16),
            *byte,
        );
    }
    runtime.memory.write_u8(
        runtime.cpu.ds,
        runtime.cpu.si.wrapping_add(directory.len() as u16),
        0,
    );
    dos_success(runtime)
}

fn find_first(runtime: &mut Runtime) -> Result<(), Trap> {
    runtime.active_process.searches.clear();
    runtime.active_process.search_index = 0;
    if !valid_search_dta(runtime) {
        return dos_error(runtime, DosError::InvalidMemoryBlock);
    }
    let source = read_dos_path(runtime);
    let path = match source {
        Ok(path) => path,
        Err(error) => return dos_error(runtime, error),
    };
    let mut directory = path.clone();
    let pattern = path.filename().as_bytes().to_vec();
    if pattern.is_empty() {
        return dos_error(runtime, DosError::PathNotFound);
    }
    directory.relative = path.parent();
    let entries = match runtime.drives.list(&directory, &pattern, runtime.cpu.cx) {
        Ok(entries) => entries,
        Err(error) => return dos_error(runtime, error),
    };
    runtime.active_process.searches = entries;
    write_next_search(runtime)
}

fn find_next(runtime: &mut Runtime) -> Result<(), Trap> {
    write_next_search(runtime)
}

fn valid_search_dta(runtime: &Runtime) -> bool {
    usize::from(runtime.dta_offset) + DTA_RESULT_SIZE <= 0x10000
}

fn write_next_search(runtime: &mut Runtime) -> Result<(), Trap> {
    if !valid_search_dta(runtime) {
        return dos_error(runtime, DosError::InvalidMemoryBlock);
    }
    let process = &mut runtime.active_process;
    let Some(entry) = process.searches.get(process.search_index) else {
        process.searches.clear();
        process.search_index = 0;
        return dos_error(runtime, DosError::NoMoreFiles);
    };
    let mut result = [0u8; DTA_RESULT_SIZE];
    result[21] = entry.attributes;
    // The file map does not carry modification times. Leave both fields unset.
    result[26..30].copy_from_slice(&entry.size.to_le_bytes());
    let name = entry.name.as_bytes();
    result[30..30 + name.len()].copy_from_slice(name);
    runtime
        .memory
        .write_slice(runtime.dta_segment, runtime.dta_offset, &result)
        .expect("fixed-size DOS directory result");
    process.search_index += 1;
    dos_success(runtime)
}

fn rename_file(runtime: &mut Runtime) -> Result<(), Trap> {
    let old = read_dos_path_at(runtime, runtime.cpu.ds, runtime.cpu.dx);
    let new = read_dos_path_at(runtime, runtime.cpu.es, runtime.cpu.di);
    match old.and_then(|old| new.and_then(|new| runtime.drives.rename(&old, &new))) {
        Ok(()) => dos_success(runtime),
        Err(error) => dos_error(runtime, error),
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch;
    use crate::{BiosKey, CpuState, DosDrive, DosError, Runtime, PSP_SEGMENT};

    #[test]
    fn enhanced_keyboard_read_consumes_in_order_and_waits_when_empty() {
        let mut runtime = Runtime::from_com(&[0xcd, 0x16, 0xcd, 0x16, 0xcd, 0x16]).unwrap();
        runtime.cpu.flags = 0x0247;
        runtime.cpu.bx = 0x1234;
        let arrow = BiosKey { ascii: 0, scan_code: 0x50 };
        let letter = BiosKey { ascii: b'a', scan_code: 0x1e };
        runtime.inject_key(arrow);
        runtime.inject_key(letter);
        for (word, remaining) in [(0x5000, 1), (0x1e61, 0)] {
            runtime.cpu.ax = 0x1000;
            let before = runtime.cpu;
            runtime.step();
            let mut expected = before;
            expected.ax = word;
            expected.ip += 2;
            assert_eq!(runtime.cpu, expected);
            assert_eq!(runtime.queued_key_count(), remaining);
            assert_eq!(*runtime.state(), crate::GuestState::Running);
        }
        runtime.cpu.ax = 0x1000;
        let before = runtime.cpu;
        runtime.step();
        assert_eq!(*runtime.state(), crate::GuestState::WaitingForInput);
        let mut expected = before;
        expected.ip += 2;
        assert_eq!(runtime.cpu, expected);
        assert!(runtime.inject_key(arrow));
        expected.ax = 0x5000;
        assert_eq!(runtime.cpu, expected);
        assert_eq!(*runtime.state(), crate::GuestState::Running);
        assert_eq!(runtime.queued_key_count(), 0);
    }

    #[test]
    fn enhanced_keyboard_status_reports_and_preserves_the_next_event() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x1100;
        runtime.cpu.bx = 0x1234;
        runtime.cpu.cx = 0x5678;
        runtime.cpu.dx = 0x9abc;
        runtime.cpu.flags = 0xfed7;
        let empty = runtime.cpu;
        dispatch(&mut runtime, 0x16).unwrap();
        assert_eq!(runtime.cpu.ax, empty.ax);
        assert_eq!(runtime.cpu.bx, empty.bx);
        assert_eq!(runtime.cpu.cx, empty.cx);
        assert_eq!(runtime.cpu.dx, empty.dx);
        assert_eq!(
            runtime.cpu.flags & !CpuState::FLAG_ZF,
            empty.flags & !CpuState::FLAG_ZF
        );
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_ZF, 0);
        assert_eq!(runtime.queued_key_count(), 0);

        runtime.inject_key(BiosKey {
            ascii: 0,
            scan_code: 0x48,
        });
        runtime.cpu.ax = 0x1100;
        runtime.cpu.flags = 0xfa97;
        let available = runtime.cpu;
        for _ in 0..2 {
            dispatch(&mut runtime, 0x16).unwrap();
            assert_eq!(runtime.cpu.ax, 0x4800);
            assert_eq!(runtime.cpu.bx, available.bx);
            assert_eq!(runtime.cpu.cx, available.cx);
            assert_eq!(runtime.cpu.dx, available.dx);
            assert_eq!(
                runtime.cpu.flags & !CpuState::FLAG_ZF,
                available.flags & !CpuState::FLAG_ZF
            );
            assert_eq!(runtime.cpu.flags & CpuState::FLAG_ZF, 0);
            assert_eq!(runtime.queued_key_count(), 1);
            runtime.cpu.ax = 0x1100;
        }
    }

    #[test]
    fn country_info_layout_boundaries_and_preserved_registers() {
        for offset in [0x300, 0xfff0] {
            let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
            runtime.cpu.ax = 0x3800;
            runtime.cpu.ds = 0xffff;
            runtime.cpu.dx = offset;
            runtime.cpu.cx = 0x5678;
            runtime.cpu.flags |= CpuState::FLAG_CF;
            let start = crate::GuestMemory::physical_address(runtime.cpu.ds, offset);
            runtime
                .memory
                .write_u8(0xffff, offset.wrapping_sub(1), 0xa5);
            let mut expected = [0u8; 34];
            expected[2] = b'$';
            expected[7] = b',';
            expected[9] = b'.';
            expected[11] = b'/';
            expected[13] = b':';
            expected[16] = 2;
            expected[19] = 2;
            expected[21] = 8;
            expected[22] = b',';
            dispatch(&mut runtime, 0x21).unwrap();
            let mut actual = [0; 34];
            runtime
                .memory
                .read_slice(0xffff, offset, &mut actual)
                .unwrap();
            assert_eq!(actual, expected, "physical start {start:x}");
            assert_eq!(runtime.cpu.ax, 0x3800);
            assert_eq!(
                (
                    runtime.cpu.bx,
                    runtime.cpu.cx,
                    runtime.cpu.ds,
                    runtime.cpu.dx
                ),
                (1, 0x5678, 0xffff, offset)
            );
            assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
            assert_eq!(runtime.memory.read_u8(0xffff, offset.wrapping_sub(1)), 0xa5);
        }
    }

    #[test]
    fn country_selection_errors_and_set_do_not_write_buffer() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.dx = 0x300;
        runtime.memory.write_u8(runtime.cpu.ds, 0x300, 0xa5);
        for (ax, bx, success) in [
            (0x3801, 0, true),
            (0x38ff, 1, true),
            (0x382c, 0, false),
            (0x38ff, 256, false),
        ] {
            runtime.cpu.ax = ax;
            runtime.cpu.bx = bx;
            dispatch(&mut runtime, 0x21).unwrap();
            assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF == 0, success);
            if !success {
                assert_eq!(runtime.cpu.ax, 2);
            }
        }
        runtime.cpu.ax = 0x3801;
        runtime.cpu.dx = 0xffff;
        runtime.memory.write_u8(runtime.cpu.ds, 0xffff, 0xa5);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.memory.read_u8(runtime.cpu.ds, 0xffff), 0xa5);
    }

    #[test]
    fn country_case_mapping_is_far_callable_and_preserves_state() {
        for value in 0u8..=255 {
            let mut runtime = Runtime::from_com(&[0x9a, 0, 2, 0, 8, 0xf4]).unwrap();
            runtime.cpu.ax = 0x3800;
            runtime.cpu.dx = 0x300;
            dispatch(&mut runtime, 0x21).unwrap();
            runtime.cpu.ax = 0x1200 | u16::from(value);
            runtime.cpu.bx = 0xabcd;
            runtime.cpu.flags = 0x843;
            let sp = runtime.cpu.sp;
            runtime.run_slice(100);
            let expected = super::CP437_UPPERCASE
                .iter()
                .find(|pair| pair.0 == value)
                .map_or(value, |pair| pair.1);
            assert_eq!(runtime.cpu.ax, 0x1200 | u16::from(expected));
            assert_eq!(
                (runtime.cpu.bx, runtime.cpu.sp, runtime.cpu.flags),
                (0xabcd, sp, 0x843)
            );
        }
    }

    #[test]
    fn equipment_query_matches_bda_and_advertises_no_coprocessor() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        dispatch(&mut runtime, 0x11).unwrap();
        assert_eq!(runtime.cpu.ax, 0x0020);
        assert_eq!(runtime.cpu.ax, runtime.memory.read_u16(0x0040, 0x0010));
        assert_eq!(runtime.cpu.ax & 2, 0);
        let image = include_bytes!("../../SunlightMines.sunapp/Program/SUNMINE.EXE");
        let mut mz_runtime = Runtime::from_program(image, &[]).unwrap();
        dispatch(&mut mz_runtime, 0x11).unwrap();
        assert_eq!(mz_runtime.cpu.ax, 0x0020);
    }

    const VIDEO_STATE_BUFFER: usize = 0x80;

    fn call_video_state(runtime: &mut Runtime, ax: u16, cx: u16, es: u16, bx: u16) -> CpuState {
        runtime.cpu.ax = ax;
        runtime.cpu.cx = cx;
        runtime.cpu.es = es;
        runtime.cpu.bx = bx;
        dispatch(runtime, 0x10).unwrap();
        runtime.cpu
    }

    fn read_bda(runtime: &Runtime) -> [u8; 0x60] {
        let mut bda = [0u8; 0x60];
        runtime.memory.read_slice(0x0040, 0x0049, &mut bda).unwrap();
        bda
    }

    #[test]
    fn video_state_size_query_reports_documented_blocks_and_preserves_state() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.dx = 0x1234;
        runtime.cpu.si = 0x5678;
        runtime.cpu.di = 0x9abc;
        runtime.cpu.bp = 0xdef0;
        runtime.cpu.flags = 0x0a53;
        runtime
            .memory
            .write_slice(0x2000, 0x0100, &[0xa5; VIDEO_STATE_BUFFER])
            .unwrap();
        let before_bda = read_bda(&runtime);
        let before = runtime.cpu;
        let after = call_video_state(&mut runtime, 0x1c00, 0x0002, 0x2000, 0x0100);
        assert_eq!(after.al(), 0x1c);
        assert_eq!(after.ah(), 0x1c);
        assert_eq!(after.bx, 2);
        assert_eq!(after.cx, 0x0002);
        assert_eq!(after.dx, before.dx);
        assert_eq!(after.si, before.si);
        assert_eq!(after.di, before.di);
        assert_eq!(after.bp, before.bp);
        assert_eq!(after.es, 0x2000);
        assert_eq!(after.flags, before.flags);
        assert_eq!(read_bda(&runtime), before_bda);
        let mut untouched = [0u8; VIDEO_STATE_BUFFER];
        runtime
            .memory
            .read_slice(0x2000, 0x0100, &mut untouched)
            .unwrap();
        assert_eq!(untouched, [0xa5; VIDEO_STATE_BUFFER]);
    }

    #[test]
    fn video_state_query_rejects_unmodeled_areas_and_reserved_bits() {
        // DN's CX=0007h asks for hardware, BIOS data and DAC state. Only the
        // BIOS data area is modeled, so the whole request is rejected instead
        // of returning a convenient size.
        for mask in [
            0x0000, 0x0001, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008, 0x8002,
        ] {
            let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
            let after = call_video_state(&mut runtime, 0x1c00, mask, 0x2000, 0x0100);
            assert_eq!(after.al(), 0, "mask {mask:04x}");
            assert_eq!(after.ah(), 0x1c, "mask {mask:04x}");
            assert_eq!(after.bx, 0x0100, "mask {mask:04x}");
        }
    }

    #[test]
    fn video_state_invalid_subfunction_and_save_restore_are_not_advertised() {
        for al in [0x03u8, 0x04, 0xff] {
            let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
            let after = call_video_state(
                &mut runtime,
                u16::from_le_bytes([al, 0x1c]),
                0x0002,
                0x2000,
                0x0100,
            );
            assert_eq!(after.al(), 0, "AL={al:02x}");
            assert_eq!(after.ah(), 0x1c, "AL={al:02x}");
        }
        for al in [0x01u8, 0x02] {
            let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
            let after = call_video_state(
                &mut runtime,
                u16::from_le_bytes([al, 0x1c]),
                0x0007,
                0x2000,
                0x0100,
            );
            assert_eq!(after.al(), 0, "AL={al:02x} with DN's mask");
        }
    }

    #[test]
    fn video_state_save_writes_documented_header_and_bda() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x0200;
        runtime.cpu.dx = 0x0305;
        dispatch(&mut runtime, 0x10).unwrap();
        runtime.cpu.ax = 0x0100;
        runtime.cpu.cx = 0x0b0c;
        dispatch(&mut runtime, 0x10).unwrap();
        let bda = read_bda(&runtime);
        runtime
            .memory
            .write_slice(0x2000, 0x0100, &[0xa5; VIDEO_STATE_BUFFER + 0x20])
            .unwrap();
        let before = runtime.cpu;
        let after = call_video_state(&mut runtime, 0x1c01, 0x0002, 0x2000, 0x0100);
        assert_eq!(after.al(), 0x1c);
        assert_eq!(after.ah(), 0x1c);
        assert_eq!(after.bx, 0x0100);
        assert_eq!(after.cx, 0x0002);
        assert_eq!(after.flags, before.flags);
        let mut buffer = [0u8; VIDEO_STATE_BUFFER];
        runtime
            .memory
            .read_slice(0x2000, 0x0100, &mut buffer)
            .unwrap();
        assert!(buffer[..2].iter().all(|byte| *byte == 0));
        assert_eq!(u16::from_le_bytes([buffer[2], buffer[3]]), 0x20);
        assert!(buffer[4..0x20].iter().all(|byte| *byte == 0));
        assert_eq!(&buffer[0x20..], &bda[..]);
        // Exactly the documented 0x80 bytes are written.
        for offset in 0x0180..0x01a0 {
            assert_eq!(runtime.memory.read_u8(0x2000, offset), 0xa5);
        }
    }

    #[test]
    fn video_state_save_restore_round_trips_text_mode_state() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x0200;
        runtime.cpu.dx = 0x0305;
        dispatch(&mut runtime, 0x10).unwrap();
        runtime.cpu.ax = 0x0100;
        runtime.cpu.cx = 0x0b0c;
        dispatch(&mut runtime, 0x10).unwrap();
        runtime.cpu.ax = 0x1003;
        runtime.cpu.bx = 0x0000;
        dispatch(&mut runtime, 0x10).unwrap();
        let saved_bda = read_bda(&runtime);
        assert_eq!(
            call_video_state(&mut runtime, 0x1c01, 0x0002, 0x2000, 0x0100).al(),
            0x1c
        );

        // Change every modeled field the BDA area covers.
        runtime.cpu.bx = 0;
        runtime.cpu.ax = 0x0200;
        runtime.cpu.dx = 0x0a0a;
        dispatch(&mut runtime, 0x10).unwrap();
        runtime.cpu.ax = 0x0100;
        runtime.cpu.cx = 0x0000;
        dispatch(&mut runtime, 0x10).unwrap();
        runtime.cpu.ax = 0x0013;
        dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.video_mode(), crate::GuestVideoMode::Vga320x200x256);

        assert_eq!(
            call_video_state(&mut runtime, 0x1c02, 0x0002, 0x2000, 0x0100).al(),
            0x1c
        );
        assert_eq!(runtime.video_mode(), crate::GuestVideoMode::Text80x25Color);
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (5, 3));
        assert_eq!(runtime.cursor_shape(), 0x0b0c);
        assert_eq!(read_bda(&runtime), saved_bda);
        // Attribute mode is hardware state, outside the BDA area, so the
        // mode set's Blink default remains.
        assert_eq!(
            runtime.text_attribute_mode(),
            crate::TextAttributeMode::Blink
        );
    }

    #[test]
    fn video_state_restore_of_graphics_bda_switches_the_model() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x0013;
        dispatch(&mut runtime, 0x10).unwrap();
        let graphics_bda = read_bda(&runtime);
        assert_eq!(
            call_video_state(&mut runtime, 0x1c01, 0x0002, 0x2000, 0x0100).al(),
            0x1c
        );
        runtime.cpu.ax = 0x0003;
        dispatch(&mut runtime, 0x10).unwrap();
        assert_eq!(runtime.video_mode(), crate::GuestVideoMode::Text80x25Color);
        runtime.memory.write_u8(0xa000, 0, 0x5a);
        runtime.set_text_attribute_mode(crate::TextAttributeMode::BackgroundIntensity);
        assert_eq!(
            call_video_state(&mut runtime, 0x1c02, 0x0002, 0x2000, 0x0100).al(),
            0x1c
        );
        assert_eq!(runtime.video_mode(), crate::GuestVideoMode::Vga320x200x256);
        assert_eq!(runtime.cursor_shape(), 0x2000);
        assert_eq!(read_bda(&runtime), graphics_bda);
        assert_eq!(runtime.memory.read_u8(0xa000, 0), 0x5a);
        assert_eq!(
            runtime.text_attribute_mode(),
            crate::TextAttributeMode::BackgroundIntensity
        );
    }

    #[test]
    fn video_state_buffers_must_fit_guest_ram_and_fail_without_partial_writes() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        let bda = read_bda(&runtime);
        // Segment:offset ranges that cross the top of the megabyte wrap in
        // plain address calculation; save/restore must reject them.
        for (es, bx) in [(0xffff, 0xffffu16), (0xffff, 0xff80), (0x2000, 0xff81)] {
            for al in [0x01u8, 0x02] {
                let after =
                    call_video_state(&mut runtime, u16::from_le_bytes([al, 0x1c]), 0x0002, es, bx);
                assert_eq!(after.al(), 0, "AL={al:02x} ES:BX={es:04x}:{bx:04x}");
            }
        }
        // The font ROM is not a valid save destination.
        assert_eq!(
            call_video_state(&mut runtime, 0x1c01, 0x0002, 0xc000, 0x0000).al(),
            0
        );
        assert_eq!(
            runtime.memory.read_u8(0xc000, 0x0000),
            crate::font::FONT_8[0]
        );
        assert_eq!(read_bda(&runtime), bda);
        // A corrupt header or BDA is rejected before any model change.
        runtime
            .memory
            .write_slice(0x2000, 0x0100, &[0u8; 0x80])
            .unwrap();
        assert_eq!(
            call_video_state(&mut runtime, 0x1c02, 0x0002, 0x2000, 0x0100).al(),
            0
        );
        assert_eq!(read_bda(&runtime), bda);
        assert_eq!(runtime.video_mode(), crate::GuestVideoMode::Text80x25Color);
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (0, 0));
    }

    #[test]
    fn video_state_repeated_save_restore_is_deterministic() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x0200;
        runtime.cpu.dx = 0x0102;
        dispatch(&mut runtime, 0x10).unwrap();
        let bda = read_bda(&runtime);
        for _ in 0..100 {
            assert_eq!(
                call_video_state(&mut runtime, 0x1c01, 0x0002, 0x2000, 0x0100).al(),
                0x1c
            );
            let mut first = [0u8; VIDEO_STATE_BUFFER];
            runtime
                .memory
                .read_slice(0x2000, 0x0100, &mut first)
                .unwrap();
            assert_eq!(&first[0x20..], &bda[..]);
            assert_eq!(
                call_video_state(&mut runtime, 0x1c02, 0x0002, 0x2000, 0x0100).al(),
                0x1c
            );
            assert_eq!(read_bda(&runtime), bda);
        }
    }

    #[test]
    fn ctrl_break_flag_is_guest_local_and_unknown_functions_are_rejected() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        let mut other = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x3301;
        runtime.cpu.set_dl(1);
        dispatch(&mut runtime, 0x21).unwrap();
        runtime.cpu.ax = 0x3300;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.dl(), 1);
        other.cpu.ax = 0x3300;
        dispatch(&mut other, 0x21).unwrap();
        assert_eq!(other.cpu.dl(), 0);
        runtime.cpu.ax = 0x33ff;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.al(), 0xff);
    }

    #[test]
    fn list_of_lists_exposes_a_bounded_terminating_sft_capacity_chain() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x5200;
        runtime.cpu.flags |= CpuState::FLAG_CF;
        dispatch(&mut runtime, 0x21).unwrap();
        let segment = runtime.cpu.es;
        let offset = runtime.cpu.bx;
        assert!(segment < crate::DOS_ARENA_START_SEGMENT);
        let sft_offset = runtime.memory.read_u16(segment, offset + 4);
        let sft_segment = runtime.memory.read_u16(segment, offset + 6);
        assert_eq!(runtime.memory.read_u16(sft_segment, sft_offset), 0xffff);
        assert_eq!(runtime.memory.read_u16(sft_segment, sft_offset + 2), 0xffff);
        assert_eq!(runtime.memory.read_u16(sft_segment, sft_offset + 4), 64);
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, CpuState::FLAG_CF);
        // Guest writes cannot change the authoritative capacity on next call.
        runtime.memory.write_u16(sft_segment, sft_offset + 4, 0);
        runtime.cpu.ax = 0x5200;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.memory.read_u16(sft_segment, sft_offset + 4), 64);
    }

    #[test]
    fn absent_multiplex_executes_guest_iret_for_arbitrary_functions() {
        for ax in [0x9900, 0x9905, 0xd44d, 0x1680] {
            let mut runtime = Runtime::from_com(&[0xcd, 0x2f]).unwrap();
            runtime.cpu.ax = ax;
            runtime.cpu.bx = 0x1234;
            let before = runtime.cpu;
            runtime.run_slice(2);
            let mut expected = before;
            expected.ip += 2;
            assert_eq!(runtime.cpu, expected);
        }
    }

    fn path(runtime: &mut Runtime, value: &[u8]) {
        runtime
            .memory
            .write_slice(PSP_SEGMENT, 0x0200, value)
            .unwrap();
        runtime
            .memory
            .write_u8(PSP_SEGMENT, 0x0200 + value.len() as u16, 0);
        runtime.cpu.ds = PSP_SEGMENT;
        runtime.cpu.dx = 0x0200;
    }

    #[test]
    fn file_services_use_handles_carry_and_guest_buffers() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        path(&mut runtime, b"C:\\DATA.TXT");
        runtime.cpu.cx = 0;
        runtime.cpu.set_ah(0x3c);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        let handle = runtime.cpu.ax;
        assert!(handle >= 5);

        runtime
            .memory
            .write_slice(PSP_SEGMENT, 0x0300, b"hello")
            .unwrap();
        runtime.cpu.bx = handle;
        runtime.cpu.cx = 5;
        runtime.cpu.dx = 0x0300;
        runtime.cpu.ds = PSP_SEGMENT;
        runtime.cpu.set_ah(0x40);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.ax, 5);

        runtime.cpu.bx = handle;
        runtime.cpu.cx = 0;
        runtime.cpu.dx = 0;
        runtime.cpu.set_al(0);
        runtime.cpu.set_ah(0x42);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!((runtime.cpu.dx, runtime.cpu.ax), (0, 0));

        runtime.cpu.bx = handle;
        runtime.cpu.cx = 5;
        runtime.cpu.dx = 0x0320;
        runtime.cpu.ds = PSP_SEGMENT;
        runtime.cpu.set_ah(0x3f);
        dispatch(&mut runtime, 0x21).unwrap();
        let mut received = [0; 5];
        runtime
            .memory
            .read_slice(PSP_SEGMENT, 0x0320, &mut received)
            .unwrap();
        assert_eq!(received, *b"hello");

        runtime.cpu.bx = handle;
        runtime.cpu.set_ah(0x3e);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
    }

    #[test]
    fn console_handle_reads_block_until_a_guest_key_arrives() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.bx = 0;
        runtime.cpu.cx = 1;
        runtime.cpu.dx = 0x0340;
        runtime.cpu.ds = PSP_SEGMENT;
        runtime.cpu.set_ah(0x3f);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.state(), &crate::GuestState::WaitingForInput);

        runtime.inject_ascii(b'Q');
        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0x0340), b'Q');
        assert_eq!(runtime.cpu.ax, 1);
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(runtime.state(), &crate::GuestState::Running);
    }

    #[test]
    fn console_handle_enter_supplies_cr_lf_without_a_second_keypress() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.bx = 0;
        runtime.cpu.cx = 1;
        runtime.cpu.dx = 0x0340;
        runtime.cpu.ds = PSP_SEGMENT;
        runtime.cpu.set_ah(0x3f);
        dispatch(&mut runtime, 0x21).unwrap();

        runtime.inject_ascii(b'\r');
        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0x0340), b'\r');
        assert_eq!(runtime.cpu.ax, 1);

        runtime.cpu.bx = 0;
        runtime.cpu.cx = 1;
        runtime.cpu.dx = 0x0341;
        runtime.cpu.ds = PSP_SEGMENT;
        runtime.cpu.set_ah(0x3f);
        dispatch(&mut runtime, 0x21).unwrap();

        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0x0341), b'\n');
        assert_eq!(runtime.cpu.ax, 1);
        assert_eq!(runtime.state(), &crate::GuestState::Running);
        assert_eq!((runtime.cursor_column(), runtime.cursor_row()), (0, 1));
    }

    #[test]
    fn disk_space_uses_dos_selectors_without_changing_drive_or_flags() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        let c = crate::DriveSpace {
            sectors_per_cluster: 8,
            free_clusters: 12_345,
            bytes_per_sector: 512,
            total_clusters: 32_000,
        };
        let d = crate::DriveSpace {
            sectors_per_cluster: 64,
            free_clusters: 0,
            bytes_per_sector: 512,
            total_clusters: 65_535,
        };
        runtime.drives.set_drive_space(DosDrive::C, Some(c)).unwrap();
        runtime.drives.set_drive_space(DosDrive::D, Some(d)).unwrap();
        runtime.drives.select(DosDrive::D).unwrap();

        for (dl, output) in [
            (0, Some((64, 0, 512, 65_535))),
            (3, Some((8, 12_345, 512, 32_000))),
            (4, Some((64, 0, 512, 65_535))),
            (20, None), // T: is mapped but has no allocation data.
            (1, None),  // A: is not mapped.
            (255, None),
        ] {
            for flags in [0xf246, 0xf293] {
                runtime.cpu.gpr_upper = [0xface; 8];
                runtime.cpu.ax = 0x36ff;
                runtime.cpu.bx = 0xaabb;
                runtime.cpu.cx = 0xccdd;
                runtime.cpu.dx = u16::from_le_bytes([dl, 0xee]);
                runtime.cpu.si = 0x1234;
                runtime.cpu.di = 0x5678;
                runtime.cpu.flags = flags;
                let mut expected = runtime.cpu;
                if let Some(registers) = output {
                    (expected.ax, expected.bx, expected.cx, expected.dx) = registers;
                } else {
                    expected.ax = 0xffff;
                }
                dispatch(&mut runtime, 0x21).unwrap();
                assert_eq!(runtime.cpu, expected, "DL={dl}");
                assert_eq!(runtime.drives.current_drive, DosDrive::D);
            }
        }

        let path = runtime.drives.parse_path(b"D:NEW.DAT").unwrap();
        runtime.drives.create_file(&path, 0).unwrap();
        runtime.cpu.ax = 0x3600;
        runtime.cpu.set_dl(0);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.ax, 0xffff);
    }

    #[test]
    fn disk_space_rejects_unrepresentable_or_inconsistent_metadata() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        let space = crate::DriveSpace {
            sectors_per_cluster: 128,
            free_clusters: 65_535,
            bytes_per_sector: 4096,
            total_clusters: 65_535,
        };
        // The byte capacity exceeds u32, but all DOS outputs fit.
        runtime.drives.set_drive_space(DosDrive::C, Some(space)).unwrap();
        runtime.cpu.ax = 0x3600;
        runtime.cpu.dx = 3;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!((runtime.cpu.ax, runtime.cpu.bx, runtime.cpu.cx, runtime.cpu.dx),
                   (128, 65_535, 4096, 65_535));

        for invalid in [
            crate::DriveSpace { sectors_per_cluster: 0, ..space },
            crate::DriveSpace { sectors_per_cluster: 65_535, ..space },
            crate::DriveSpace { sectors_per_cluster: u64::MAX, ..space },
            crate::DriveSpace { bytes_per_sector: 0, ..space },
            crate::DriveSpace { bytes_per_sector: 65_536, ..space },
            crate::DriveSpace { free_clusters: 65_536, ..space },
            crate::DriveSpace { total_clusters: 65_536, ..space },
            crate::DriveSpace { total_clusters: 65_534, ..space },
        ] {
            runtime.drives.set_drive_space(DosDrive::C, Some(invalid)).unwrap();
            runtime.cpu.ax = 0x3600;
            runtime.cpu.dx = 3;
            let mut expected = runtime.cpu;
            expected.ax = 0xffff;
            dispatch(&mut runtime, 0x21).unwrap();
            assert_eq!(runtime.cpu, expected, "{invalid:?}");
        }
    }

    #[test]
    fn drive_selection_and_find_first_write_a_dta_result() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime
            .drives_mut()
            .add_base_file(DosDrive::D, "CHRONOS.TXT", b"file".to_vec())
            .unwrap();
        runtime.cpu.set_dl(3);
        runtime.cpu.set_ah(0x0e);
        dispatch(&mut runtime, 0x21).unwrap();
        runtime.cpu.set_ah(0x19);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.al(), 3);

        path(&mut runtime, b"D:\\*.TXT");
        runtime.cpu.cx = 0;
        runtime.cpu.set_ah(0x4e);
        dispatch(&mut runtime, 0x21).unwrap();
        let mut name = [0; 12];
        runtime
            .memory
            .read_slice(PSP_SEGMENT, 0x009e, &mut name)
            .unwrap();
        assert_eq!(&name[..11], b"CHRONOS.TXT");

        runtime.cpu.set_ah(0x4f);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(runtime.cpu.ax, 18);
    }

    fn search_call(runtime: &mut Runtime, ah: u8, error: Option<DosError>) {
        runtime.cpu.set_ah(ah);
        dispatch(runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF != 0, error.is_some());
        if let Some(error) = error {
            assert_eq!(runtime.cpu.ax, error.code());
        }
    }

    fn search_name(runtime: &Runtime) -> [u8; 13] {
        let mut result = [0; 13];
        runtime
            .memory
            .read_slice(runtime.dta_segment, runtime.dta_offset + 30, &mut result)
            .unwrap();
        result
    }

    #[test]
    fn directory_search_filters_effective_entries_and_exhausts() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        for name in ["A1.TXT", "A2.TXT", "A3.TXT", "A4.TXT", "A5.TXT", "A10.TXT", "LONGFILENAME.TXT"] {
            runtime.drives.add_base_file(DosDrive::C, name, vec![1, 2, 3]).unwrap();
        }
        for (name, attributes) in [("A4.TXT", crate::fs::ATTR_HIDDEN), ("A5.TXT", crate::fs::ATTR_SYSTEM)] {
            let path = runtime.drives.parse_path(name.as_bytes()).unwrap();
            runtime.drives.set_attributes(&path, attributes).unwrap();
        }
        let subdir = runtime.drives.parse_path(b"A6.TXT").unwrap();
        runtime.drives.mkdir(&subdir).unwrap();
        path(&mut runtime, b"C:\\A?.TXT");
        runtime.cpu.cx = 0;
        search_call(&mut runtime, 0x4e, None);
        assert_eq!(&search_name(&runtime)[..7], b"A1.TXT\0");
        assert_eq!(runtime.memory.read_u16(PSP_SEGMENT, 0x0098), 0);
        assert_eq!(runtime.memory.read_u16(PSP_SEGMENT, 0x009a), 3);
        for expected in [b"A2.TXT\0", b"A3.TXT\0"] {
            search_call(&mut runtime, 0x4f, None);
            assert_eq!(&search_name(&runtime)[..7], expected);
        }
        for _ in 0..2 {
            search_call(&mut runtime, 0x4f, Some(DosError::NoMoreFiles));
        }
        runtime.cpu.cx = 0x16;
        search_call(&mut runtime, 0x4e, None);
        for expected in [b"A2.TXT\0", b"A3.TXT\0", b"A4.TXT\0", b"A5.TXT\0", b"A6.TXT\0"] {
            search_call(&mut runtime, 0x4f, None);
            assert_eq!(&search_name(&runtime)[..7], expected);
        }
        search_call(&mut runtime, 0x4f, Some(DosError::NoMoreFiles));
        path(&mut runtime, b"C:\\LONGFILENAME.TXT");
        search_call(&mut runtime, 0x4e, Some(DosError::NoMoreFiles));
    }

    #[test]
    fn directory_search_rejects_bad_paths_and_dta_without_partial_write() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        for name in ["A.TXT", "B.TXT"] {
            runtime.drives.add_base_file(DosDrive::C, name, vec![]).unwrap();
        }
        path(&mut runtime, b"C:\\*.TXT");
        search_call(&mut runtime, 0x4e, None);
        runtime.dta_offset = 0xffff;
        runtime.memory.write_u8(PSP_SEGMENT, 0xffff, 0xab);
        search_call(&mut runtime, 0x4f, Some(DosError::InvalidMemoryBlock));
        assert_eq!(runtime.memory.read_u8(PSP_SEGMENT, 0xffff), 0xab);
        runtime.dta_offset = 0xffd5;
        search_call(&mut runtime, 0x4f, None);
        assert_eq!(&search_name(&runtime)[..6], b"B.TXT\0");

        for (request, error) in [
            (&b"C:\\*.BIN"[..], DosError::NoMoreFiles),
            (&b"C:\\MISSING\\*.*"[..], DosError::PathNotFound),
            (&b"Z:\\*.*"[..], DosError::InvalidDrive),
        ] {
            path(&mut runtime, b"C:\\*.TXT");
            search_call(&mut runtime, 0x4e, None);
            path(&mut runtime, request);
            search_call(&mut runtime, 0x4e, Some(error));
            search_call(&mut runtime, 0x4f, Some(DosError::NoMoreFiles));
        }
        runtime.dta_offset = 0xffd6;
        search_call(&mut runtime, 0x4e, Some(DosError::InvalidMemoryBlock));
    }

    #[test]
    fn directory_search_isolated_across_exec_and_discarded_on_exit() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        for name in ["A.TXT", "B.TXT", "CHILD.COM"] {
            runtime.drives.add_base_file(DosDrive::C, name, vec![0xf4]).unwrap();
        }
        path(&mut runtime, b"C:\\*.TXT");
        search_call(&mut runtime, 0x4e, None);
        let child = runtime.drives.parse_path(b"C:\\CHILD.COM").unwrap();
        runtime.exec(child.clone(), b"", 0).unwrap();
        search_call(&mut runtime, 0x4f, Some(DosError::NoMoreFiles));
        path(&mut runtime, b"C:\\*.TXT");
        search_call(&mut runtime, 0x4e, None);
        assert_eq!(&search_name(&runtime)[..6], b"A.TXT\0");
        runtime.exit(0).unwrap();
        search_call(&mut runtime, 0x4f, None);
        assert_eq!(&search_name(&runtime)[..6], b"B.TXT\0");
        runtime.exec(child, b"", 0).unwrap();
        search_call(&mut runtime, 0x4f, Some(DosError::NoMoreFiles));
        runtime.exit(0).unwrap();
        runtime.exit(0).unwrap();
        assert!(runtime.active_process.searches.is_empty());
    }

    #[test]
    fn parse_filename_builds_uppercase_fcb_and_reports_wildcards() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime
            .memory
            .write_slice(PSP_SEGMENT, 0x0200, b" c:vga*.com\0")
            .unwrap();
        runtime.cpu.ds = PSP_SEGMENT;
        runtime.cpu.si = 0x0200;
        runtime.cpu.es = PSP_SEGMENT;
        runtime.cpu.di = 0x0300;
        runtime.cpu.ax = 0x2901;
        dispatch(&mut runtime, 0x21).unwrap();

        let mut fcb = [0; 12];
        runtime
            .memory
            .read_slice(PSP_SEGMENT, 0x0300, &mut fcb)
            .unwrap();
        assert_eq!(runtime.cpu.al(), 1);
        assert_eq!(fcb[0], 3);
        assert_eq!(&fcb[1..9], b"VGA?????");
        assert_eq!(&fcb[9..12], b"COM");
        assert_eq!(runtime.memory.read_u8(runtime.cpu.ds, runtime.cpu.si), 0);
    }

    #[test]
    fn duplicated_stdout_can_be_redirected_and_restored() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        path(&mut runtime, b"C:\\OUT.TXT");
        runtime.cpu.ax = 0x3c00;
        runtime.cpu.cx = 0;
        dispatch(&mut runtime, 0x21).unwrap();
        let output = runtime.cpu.ax;

        runtime.cpu.ax = 0x4500;
        runtime.cpu.bx = 1;
        dispatch(&mut runtime, 0x21).unwrap();
        let saved_stdout = runtime.cpu.ax;

        runtime.cpu.ax = 0x4600;
        runtime.cpu.bx = output;
        runtime.cpu.cx = 1;
        dispatch(&mut runtime, 0x21).unwrap();

        runtime
            .memory
            .write_slice(PSP_SEGMENT, 0x0300, b"guest")
            .unwrap();
        runtime.cpu.ax = 0x4000;
        runtime.cpu.bx = 1;
        runtime.cpu.cx = 5;
        runtime.cpu.dx = 0x0300;
        runtime.cpu.ds = PSP_SEGMENT;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.ax, 5);

        runtime.cpu.ax = 0x4600;
        runtime.cpu.bx = saved_stdout;
        runtime.cpu.cx = 1;
        dispatch(&mut runtime, 0x21).unwrap();
        runtime.cpu.set_ah(0x3e);
        runtime.cpu.bx = saved_stdout;
        dispatch(&mut runtime, 0x21).unwrap();
        runtime.cpu.set_ah(0x3e);
        runtime.cpu.bx = output;
        dispatch(&mut runtime, 0x21).unwrap();

        let file = runtime.drives().parse_path(b"C:\\OUT.TXT").unwrap();
        assert_eq!(runtime.drives().read_file(&file).unwrap(), b"guest");

        runtime
            .memory
            .write_slice(PSP_SEGMENT, 0x0300, b"!")
            .unwrap();
        runtime.cpu.ax = 0x4000;
        runtime.cpu.bx = 1;
        runtime.cpu.cx = 1;
        runtime.cpu.dx = 0x0300;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cell(0, 0).character, b'!');
    }

    #[test]
    fn ioctl_reports_console_handles_as_character_devices() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x4400;
        runtime.cpu.bx = 1;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(runtime.cpu.dx, 0x0080);

        path(&mut runtime, b"C:\\DATA.TXT");
        runtime.cpu.ax = 0x3c00;
        runtime.cpu.cx = 0;
        dispatch(&mut runtime, 0x21).unwrap();
        let file = runtime.cpu.ax;
        runtime.cpu.ax = 0x4400;
        runtime.cpu.bx = file;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(runtime.cpu.dx, 0);
    }

    #[test]
    fn unsupported_lfn_multiplex_returns_a_dos_error_without_trapping() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x7100;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_ne!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!(runtime.cpu.ax, DosError::InvalidFunction.code());
    }

    #[test]
    fn extended_error_query_is_safe_and_reports_no_pending_error() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.cpu.ax = 0x5900;
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(runtime.cpu.flags & CpuState::FLAG_CF, 0);
        assert_eq!((runtime.cpu.ax, runtime.cpu.bx, runtime.cpu.cx), (0, 0, 0));
    }

    #[test]
    fn date_and_time_are_guest_visible_but_not_settable() {
        let mut runtime = Runtime::from_com(&[0xf4]).unwrap();
        runtime.set_guest_unix_time(86_400 + 3_723);

        runtime.cpu.set_ah(0x2a);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(
            (runtime.cpu.cx, runtime.cpu.dh(), runtime.cpu.dl()),
            (1970, 1, 2)
        );
        assert_eq!(runtime.cpu.al(), 5);

        runtime.cpu.set_ah(0x2c);
        dispatch(&mut runtime, 0x21).unwrap();
        assert_eq!(
            (
                runtime.cpu.ch(),
                runtime.cpu.cl(),
                runtime.cpu.dh(),
                runtime.cpu.dl()
            ),
            (1, 2, 3, 0)
        );
    }
}
