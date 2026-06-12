#![no_std]

//! Minimal ELF64 parser and load-plan validator for static executables.
//!
//! The kernel must not map whatever addresses a binary asks for: a hostile
//! ELF could request kernel-half pages. [`plan_segments`] validates every
//! PT_LOAD against a caller-supplied user-address window and W^X before the
//! kernel touches the address space.

#[cfg(test)]
extern crate std;

pub const EM_X86_64: u16 = 0x3E;
pub const ELFOSABI_LINUX: u8 = 3;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;

const PF_X: u32 = 0x1;
const PF_W: u32 = 0x2;

const PAGE_SIZE: u64 = 4096;

/// Parsed ELF header info.
#[derive(Debug, Clone, Copy)]
pub struct ElfHeader {
    pub entry: u64,
    pub phoff: u64,
    pub phentsize: u16,
    pub phnum: u16,
    pub elf_type: u16,
    pub machine: u16,
    pub osabi: u8,
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

/// Access protection for a loadable segment. W^X is enforced during
/// validation, so writable-executable never occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentProt {
    Read,
    ReadWrite,
    ReadExec,
}

/// A validated mapping plan for one PT_LOAD segment. All address arithmetic
/// is overflow-checked and bounds-checked before this is produced, so the
/// kernel can execute it mechanically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentPlan {
    /// Exact virtual start of the segment (`p_vaddr`).
    pub vaddr: u64,
    /// `vaddr` rounded down to a page boundary.
    pub vaddr_page_start: u64,
    /// Number of 4 KiB pages spanning the segment.
    pub page_count: usize,
    /// Offset of the segment's bytes within the ELF file.
    pub file_offset: u64,
    /// Bytes to copy from the file (`p_filesz`).
    pub file_size: u64,
    /// Bytes occupied in memory (`p_memsz`); the tail past `file_size`
    /// is zero-filled (.bss).
    pub mem_size: u64,
    pub prot: SegmentProt,
}

/// Errors from ELF parsing and validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    NotElf,
    Not64Bit,
    NotStaticExecutable,
    WrongMachine,
    HasDynamicSection,
    SegmentMapFailed,
    InvalidEntry,
    Truncated,
    /// A PT_LOAD lies (partly) outside the allowed user-address window.
    SegmentOutOfRange,
    /// `p_filesz` exceeds `p_memsz`.
    InvalidSegment,
    /// A segment is both writable and executable.
    WritableExecutable,
    /// The entry point is not inside an executable PT_LOAD.
    EntryNotExecutable,
}

/// Parse ELF64 header and verify it is a static ET_EXEC for x86-64.
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
    let osabi = elf_bytes[0x07];
    // e_type at offset 0x10
    let elf_type = u16::from_le_bytes(elf_bytes[0x10..0x12].try_into().unwrap());
    if elf_type != 2 {
        // ET_EXEC = 2
        return Err(ElfError::NotStaticExecutable);
    }
    let machine = u16::from_le_bytes(elf_bytes[0x12..0x14].try_into().unwrap());
    if machine != EM_X86_64 {
        return Err(ElfError::WrongMachine);
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
        machine,
        osabi,
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
        if p_type == PT_INTERP {
            // Dynamic linker needed
            return Err(ElfError::HasDynamicSection);
        }
        if p_type != PT_LOAD {
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
        if p_type == PT_DYNAMIC {
            return true;
        }
    }
    false
}

fn validate_segment(
    seg: &ElfSegment,
    elf_len: u64,
    user_lo: u64,
    user_hi: u64,
) -> Result<Option<SegmentPlan>, ElfError> {
    if seg.p_memsz == 0 {
        return Ok(None);
    }
    if seg.p_filesz > seg.p_memsz {
        return Err(ElfError::InvalidSegment);
    }
    // W^X: a segment may be writable or executable, never both.
    if seg.p_flags & PF_W != 0 && seg.p_flags & PF_X != 0 {
        return Err(ElfError::WritableExecutable);
    }
    // The file range must lie within the binary.
    let file_end = seg
        .p_offset
        .checked_add(seg.p_filesz)
        .ok_or(ElfError::Truncated)?;
    if file_end > elf_len {
        return Err(ElfError::Truncated);
    }
    // The virtual range must lie within the user window. This is the check
    // that keeps hostile binaries out of the kernel higher half.
    let vaddr_end = seg
        .p_vaddr
        .checked_add(seg.p_memsz)
        .ok_or(ElfError::SegmentOutOfRange)?;
    if seg.p_vaddr < user_lo || vaddr_end > user_hi {
        return Err(ElfError::SegmentOutOfRange);
    }

    let prot = if seg.p_flags & PF_X != 0 {
        SegmentProt::ReadExec
    } else if seg.p_flags & PF_W != 0 {
        SegmentProt::ReadWrite
    } else {
        SegmentProt::Read
    };

    let page_start = seg.p_vaddr & !(PAGE_SIZE - 1);
    // user_hi is far below u64::MAX, so this cannot overflow.
    let page_end = (vaddr_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    Ok(Some(SegmentPlan {
        vaddr: seg.p_vaddr,
        vaddr_page_start: page_start,
        page_count: ((page_end - page_start) / PAGE_SIZE) as usize,
        file_offset: seg.p_offset,
        file_size: seg.p_filesz,
        mem_size: seg.p_memsz,
        prot,
    }))
}

/// Validate every PT_LOAD against `[user_lo, user_hi)` and W^X, verify the
/// entry point lands inside an executable segment, then emit one
/// [`SegmentPlan`] per segment via `emit`.
///
/// Nothing is emitted unless the whole binary validates, so the kernel never
/// partially maps a rejected executable.
pub fn plan_segments(
    elf_bytes: &[u8],
    header: &ElfHeader,
    user_lo: u64,
    user_hi: u64,
    emit: &mut dyn FnMut(&SegmentPlan),
) -> Result<(), ElfError> {
    let elf_len = elf_bytes.len() as u64;

    // Pass 1: validate everything (including entry coverage) before emitting.
    let mut entry_executable = false;
    for_each_load_segment(elf_bytes, header, |seg| {
        let Some(plan) = validate_segment(seg, elf_len, user_lo, user_hi)? else {
            return Ok(());
        };
        if plan.prot == SegmentProt::ReadExec
            && header.entry >= plan.vaddr
            && header.entry < plan.vaddr + plan.mem_size
        {
            entry_executable = true;
        }
        Ok(())
    })?;
    if !entry_executable {
        return Err(ElfError::EntryNotExecutable);
    }

    // Pass 2: emit the validated plans.
    for_each_load_segment(elf_bytes, header, |seg| {
        if let Some(plan) = validate_segment(seg, elf_len, user_lo, user_hi)? {
            emit(&plan);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    const USER_LO: u64 = 0x1000;
    const USER_HI: u64 = 0x0000_0001_0000_0000;

    struct Phdr {
        p_type: u32,
        flags: u32,
        offset: u64,
        vaddr: u64,
        filesz: u64,
        memsz: u64,
    }

    /// Build a minimal ELF64 image: header at 0, phdrs at 64, no section data
    /// beyond `total_len`.
    fn build_elf(entry: u64, phdrs: &[Phdr], total_len: usize) -> Vec<u8> {
        let mut elf = vec![0u8; total_len];
        elf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        elf[4] = 2; // ELFCLASS64
        elf[0x10..0x12].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        elf[0x12..0x14].copy_from_slice(&EM_X86_64.to_le_bytes());
        elf[0x18..0x20].copy_from_slice(&entry.to_le_bytes());
        elf[0x20..0x28].copy_from_slice(&64u64.to_le_bytes()); // phoff
        elf[0x36..0x38].copy_from_slice(&56u16.to_le_bytes()); // phentsize
        elf[0x38..0x3A].copy_from_slice(&(phdrs.len() as u16).to_le_bytes());

        for (i, ph) in phdrs.iter().enumerate() {
            let o = 64 + i * 56;
            elf[o..o + 4].copy_from_slice(&ph.p_type.to_le_bytes());
            elf[o + 4..o + 8].copy_from_slice(&ph.flags.to_le_bytes());
            elf[o + 8..o + 16].copy_from_slice(&ph.offset.to_le_bytes());
            elf[o + 16..o + 24].copy_from_slice(&ph.vaddr.to_le_bytes());
            elf[o + 32..o + 40].copy_from_slice(&ph.filesz.to_le_bytes());
            elf[o + 40..o + 48].copy_from_slice(&ph.memsz.to_le_bytes());
        }
        elf
    }

    fn plans_for(elf: &[u8]) -> Result<Vec<SegmentPlan>, ElfError> {
        let header = parse_elf_header(elf)?;
        let mut plans = Vec::new();
        plan_segments(elf, &header, USER_LO, USER_HI, &mut |p| plans.push(*p))?;
        Ok(plans)
    }

    #[test]
    fn plans_valid_text_and_data_segments() {
        let elf = build_elf(
            0x40_0100,
            &[
                Phdr { p_type: PT_LOAD, flags: PF_X | 0x4, offset: 0x200, vaddr: 0x40_0000, filesz: 0x500, memsz: 0x500 },
                Phdr { p_type: PT_LOAD, flags: PF_W | 0x4, offset: 0x700, vaddr: 0x40_1000, filesz: 0x100, memsz: 0x2100 },
            ],
            0x1000,
        );

        let plans = plans_for(&elf).expect("valid ELF");
        assert_eq!(plans.len(), 2);

        assert_eq!(plans[0].prot, SegmentProt::ReadExec);
        assert_eq!(plans[0].vaddr_page_start, 0x40_0000);
        assert_eq!(plans[0].page_count, 1);

        // 0x2100 bytes from 0x401000 → 3 pages; .bss tail beyond filesz.
        assert_eq!(plans[1].prot, SegmentProt::ReadWrite);
        assert_eq!(plans[1].page_count, 3);
        assert_eq!(plans[1].file_size, 0x100);
        assert_eq!(plans[1].mem_size, 0x2100);
    }

    #[test]
    fn rejects_kernel_half_segment() {
        let elf = build_elf(
            0xFFFF_FFFF_8000_0100,
            &[Phdr {
                p_type: PT_LOAD,
                flags: PF_X | 0x4,
                offset: 0x200,
                vaddr: 0xFFFF_FFFF_8000_0000, // KERNEL_START
                filesz: 0x500,
                memsz: 0x500,
            }],
            0x1000,
        );
        assert_eq!(plans_for(&elf), Err(ElfError::SegmentOutOfRange));
    }

    #[test]
    fn rejects_vaddr_overflow_wraparound() {
        let elf = build_elf(
            0x40_0100,
            &[
                Phdr { p_type: PT_LOAD, flags: PF_X | 0x4, offset: 0x200, vaddr: 0x40_0000, filesz: 0x100, memsz: 0x100 },
                Phdr { p_type: PT_LOAD, flags: 0x4, offset: 0x200, vaddr: u64::MAX - 0xFFF, filesz: 0x100, memsz: 0x2000 },
            ],
            0x1000,
        );
        assert_eq!(plans_for(&elf), Err(ElfError::SegmentOutOfRange));
    }

    #[test]
    fn rejects_writable_executable_segment() {
        let elf = build_elf(
            0x40_0100,
            &[Phdr {
                p_type: PT_LOAD,
                flags: PF_X | PF_W | 0x4,
                offset: 0x200,
                vaddr: 0x40_0000,
                filesz: 0x100,
                memsz: 0x100,
            }],
            0x1000,
        );
        assert_eq!(plans_for(&elf), Err(ElfError::WritableExecutable));
    }

    #[test]
    fn rejects_entry_outside_executable_segment() {
        // Entry points into the data segment, not text.
        let elf = build_elf(
            0x40_1000,
            &[
                Phdr { p_type: PT_LOAD, flags: PF_X | 0x4, offset: 0x200, vaddr: 0x40_0000, filesz: 0x100, memsz: 0x100 },
                Phdr { p_type: PT_LOAD, flags: PF_W | 0x4, offset: 0x300, vaddr: 0x40_1000, filesz: 0x100, memsz: 0x100 },
            ],
            0x1000,
        );
        assert_eq!(plans_for(&elf), Err(ElfError::EntryNotExecutable));
    }

    #[test]
    fn rejects_filesz_larger_than_memsz() {
        let elf = build_elf(
            0x40_0100,
            &[Phdr {
                p_type: PT_LOAD,
                flags: PF_X | 0x4,
                offset: 0x200,
                vaddr: 0x40_0000,
                filesz: 0x200,
                memsz: 0x100,
            }],
            0x1000,
        );
        assert_eq!(plans_for(&elf), Err(ElfError::InvalidSegment));
    }

    #[test]
    fn rejects_file_range_past_end_of_binary() {
        let elf = build_elf(
            0x40_0100,
            &[Phdr {
                p_type: PT_LOAD,
                flags: PF_X | 0x4,
                offset: 0xF00,
                vaddr: 0x40_0000,
                filesz: 0x500, // 0xF00 + 0x500 > 0x1000
                memsz: 0x500,
            }],
            0x1000,
        );
        assert_eq!(plans_for(&elf), Err(ElfError::Truncated));
    }

    #[test]
    fn rejects_wrong_machine() {
        let mut elf = build_elf(0x40_0100, &[], 0x100);
        elf[0x12..0x14].copy_from_slice(&0xB7u16.to_le_bytes()); // EM_AARCH64
        assert_eq!(parse_elf_header(&elf).unwrap_err(), ElfError::WrongMachine);
    }

    #[test]
    fn header_carries_osabi() {
        let mut elf = build_elf(0x40_0100, &[], 0x100);
        elf[0x07] = ELFOSABI_LINUX;
        assert_eq!(parse_elf_header(&elf).unwrap().osabi, ELFOSABI_LINUX);
    }
}
