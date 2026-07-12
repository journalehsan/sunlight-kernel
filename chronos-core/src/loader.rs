use crate::{CpuState, GuestMemory, MemoryError, MEMORY_SIZE};

/// Deterministic initial PSP segment for every Chronos COM guest.
pub const PSP_SEGMENT: u16 = 0x1000;
const COM_OFFSET: u16 = 0x0100;
const PROCESS_BYTES: usize = 0x1_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoaderError {
    MzExecutableUnsupported,
    ProgramTooLarge { size: usize, maximum: usize },
    Memory(MemoryError),
}

impl From<MemoryError> for LoaderError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

/// Installs a small PSP and loads a `.COM` image at `PSP:0100`.
pub fn load_com(memory: &mut GuestMemory, image: &[u8]) -> Result<CpuState, LoaderError> {
    if image.starts_with(b"MZ") {
        return Err(LoaderError::MzExecutableUnsupported);
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

    // PSP:0000 is a legacy INT 20h termination entry point. PSP:0080 holds
    // the command-tail length, zero for this initial no-arguments runtime.
    memory.write_slice(PSP_SEGMENT, 0, &[0xcd, 0x20])?;
    memory.write_u8(PSP_SEGMENT, 0x0080, 0);
    memory.write_slice(PSP_SEGMENT, COM_OFFSET, image)?;

    Ok(CpuState {
        cs: PSP_SEGMENT,
        ds: PSP_SEGMENT,
        es: PSP_SEGMENT,
        ss: PSP_SEGMENT,
        ip: COM_OFFSET,
        sp: 0xfffe,
        flags: 0x0002,
        ..CpuState::default()
    })
}

#[cfg(test)]
mod tests {
    use super::{load_com, LoaderError, PSP_SEGMENT};
    use crate::GuestMemory;

    #[test]
    fn com_image_and_psp_are_initialized_at_the_conventional_offsets() {
        let mut memory = GuestMemory::new();
        let cpu = load_com(&mut memory, &[0x90, 0xf4]).unwrap();

        assert_eq!(memory.read_u16(PSP_SEGMENT, 0), 0x20cd);
        assert_eq!(memory.read_u8(PSP_SEGMENT, 0x80), 0);
        assert_eq!(memory.read_u8(PSP_SEGMENT, 0x100), 0x90);
        assert_eq!(cpu.cs, PSP_SEGMENT);
        assert_eq!(cpu.ip, 0x100);
        assert_eq!(cpu.sp, 0xfffe);
    }

    #[test]
    fn loader_rejects_programs_that_do_not_fit_the_process_segment() {
        let mut memory = GuestMemory::new();
        let image = [0u8; 0xff01];
        assert!(matches!(
            load_com(&mut memory, &image),
            Err(LoaderError::ProgramTooLarge { .. })
        ));
    }

    #[test]
    fn loader_explicitly_rejects_mz_executables() {
        let mut memory = GuestMemory::new();
        assert_eq!(
            load_com(&mut memory, b"MZ"),
            Err(LoaderError::MzExecutableUnsupported)
        );
    }
}
