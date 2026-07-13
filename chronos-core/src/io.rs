use crate::{default_vga_dac_entries, CpuState, Rgb8, VgaDacEntry};

pub const VGA_DAC_READ_INDEX_PORT: u16 = 0x03c7;
pub const VGA_DAC_WRITE_INDEX_PORT: u16 = 0x03c8;
pub const VGA_DAC_DATA_PORT: u16 = 0x03c9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoOperation {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoWidth {
    Byte,
    Word,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoTrap {
    pub operation: IoOperation,
    pub port: u16,
    pub width: IoWidth,
    pub value: Option<u16>,
}

impl IoTrap {
    const fn unsupported(
        operation: IoOperation,
        port: u16,
        width: IoWidth,
        value: Option<u16>,
    ) -> Self {
        Self {
            operation,
            port,
            width,
            value,
        }
    }
}

/// A virtual device boundary. Implementations operate entirely on guest-owned
/// state; no implementation in Chronos contains a host-native port instruction.
pub trait GuestIoDevice {
    fn read_u8(&mut self, port: u16) -> Result<u8, IoTrap>;
    fn write_u8(&mut self, port: u16, value: u8) -> Result<(), IoTrap>;
    fn read_u16(&mut self, port: u16) -> Result<u16, IoTrap>;
    fn write_u16(&mut self, port: u16, value: u16) -> Result<(), IoTrap>;
}

#[derive(Clone, Debug)]
pub struct VgaDac {
    entries: [VgaDacEntry; 256],
    palette: [Rgb8; 256],
    write_index: u8,
    write_component: u8,
    pending_write: [u8; 3],
    read_index: u8,
    read_component: u8,
    palette_generation: u64,
}

impl VgaDac {
    pub fn new() -> Self {
        let entries = default_vga_dac_entries();
        let palette = core::array::from_fn(|index| entries[index].to_rgb8());
        Self {
            entries,
            palette,
            write_index: 0,
            write_component: 0,
            pending_write: [0; 3],
            read_index: 0,
            read_component: 0,
            palette_generation: 0,
        }
    }

    pub fn reset_default(&mut self) {
        self.entries = default_vga_dac_entries();
        self.palette = core::array::from_fn(|index| self.entries[index].to_rgb8());
        self.reset_sequence();
        self.palette_generation = self.palette_generation.wrapping_add(1);
    }

    fn reset_sequence(&mut self) {
        self.write_index = 0;
        self.write_component = 0;
        self.pending_write = [0; 3];
        self.read_index = 0;
        self.read_component = 0;
    }

    pub const fn entries(&self) -> &[VgaDacEntry; 256] {
        &self.entries
    }

    pub const fn palette(&self) -> &[Rgb8; 256] {
        &self.palette
    }

    pub const fn palette_generation(&self) -> u64 {
        self.palette_generation
    }

    pub fn palette_checksum(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for entry in &self.entries {
            for component in [entry.red_6bit, entry.green_6bit, entry.blue_6bit] {
                hash ^= component as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    fn set_write_index(&mut self, index: u8) {
        self.write_index = index;
        self.write_component = 0;
        self.pending_write = [0; 3];
    }

    fn set_read_index(&mut self, index: u8) {
        self.read_index = index;
        self.read_component = 0;
    }

    fn write_data(&mut self, value: u8) -> bool {
        self.pending_write[self.write_component as usize] = value.min(63);
        self.write_component += 1;
        if self.write_component != 3 {
            return false;
        }
        let entry = VgaDacEntry::new(
            self.pending_write[0],
            self.pending_write[1],
            self.pending_write[2],
        );
        self.entries[self.write_index as usize] = entry;
        self.palette[self.write_index as usize] = entry.to_rgb8();
        self.write_index = self.write_index.wrapping_add(1);
        self.write_component = 0;
        self.pending_write = [0; 3];
        self.palette_generation = self.palette_generation.wrapping_add(1);
        true
    }

    fn read_data(&mut self) -> u8 {
        let entry = self.entries[self.read_index as usize];
        let value = match self.read_component {
            0 => entry.red_6bit,
            1 => entry.green_6bit,
            _ => entry.blue_6bit,
        };
        self.read_component += 1;
        if self.read_component == 3 {
            self.read_component = 0;
            self.read_index = self.read_index.wrapping_add(1);
        }
        value
    }
}

impl Default for VgaDac {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-runtime allowlist dispatcher. Word accesses use two little-endian byte
/// cycles (`low -> port`, `high -> port + 1`) only after both cycles have been
/// validated, so an unsupported adjacent port cannot cause a partial write.
#[derive(Clone, Debug, Default)]
pub struct GuestIoDispatcher {
    vga_dac: VgaDac,
    entries_committed_slice: u32,
    unsupported_attempts: u64,
}

impl GuestIoDispatcher {
    pub const fn vga_dac(&self) -> &VgaDac {
        &self.vga_dac
    }

    pub fn reset_mode13(&mut self) {
        self.vga_dac.reset_default();
    }

    pub fn begin_slice(&mut self) {
        self.entries_committed_slice = 0;
    }

    pub const fn entries_committed_slice(&self) -> u32 {
        self.entries_committed_slice
    }

    pub const fn unsupported_attempts(&self) -> u64 {
        self.unsupported_attempts
    }

    const fn can_read_u8(port: u16) -> bool {
        port == VGA_DAC_DATA_PORT
    }

    const fn can_write_u8(port: u16) -> bool {
        matches!(
            port,
            VGA_DAC_READ_INDEX_PORT | VGA_DAC_WRITE_INDEX_PORT | VGA_DAC_DATA_PORT
        )
    }

    fn reject<T>(
        &mut self,
        operation: IoOperation,
        port: u16,
        width: IoWidth,
        value: Option<u16>,
    ) -> Result<T, IoTrap> {
        self.unsupported_attempts = self.unsupported_attempts.wrapping_add(1);
        Err(IoTrap::unsupported(operation, port, width, value))
    }
}

impl GuestIoDevice for GuestIoDispatcher {
    fn read_u8(&mut self, port: u16) -> Result<u8, IoTrap> {
        // Chronos does not fabricate VGA status values for 03C7h/03C8h.
        // Until their real readback semantics are needed, those reads trap;
        // only the sequential DAC data read at 03C9h is supported.
        if port == VGA_DAC_DATA_PORT {
            Ok(self.vga_dac.read_data())
        } else {
            self.reject(IoOperation::Read, port, IoWidth::Byte, None)
        }
    }

    fn write_u8(&mut self, port: u16, value: u8) -> Result<(), IoTrap> {
        match port {
            VGA_DAC_READ_INDEX_PORT => self.vga_dac.set_read_index(value),
            VGA_DAC_WRITE_INDEX_PORT => self.vga_dac.set_write_index(value),
            VGA_DAC_DATA_PORT => {
                if self.vga_dac.write_data(value) {
                    self.entries_committed_slice = self.entries_committed_slice.saturating_add(1);
                }
            }
            _ => return self.reject(IoOperation::Write, port, IoWidth::Byte, Some(value as u16)),
        }
        Ok(())
    }

    fn read_u16(&mut self, port: u16) -> Result<u16, IoTrap> {
        let high_port = port.wrapping_add(1);
        if !Self::can_read_u8(port) || !Self::can_read_u8(high_port) {
            return self.reject(IoOperation::Read, port, IoWidth::Word, None);
        }
        let low = self.read_u8(port)?;
        let high = self.read_u8(high_port)?;
        Ok(u16::from_le_bytes([low, high]))
    }

    fn write_u16(&mut self, port: u16, value: u16) -> Result<(), IoTrap> {
        let high_port = port.wrapping_add(1);
        if !Self::can_write_u8(port) || !Self::can_write_u8(high_port) {
            return self.reject(IoOperation::Write, port, IoWidth::Word, Some(value));
        }
        let [low, high] = value.to_le_bytes();
        self.write_u8(port, low)?;
        self.write_u8(high_port, high)
    }
}

pub(crate) fn execute_io_instruction<D: GuestIoDevice>(
    device: &mut D,
    cpu: &mut CpuState,
    opcode: u8,
    immediate_port: Option<u8>,
) -> Result<(), IoTrap> {
    let port = immediate_port.map_or(cpu.dx, u16::from);
    match opcode {
        0xe4 | 0xec => cpu.set_al(device.read_u8(port)?),
        0xe5 | 0xed => cpu.ax = device.read_u16(port)?,
        0xe6 | 0xee => device.write_u8(port, cpu.al())?,
        0xe7 | 0xef => device.write_u16(port, cpu.ax)?,
        _ => unreachable!("caller only dispatches x86 IN/OUT opcodes"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    #[derive(Default)]
    struct MockIo {
        reads: Vec<(u16, IoWidth)>,
        writes: Vec<(u16, IoWidth, u16)>,
    }

    impl GuestIoDevice for MockIo {
        fn read_u8(&mut self, port: u16) -> Result<u8, IoTrap> {
            self.reads.push((port, IoWidth::Byte));
            Ok(0x5a)
        }
        fn write_u8(&mut self, port: u16, value: u8) -> Result<(), IoTrap> {
            self.writes.push((port, IoWidth::Byte, value as u16));
            Ok(())
        }
        fn read_u16(&mut self, port: u16) -> Result<u16, IoTrap> {
            self.reads.push((port, IoWidth::Word));
            Ok(0xa55a)
        }
        fn write_u16(&mut self, port: u16, value: u16) -> Result<(), IoTrap> {
            self.writes.push((port, IoWidth::Word, value));
            Ok(())
        }
    }

    #[test]
    fn all_8086_in_out_forms_route_once_and_preserve_flags_and_dx() {
        for (opcode, immediate, expected_port, width, write) in [
            (0xe4, Some(0x77), 0x0077, IoWidth::Byte, false),
            (0xe5, Some(0x77), 0x0077, IoWidth::Word, false),
            (0xec, None, 0x3456, IoWidth::Byte, false),
            (0xed, None, 0x3456, IoWidth::Word, false),
            (0xe6, Some(0x77), 0x0077, IoWidth::Byte, true),
            (0xe7, Some(0x77), 0x0077, IoWidth::Word, true),
            (0xee, None, 0x3456, IoWidth::Byte, true),
            (0xef, None, 0x3456, IoWidth::Word, true),
        ] {
            let mut io = MockIo::default();
            let mut cpu = CpuState {
                ax: 0x1234,
                dx: 0x3456,
                flags: 0x0ad7,
                ..CpuState::default()
            };
            execute_io_instruction(&mut io, &mut cpu, opcode, immediate).unwrap();
            assert_eq!(cpu.dx, 0x3456);
            assert_eq!(cpu.flags, 0x0ad7);
            if write {
                assert_eq!(
                    io.writes,
                    vec![(
                        expected_port,
                        width,
                        0x1234 & if width == IoWidth::Byte { 0xff } else { 0xffff }
                    )]
                );
                assert!(io.reads.is_empty());
                assert_eq!(cpu.ax, 0x1234);
            } else {
                assert_eq!(io.reads, vec![(expected_port, width)]);
                assert!(io.writes.is_empty());
                assert_eq!(
                    cpu.ax,
                    if width == IoWidth::Byte {
                        0x125a
                    } else {
                        0xa55a
                    }
                );
            }
        }
    }

    #[test]
    fn dac_write_and_read_sequences_commit_atomically_and_wrap() {
        let mut io = GuestIoDispatcher::default();
        io.write_u8(VGA_DAC_WRITE_INDEX_PORT, 255).unwrap();
        io.write_u8(VGA_DAC_DATA_PORT, 99).unwrap();
        io.write_u8(VGA_DAC_DATA_PORT, 2).unwrap();
        assert_eq!(io.vga_dac().palette_generation(), 0);
        assert_ne!(io.vga_dac().entries()[255], VgaDacEntry::new(63, 2, 3));
        io.write_u8(VGA_DAC_DATA_PORT, 3).unwrap();
        assert_eq!(io.vga_dac().entries()[255], VgaDacEntry::new(63, 2, 3));
        assert_eq!(io.vga_dac().palette_generation(), 1);
        assert_eq!(io.entries_committed_slice(), 1);

        io.write_u8(VGA_DAC_DATA_PORT, 4).unwrap();
        io.write_u8(VGA_DAC_DATA_PORT, 5).unwrap();
        io.write_u8(VGA_DAC_DATA_PORT, 6).unwrap();
        assert_eq!(io.vga_dac().entries()[0], VgaDacEntry::new(4, 5, 6));

        io.write_u8(VGA_DAC_READ_INDEX_PORT, 255).unwrap();
        assert_eq!(io.read_u8(VGA_DAC_DATA_PORT).unwrap(), 63);
        assert_eq!(io.read_u8(VGA_DAC_DATA_PORT).unwrap(), 2);
        assert_eq!(io.read_u8(VGA_DAC_DATA_PORT).unwrap(), 3);
        assert_eq!(io.read_u8(VGA_DAC_DATA_PORT).unwrap(), 4);
    }

    #[test]
    fn resetting_indices_discards_incomplete_writes_and_read_write_state_is_independent() {
        let mut io = GuestIoDispatcher::default();
        let original = io.vga_dac().entries()[32];
        io.write_u8(VGA_DAC_WRITE_INDEX_PORT, 32).unwrap();
        io.write_u8(VGA_DAC_DATA_PORT, 1).unwrap();
        io.write_u8(VGA_DAC_READ_INDEX_PORT, 32).unwrap();
        assert_eq!(io.read_u8(VGA_DAC_DATA_PORT).unwrap(), original.red_6bit);
        io.write_u8(VGA_DAC_WRITE_INDEX_PORT, 33).unwrap();
        assert_eq!(io.vga_dac().entries()[32], original);
        assert_eq!(io.vga_dac().palette_generation(), 0);
        assert_eq!(io.read_u8(VGA_DAC_DATA_PORT).unwrap(), original.green_6bit);
        io.write_u8(VGA_DAC_READ_INDEX_PORT, 32).unwrap();
        assert_eq!(
            io.read_u8(VGA_DAC_DATA_PORT).unwrap(),
            original.red_6bit,
            "a new read index restarts at red"
        );
    }

    #[test]
    fn unsupported_accesses_are_explicit_and_word_writes_are_prevalidated() {
        let mut io = GuestIoDispatcher::default();
        let generation = io.vga_dac().palette_generation();
        assert_eq!(
            io.write_u16(VGA_DAC_DATA_PORT, 0x1234),
            Err(IoTrap::unsupported(
                IoOperation::Write,
                VGA_DAC_DATA_PORT,
                IoWidth::Word,
                Some(0x1234)
            ))
        );
        assert_eq!(io.vga_dac().palette_generation(), generation);
        assert_eq!(
            io.read_u8(VGA_DAC_READ_INDEX_PORT),
            Err(IoTrap::unsupported(
                IoOperation::Read,
                VGA_DAC_READ_INDEX_PORT,
                IoWidth::Byte,
                None
            ))
        );
        assert_eq!(
            io.write_u8(0x1234, 7),
            Err(IoTrap::unsupported(
                IoOperation::Write,
                0x1234,
                IoWidth::Byte,
                Some(7)
            ))
        );
    }
}
