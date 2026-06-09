#![no_std]

/// Minimal ELF64 parser for static executables.

/// Parsed ELF header info.
#[derive(Debug, Clone, Copy)]
pub struct ElfHeader {
    pub entry: u64,
    pub phoff: u64,
    pub phentsize: u16,
    pub phnum: u16,
    pub elf_type: u16,
}

/// ELF program header (segment) info.
#[derive(Debug, Clone, Copy)]
pub struct ElfSegment {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
}

/// Errors from ELF parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    NotElf,
    Not64Bit,
    NotStaticExecutable,
    HasDynamicSection,
    SegmentMapFailed,
    InvalidEntry,
    Truncated,
}

/// Parse ELF64 header and verify it is a static ET_EXEC.
pub fn parse_elf_header(elf_bytes: &[u8]) -> Result<ElfHeader, ElfError> {
    if elf_bytes.len() < 64 {
        return Err(ElfError::Truncated);
    }
    if elf_bytes[0..4] != [0x7f, b'E', b'L', b'F'] {
        return Err(ElfError::NotElf);
    }
    // EI_CLASS at offset 4: must be ELFCLASS64 (2)
    if elf_bytes[4] != 2 {
        return Err(ElfError::Not64Bit);
    }
    // e_type at offset 0x10
    let elf_type = u16::from_le_bytes(elf_bytes[0x10..0x12].try_into().unwrap());
    if elf_type != 2 {
        // ET_EXEC = 2
        return Err(ElfError::NotStaticExecutable);
    }
    let entry = u64::from_le_bytes(elf_bytes[0x18..0x20].try_into().unwrap());
    let phoff = u64::from_le_bytes(elf_bytes[0x20..0x28].try_into().unwrap());
    let phentsize = u16::from_le_bytes(elf_bytes[0x36..0x38].try_into().unwrap());
    let phnum = u16::from_le_bytes(elf_bytes[0x38..0x3A].try_into().unwrap());
    if entry == 0 {
        return Err(ElfError::InvalidEntry);
    }
    Ok(ElfHeader {
        entry,
        phoff,
        phentsize,
        phnum,
        elf_type,
    })
}

/// Iterate over PT_LOAD segments in an ELF64 binary.
/// `f` is called for each segment; if it returns Err, iteration stops.
pub fn for_each_load_segment<F>(
    elf_bytes: &[u8],
    header: &ElfHeader,
    mut f: F,
) -> Result<(), ElfError>
where
    F: FnMut(&ElfSegment) -> Result<(), ElfError>,
{
    for i in 0..header.phnum {
        let ph_start = header.phoff as usize + (i as usize * header.phentsize as usize);
        let ph_end = ph_start + header.phentsize as usize;
        if ph_end > elf_bytes.len() {
            return Err(ElfError::Truncated);
        }
        let ph = &elf_bytes[ph_start..ph_end];
        let p_type = u32::from_le_bytes(ph[0x00..0x04].try_into().unwrap());
        if p_type == 3 {
            // PT_INTERP — dynamic linker needed
            return Err(ElfError::HasDynamicSection);
        }
        if p_type != 1 {
            // Not PT_LOAD
            continue;
        }
        let p_flags = u32::from_le_bytes(ph[0x04..0x08].try_into().unwrap());
        let p_offset = u64::from_le_bytes(ph[0x08..0x10].try_into().unwrap());
        let p_vaddr = u64::from_le_bytes(ph[0x10..0x18].try_into().unwrap());
        let p_filesz = u64::from_le_bytes(ph[0x20..0x28].try_into().unwrap());
        let p_memsz = u64::from_le_bytes(ph[0x28..0x30].try_into().unwrap());
        let seg = ElfSegment {
            p_type,
            p_flags,
            p_offset,
            p_vaddr,
            p_filesz,
            p_memsz,
        };
        f(&seg)?;
    }
    Ok(())
}

/// Check if an ELF has a PT_DYNAMIC segment.
pub fn has_dynamic_section(elf_bytes: &[u8], header: &ElfHeader) -> bool {
    for i in 0..header.phnum {
        let ph_start = header.phoff as usize + (i as usize * header.phentsize as usize);
        let ph_end = ph_start + header.phentsize as usize;
        if ph_end > elf_bytes.len() {
            break;
        }
        let p_type = u32::from_le_bytes(
            elf_bytes[ph_start..ph_start + 4].try_into().unwrap()
        );
        if p_type == 2 {
            // PT_DYNAMIC
            return true;
        }
    }
    false
}
