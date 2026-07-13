use crate::{
    classify_executable, parse_mz, relocations, CpuState, DosMemoryArena, ExecutableFormat,
    GuestMemory, MemoryError, MzError, MEMORY_SIZE,
};

/// Legacy deterministic initial PSP segment retained by the direct COM API.
pub const PSP_SEGMENT: u16 = 0x1000;
pub const COM_OFFSET: u16 = 0x0100;
pub const PSP_PARAGRAPHS: u16 = 0x10;
const PROCESS_BYTES: usize = 0x1_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoaderError {
    UnsupportedExecutableFormat,
    Mz(MzError),
    ProgramTooLarge { size: usize, maximum: usize },
    InsufficientMemory { requested: u16, largest: u16 },
    InvalidInitialStack,
    Memory(MemoryError),
}

impl From<MemoryError> for LoaderError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<MzError> for LoaderError {
    fn from(error: MzError) -> Self {
        Self::Mz(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedProgram {
    pub cpu: CpuState,
    pub psp_segment: u16,
    pub load_segment: u16,
    pub paragraphs: u16,
    pub format: ExecutableFormat,
}

/// Installs a PSP and loads a `.COM` image at `PSP:0100`.
pub fn load_com(memory: &mut GuestMemory, image: &[u8]) -> Result<CpuState, LoaderError> {
    load_com_with_command_tail(memory, image, &[])
}

/// Loads a `.COM` image at the legacy PSP segment. This API is kept for
/// existing callers; new runtimes allocate their PSP through `DosMemoryArena`.
pub fn load_com_with_command_tail(
    memory: &mut GuestMemory,
    image: &[u8],
    command_tail: &[u8],
) -> Result<CpuState, LoaderError> {
    if classify_executable(image)? != ExecutableFormat::Com {
        return Err(LoaderError::UnsupportedExecutableFormat);
    }
    let maximum = PROCESS_BYTES - COM_OFFSET as usize;
    if image.len() > maximum
        || image.len()
            > MEMORY_SIZE.saturating_sub(GuestMemory::physical_address(PSP_SEGMENT, COM_OFFSET))
    {
        return Err(LoaderError::ProgramTooLarge {
            size: image.len(),
            maximum,
        });
    }
    build_psp(
        memory,
        PSP_SEGMENT,
        None,
        0,
        PSP_SEGMENT.wrapping_add(0x1000),
        command_tail,
    )?;
    memory.write_slice(PSP_SEGMENT, COM_OFFSET, image)?;
    Ok(com_cpu(PSP_SEGMENT))
}

pub fn load_program(
    memory: &mut GuestMemory,
    arena: &mut DosMemoryArena,
    image: &[u8],
    command_tail: &[u8],
    parent_psp: Option<u16>,
    environment_segment: u16,
) -> Result<LoadedProgram, LoaderError> {
    match classify_executable(image)? {
        ExecutableFormat::Com => load_com_in_arena(
            memory,
            arena,
            image,
            command_tail,
            parent_psp,
            environment_segment,
        ),
        ExecutableFormat::Mz => load_mz_in_arena(
            memory,
            arena,
            image,
            command_tail,
            parent_psp,
            environment_segment,
        ),
    }
}

pub fn build_psp(
    memory: &mut GuestMemory,
    psp_segment: u16,
    parent_psp: Option<u16>,
    environment_segment: u16,
    end_segment: u16,
    command_tail: &[u8],
) -> Result<(), LoaderError> {
    let paragraphs = end_segment
        .checked_sub(psp_segment)
        .ok_or(LoaderError::InvalidInitialStack)?;
    let mcb_segment = psp_segment.wrapping_sub(1);
    memory.write_u8(mcb_segment, 0, b'Z');
    memory.write_u16(mcb_segment, 1, psp_segment);
    memory.write_u16(mcb_segment, 3, paragraphs);
    memory.write_slice(psp_segment, 0, &[0; 0x100])?;
    memory.write_slice(psp_segment, 0, &[0xcd, 0x20])?;
    memory.write_u16(psp_segment, 0x0002, end_segment);
    memory.write_u16(psp_segment, 0x0016, parent_psp.unwrap_or(0));
    memory.write_u16(psp_segment, 0x002c, environment_segment);
    for handle in 0..3 {
        memory.write_u8(psp_segment, 0x0018 + handle, handle as u8);
    }
    let tail_len = command_tail.len().min(126);
    memory.write_u8(psp_segment, 0x0080, tail_len as u8);
    memory.write_slice(psp_segment, 0x0081, &command_tail[..tail_len])?;
    memory.write_u8(psp_segment, 0x0081u16.wrapping_add(tail_len as u16), b'\r');
    Ok(())
}

fn load_com_in_arena(
    memory: &mut GuestMemory,
    arena: &mut DosMemoryArena,
    image: &[u8],
    command_tail: &[u8],
    parent_psp: Option<u16>,
    environment_segment: u16,
) -> Result<LoadedProgram, LoaderError> {
    let bytes = COM_OFFSET as usize + image.len();
    let paragraphs = paragraphs_for(bytes)?;
    let psp_segment = allocate_process(arena, paragraphs)?;
    let end_segment = psp_segment
        .checked_add(paragraphs)
        .ok_or(LoaderError::ProgramTooLarge {
            size: bytes,
            maximum: 0,
        })?;
    build_psp(
        memory,
        psp_segment,
        parent_psp,
        environment_segment,
        end_segment,
        command_tail,
    )?;
    memory.write_slice(psp_segment, COM_OFFSET, image)?;
    Ok(LoadedProgram {
        cpu: com_cpu(psp_segment),
        psp_segment,
        load_segment: psp_segment,
        paragraphs,
        format: ExecutableFormat::Com,
    })
}

fn load_mz_in_arena(
    memory: &mut GuestMemory,
    arena: &mut DosMemoryArena,
    image: &[u8],
    command_tail: &[u8],
    parent_psp: Option<u16>,
    environment_segment: u16,
) -> Result<LoadedProgram, LoaderError> {
    let header = parse_mz(image)?;
    let image_paragraphs = paragraphs_for(header.image_size)?;
    let requested_paragraphs = PSP_PARAGRAPHS
        .checked_add(image_paragraphs)
        .and_then(|value| value.checked_add(header.e_minalloc))
        .ok_or(LoaderError::ProgramTooLarge {
            size: header.image_size,
            maximum: MEMORY_SIZE,
        })?;
    let paragraphs = requested_paragraphs;
    let psp_segment = allocate_process(arena, paragraphs)?;
    let load_segment =
        psp_segment
            .checked_add(PSP_PARAGRAPHS)
            .ok_or(LoaderError::ProgramTooLarge {
                size: header.image_size,
                maximum: MEMORY_SIZE,
            })?;
    let end_segment = psp_segment
        .checked_add(paragraphs)
        .ok_or(LoaderError::ProgramTooLarge {
            size: header.image_size,
            maximum: MEMORY_SIZE,
        })?;
    build_psp(
        memory,
        psp_segment,
        parent_psp,
        environment_segment,
        end_segment,
        command_tail,
    )?;
    memory.write_slice(
        load_segment,
        0,
        &image[header.header_size..header.logical_size],
    )?;
    for (index, relocation) in relocations(image, header)?.into_iter().enumerate() {
        let target_segment = load_segment.checked_add(relocation.segment).ok_or(
            MzError::RelocationOutsideImage {
                index: index as u16,
            },
        )?;
        let value = memory.read_u16(target_segment, relocation.offset);
        memory.write_u16(
            target_segment,
            relocation.offset,
            value.wrapping_add(load_segment),
        );
    }
    let cs = load_segment
        .checked_add(header.e_cs)
        .ok_or(LoaderError::InvalidInitialStack)?;
    let ss = load_segment
        .checked_add(header.e_ss)
        .ok_or(LoaderError::InvalidInitialStack)?;
    if !segment_in_process(cs, psp_segment, paragraphs)
        || !segment_in_process(ss, psp_segment, paragraphs)
    {
        return Err(LoaderError::InvalidInitialStack);
    }
    let cs_linear = header.e_cs as usize * 16 + header.e_ip as usize;
    let ss_linear = header.e_ss as usize * 16 + header.e_sp as usize;
    let allocation_bytes =
        (image_paragraphs as usize + header.e_minalloc as usize).saturating_mul(16);
    // A DOS stack pointer may designate the first byte immediately after the
    // allocated stack area. FPC's large-model startup uses that conventional
    // top-of-stack form, so equality is valid here.
    if cs_linear >= header.image_size || ss_linear > allocation_bytes {
        return Err(LoaderError::InvalidInitialStack);
    }
    Ok(LoadedProgram {
        cpu: CpuState {
            cs,
            ip: header.e_ip,
            ss,
            sp: header.e_sp,
            ds: psp_segment,
            es: psp_segment,
            flags: 0x0002,
            ..CpuState::default()
        },
        psp_segment,
        load_segment,
        paragraphs,
        format: ExecutableFormat::Mz,
    })
}

fn allocate_process(arena: &mut DosMemoryArena, paragraphs: u16) -> Result<u16, LoaderError> {
    arena
        .allocate_process(paragraphs)
        .map_err(|error| match error {
            crate::ArenaError::InsufficientMemory { largest } => LoaderError::InsufficientMemory {
                requested: paragraphs,
                largest,
            },
            _ => LoaderError::InsufficientMemory {
                requested: paragraphs,
                largest: arena.largest_available(),
            },
        })
}

fn paragraphs_for(bytes: usize) -> Result<u16, LoaderError> {
    let paragraphs =
        bytes
            .checked_add(15)
            .map(|value| value / 16)
            .ok_or(LoaderError::ProgramTooLarge {
                size: bytes,
                maximum: MEMORY_SIZE,
            })?;
    u16::try_from(paragraphs).map_err(|_| LoaderError::ProgramTooLarge {
        size: bytes,
        maximum: MEMORY_SIZE,
    })
}

fn segment_in_process(segment: u16, psp_segment: u16, paragraphs: u16) -> bool {
    let end = psp_segment as u32 + paragraphs as u32;
    (psp_segment as u32..end).contains(&(segment as u32))
}

fn com_cpu(psp_segment: u16) -> CpuState {
    CpuState {
        cs: psp_segment,
        ds: psp_segment,
        es: psp_segment,
        ss: psp_segment,
        ip: COM_OFFSET,
        sp: 0xfffe,
        flags: 0x0002,
        ..CpuState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_psp, load_com, load_com_with_command_tail, load_program, LoaderError, PSP_SEGMENT,
    };
    use crate::{DosMemoryArena, ExecutableFormat, GuestMemory};

    fn mz_image() -> [u8; 64] {
        let mut image = [0u8; 64];
        image[..2].copy_from_slice(b"MZ");
        image[2..4].copy_from_slice(&64u16.to_le_bytes());
        image[4..6].copy_from_slice(&1u16.to_le_bytes());
        image[6..8].copy_from_slice(&1u16.to_le_bytes());
        image[8..10].copy_from_slice(&2u16.to_le_bytes());
        image[0x18..0x1a].copy_from_slice(&0x1cu16.to_le_bytes());
        image[0x1c..0x20].copy_from_slice(&0u32.to_le_bytes());
        image[32..34].copy_from_slice(&0x1234u16.to_le_bytes());
        image
    }

    #[test]
    fn com_image_and_psp_are_initialized_at_the_conventional_offsets() {
        let mut memory = GuestMemory::new();
        let cpu = load_com(&mut memory, &[0x90, 0xf4]).unwrap();
        assert_eq!(memory.read_u16(PSP_SEGMENT, 0), 0x20cd);
        assert_eq!(memory.read_u8(PSP_SEGMENT, 0x80), 0);
        assert_eq!(memory.read_u8(PSP_SEGMENT, 0x100), 0x90);
        assert_eq!(cpu.cs, PSP_SEGMENT);
    }

    #[test]
    fn loader_keeps_bounded_com_tails_and_rejects_mz_as_com() {
        let mut memory = GuestMemory::new();
        load_com_with_command_tail(&mut memory, &[0x90], b" ONE TWO").unwrap();
        assert_eq!(memory.read_u8(PSP_SEGMENT, 0x80), 8);
        assert!(matches!(
            load_com(&mut memory, b"MZ"),
            Err(LoaderError::Mz(_))
        ));
    }

    #[test]
    fn mz_loader_places_psp_image_relocations_and_registers() {
        let mut memory = GuestMemory::new();
        let mut arena = DosMemoryArena::new();
        let program = load_program(&mut memory, &mut arena, &mz_image(), b"", None, 0).unwrap();
        assert_eq!(program.format, ExecutableFormat::Mz);
        assert_eq!(program.load_segment, program.psp_segment + 0x10);
        assert_eq!(
            memory.read_u16(program.load_segment, 0),
            0x1234 + program.load_segment
        );
        assert_eq!(program.cpu.cs, program.load_segment);
        assert_eq!(program.cpu.ds, program.psp_segment);
        assert_eq!(
            memory.read_u16(program.psp_segment, 2),
            program.psp_segment + program.paragraphs
        );
    }

    #[test]
    fn mz_loader_keeps_unrequested_conventional_memory_available_for_children() {
        let mut memory = GuestMemory::new();
        let mut arena = DosMemoryArena::new();
        let program = load_program(&mut memory, &mut arena, &mz_image(), b"", None, 0).unwrap();

        assert_eq!(program.paragraphs, 0x12);
        assert!(arena.largest_available() > 0x8000);
    }

    #[test]
    fn psp_carries_parent_environment_and_default_handles() {
        let mut memory = GuestMemory::new();
        build_psp(&mut memory, 0x2000, Some(0x1000), 0x2200, 0x3000, b"x").unwrap();
        assert_eq!(memory.read_u8(0x1fff, 0), b'Z');
        assert_eq!(memory.read_u16(0x1fff, 1), 0x2000);
        assert_eq!(memory.read_u16(0x1fff, 3), 0x1000);
        assert_eq!(memory.read_u16(0x2000, 0x16), 0x1000);
        assert_eq!(memory.read_u16(0x2000, 0x2c), 0x2200);
        assert_eq!(memory.read_u8(0x2000, 0x18), 0);
        assert_eq!(memory.read_u8(0x2000, 0x80), 1);
    }
}
