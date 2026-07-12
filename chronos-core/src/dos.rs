use alloc::vec;

use crate::{
    CpuState, DirectoryEntry, DosDrive, DosError, DosPath, OpenMode, Runtime, Trap,
    DEFAULT_ATTRIBUTE, TEXT_COLUMNS,
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
        0x1a => {
            runtime.dta_segment = runtime.cpu.ds;
            runtime.dta_offset = runtime.cpu.dx;
            dos_success(runtime)
        }
        0x2f => {
            runtime.cpu.es = runtime.dta_segment;
            runtime.cpu.bx = runtime.dta_offset;
            dos_success(runtime)
        }
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
        0x47 => get_current_directory(runtime),
        0x4e => find_first(runtime),
        0x4f => find_next(runtime),
        0x56 => rename_file(runtime),
        0x4c => runtime.exit(runtime.cpu.al()),
        function => Err(Trap::UnsupportedInterrupt {
            interrupt: 0x21,
            function,
        }),
    }
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
    if handle == 0 {
        if requested == 0 {
            runtime.cpu.ax = 0;
            return dos_success(runtime);
        }
        if let Some(key) = runtime.pop_key() {
            runtime
                .memory
                .write_u8(runtime.cpu.ds, runtime.cpu.dx, key.ascii);
            runtime.cpu.ax = 1;
        } else {
            runtime.cpu.ax = 0;
        }
        return dos_success(runtime);
    }
    if handle <= 4 {
        return dos_error(runtime, DosError::InvalidHandle);
    }
    let descriptor = match runtime.handles.get(handle) {
        Ok(value) => value.clone(),
        Err(error) => return dos_error(runtime, error),
    };
    if !descriptor.mode.can_read() {
        return dos_error(runtime, DosError::AccessDenied);
    }
    let data = match runtime.drives.read_file(&descriptor.path) {
        Ok(data) => data,
        Err(error) => return dos_error(runtime, error),
    };
    let start = descriptor.position.min(data.len());
    let count = requested.min(data.len() - start);
    if runtime
        .memory
        .write_slice(runtime.cpu.ds, runtime.cpu.dx, &data[start..start + count])
        .is_err()
    {
        return dos_error(runtime, DosError::InsufficientMemory);
    }
    if let Ok(entry) = runtime.handles.get_mut(handle) {
        entry.position = start + count;
    }
    runtime.cpu.ax = count as u16;
    dos_success(runtime)
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
    if matches!(handle, 1 | 2) {
        for byte in bytes {
            runtime.teletype(byte, DEFAULT_ATTRIBUTE);
        }
        runtime.cpu.ax = requested as u16;
        return dos_success(runtime);
    }
    if handle <= 4 {
        return dos_error(runtime, DosError::InvalidHandle);
    }
    let descriptor = match runtime.handles.get(handle) {
        Ok(value) => value.clone(),
        Err(error) => return dos_error(runtime, error),
    };
    if !descriptor.mode.can_write() {
        return dos_error(runtime, DosError::AccessDenied);
    }
    let written = match runtime.drives.write_file(
        &descriptor.path,
        descriptor.position,
        &bytes,
        requested == 0,
    ) {
        Ok(value) => value,
        Err(error) => return dos_error(runtime, error),
    };
    if let Ok(entry) = runtime.handles.get_mut(handle) {
        entry.position = entry.position.saturating_add(written);
    }
    runtime.cpu.ax = written as u16;
    dos_success(runtime)
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
    let base = match origin {
        0 => 0i64,
        1 => descriptor.position.min(i64::MAX as usize) as i64,
        2 => match runtime.drives.file_len(&descriptor.path) {
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
    if let Ok(entry) = runtime.handles.get_mut(handle) {
        entry.position = new_position as usize;
    }
    let output = new_position as u32;
    runtime.cpu.dx = (output >> 16) as u16;
    runtime.cpu.ax = output as u16;
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
    use crate::{CpuState, DosDrive, Runtime, PSP_SEGMENT};

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
}
