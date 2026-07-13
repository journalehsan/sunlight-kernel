use alloc::{vec, vec::Vec};

use crate::{
    CpuState, DirectoryEntry, DosDrive, DosError, DosHandle, DosPath, GuestVideoMode, OpenMode,
    Runtime, Trap, DEFAULT_ATTRIBUTE, TEXT_COLUMNS,
};

pub const DOS_VERSION_MAJOR: u8 = 5;
pub const DOS_VERSION_MINOR: u8 = 0;
const MAX_DOS_STRING_BYTES: usize = 0x1_0000;
const DOS_PATH_BYTES: usize = 241;
const DTA_RESULT_SIZE: usize = 43;

pub fn dispatch(runtime: &mut Runtime, interrupt: u8) -> Result<(), Trap> {
    match interrupt {
        0x20 => runtime.exit(0),
        0x10 => dispatch_video(runtime),
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
            runtime.cpu.set_al(runtime.video_mode().bios_mode());
            runtime.cpu.set_ah(match runtime.video_mode() {
                GuestVideoMode::Text80x25Color => TEXT_COLUMNS as u8,
                GuestVideoMode::Vga320x200x256 => 40,
            });
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
            dos_success(runtime)
        }
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
    runtime.searches = entries;
    runtime.search_index = 0;
    write_next_search(runtime)
}

fn find_next(runtime: &mut Runtime) -> Result<(), Trap> {
    write_next_search(runtime)
}

fn write_next_search(runtime: &mut Runtime) -> Result<(), Trap> {
    let Some(entry) = runtime.searches.get(runtime.search_index).cloned() else {
        return dos_error(runtime, DosError::NoMoreFiles);
    };
    runtime.search_index += 1;
    write_dta_entry(runtime, &entry);
    dos_success(runtime)
}

fn write_dta_entry(runtime: &mut Runtime, entry: &DirectoryEntry) {
    let zero = [0u8; DTA_RESULT_SIZE];
    let _ = runtime
        .memory
        .write_slice(runtime.dta_segment, runtime.dta_offset, &zero);
    runtime.memory.write_u8(
        runtime.dta_segment,
        runtime.dta_offset.wrapping_add(21),
        entry.attributes,
    );
    runtime
        .memory
        .write_u16(runtime.dta_segment, runtime.dta_offset.wrapping_add(22), 0);
    runtime.memory.write_u16(
        runtime.dta_segment,
        runtime.dta_offset.wrapping_add(24),
        0x0021,
    );
    let size = entry.size.to_le_bytes();
    let _ = runtime.memory.write_slice(
        runtime.dta_segment,
        runtime.dta_offset.wrapping_add(26),
        &size,
    );
    let name = entry.name.as_bytes();
    let name_len = name.len().min(12);
    let _ = runtime.memory.write_slice(
        runtime.dta_segment,
        runtime.dta_offset.wrapping_add(30),
        &name[..name_len],
    );
    runtime.memory.write_u8(
        runtime.dta_segment,
        runtime.dta_offset.wrapping_add(30 + name_len as u16),
        0,
    );
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
    use crate::{CpuState, DosDrive, DosError, Runtime, PSP_SEGMENT};

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
