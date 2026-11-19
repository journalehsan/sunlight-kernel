use crate::text_mode::{TEXT_ROWS, VIDEO_BYTES, VIDEO_PHYSICAL};
use crate::video::{
    VGA_FRAMEBUFFER_BYTES, VGA_FRAMEBUFFER_PHYSICAL, VGA_FRAMEBUFFER_SEGMENT, VGA_HEIGHT, VGA_WIDTH,
};
use alloc::vec;
use alloc::vec::Vec;

/// DOS real-mode physical address space.
pub const MEMORY_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: usize = MEMORY_SIZE - 1;

pub(crate) const INT15_SEGMENT: u16 = 0xf000;
pub(crate) const INT15_OFFSET: u16 = 0xe000;

// Chronos implements no INT 15h subservices. Return the AT unsupported-function
// status in AH and CF, preserving AL, other registers and non-CF caller flags.
// Edit the saved FLAGS so IRET works for INT, tail jumps, and PUSHF/CALL FAR.
const INT15_FALLBACK: [u8; 11] = [
    0x55, // push bp
    0x89, 0xe5, // mov bp, sp
    0x83, 0x4e, 0x06, 0x01, // or word [bp+6], 1 (defaults to SS)
    0x5d, // pop bp
    0xb4, 0x86, // mov ah, 86h
    0xcf, // iret
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    SliceTooLarge { requested: usize },
}

/// Safe, one-megabyte guest memory with 20-bit real-mode address wrapping.
pub struct GuestMemory {
    bytes: Vec<u8>,
    video_dirty_rows: [bool; TEXT_ROWS],
    firmware_installed: bool,
    vga_addressing_disabled: bool,
    graphics_dirty_rows: [bool; VGA_HEIGHT],
    framebuffer_generation: u64,
}

impl GuestMemory {
    pub fn new() -> Self {
        Self {
            bytes: vec![0; MEMORY_SIZE],
            video_dirty_rows: [true; TEXT_ROWS],
            firmware_installed: false,
            vga_addressing_disabled: false,
            graphics_dirty_rows: [false; VGA_HEIGHT],
            framebuffer_generation: 0,
        }
    }

    /// Host initialization only. Guest stores to this installed ROM are ignored,
    /// like writes to physical ROM, rather than raising invented CPU faults.
    pub(crate) fn install_firmware(&mut self) {
        let start = crate::font::ROM_START;
        self.bytes[start..start + 2048].copy_from_slice(crate::font::FONT_8);
        self.bytes[start + 2048..crate::font::ROM_END].copy_from_slice(crate::font::FONT_16);
        let start = Self::physical_address(INT15_SEGMENT, INT15_OFFSET);
        self.bytes[start..start + INT15_FALLBACK.len()].copy_from_slice(&INT15_FALLBACK);
        self.firmware_installed = true;
    }

    fn is_rom(&self, address: usize) -> bool {
        let int15 = Self::physical_address(INT15_SEGMENT, INT15_OFFSET);
        self.firmware_installed
            && ((crate::font::ROM_START..crate::font::ROM_END).contains(&address)
                || (int15..int15 + INT15_FALLBACK.len()).contains(&address))
    }

    pub fn clear(&mut self) {
        self.bytes.fill(0);
        self.firmware_installed = false;
        self.vga_addressing_disabled = false;
        self.video_dirty_rows = [true; TEXT_ROWS];
        self.graphics_dirty_rows = [false; VGA_HEIGHT];
        self.framebuffer_generation = 0;
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Calculates `((segment << 4) + offset) & 0xFFFFF`.
    #[inline]
    pub const fn physical_address(segment: u16, offset: u16) -> usize {
        (((segment as usize) << 4) + offset as usize) & ADDRESS_MASK
    }

    /// Physical start of a guest buffer when the whole range fits inside the
    /// one-megabyte real-mode address space. Unlike `physical_address`, this
    /// refuses to wrap: a segment:offset pair whose range crosses the top of
    /// memory is not a valid save/restore buffer.
    pub const fn checked_buffer_start(segment: u16, offset: u16, len: usize) -> Option<usize> {
        if len > 0x10000 - offset as usize {
            return None;
        }
        let start = ((segment as usize) << 4) + offset as usize;
        match start.checked_add(len) {
            Some(end) if end <= MEMORY_SIZE => Some(start),
            _ => None,
        }
    }

    /// True when every byte in `start..start+len` is writable guest RAM.
    pub fn range_is_ram(&self, start: usize, len: usize) -> bool {
        if len > MEMORY_SIZE {
            return false;
        }
        match start.checked_add(len) {
            Some(end) if end <= MEMORY_SIZE => {
                (start..end).all(|address| !self.is_rom(address) && !self.vga_disconnected(address))
            }
            _ => false,
        }
    }

    #[inline]
    fn wrapped_address(address: usize) -> usize {
        address & ADDRESS_MASK
    }

    /// CPU-side decoding for the single virtual color VGA adapter.
    pub const fn vga_addressing_enabled(&self) -> bool {
        !self.vga_addressing_disabled
    }

    pub(crate) fn set_vga_addressing_enabled(&mut self, enabled: bool) {
        self.vga_addressing_disabled = !enabled;
    }

    fn vga_disconnected(&self, address: usize) -> bool {
        self.vga_addressing_disabled && matches!(address, 0xa0000..=0xaffff | 0xb8000..=0xbffff)
    }

    fn read_bus_byte(&self, address: usize) -> u8 {
        if self.vga_disconnected(address) {
            0xff
        } else {
            self.bytes[address]
        }
    }

    fn write_bus_byte(&mut self, address: usize, value: u8) {
        if !self.vga_disconnected(address) && !self.is_rom(address) && self.bytes[address] != value
        {
            self.bytes[address] = value;
            self.mark_video_dirty(address);
        }
    }

    /// Display-side fetch, independent of the CPU's address decoder.
    pub(crate) fn read_display_byte(&self, segment: u16, offset: u16) -> u8 {
        self.bytes[Self::physical_address(segment, offset)]
    }

    #[inline]
    pub fn read_u8(&self, segment: u16, offset: u16) -> u8 {
        self.read_bus_byte(Self::physical_address(segment, offset))
    }

    #[inline]
    pub fn write_u8(&mut self, segment: u16, offset: u16, value: u8) {
        self.write_bus_byte(Self::physical_address(segment, offset), value);
    }

    pub fn read_u16(&self, segment: u16, offset: u16) -> u16 {
        let lo = self.read_u8(segment, offset);
        let hi = self.read_u8(segment, offset.wrapping_add(1));
        u16::from_le_bytes([lo, hi])
    }

    pub fn write_u16(&mut self, segment: u16, offset: u16, value: u16) {
        let [lo, hi] = value.to_le_bytes();
        self.write_u8(segment, offset, lo);
        self.write_u8(segment, offset.wrapping_add(1), hi);
    }

    pub fn read_slice(
        &self,
        segment: u16,
        offset: u16,
        destination: &mut [u8],
    ) -> Result<(), MemoryError> {
        if destination.len() > MEMORY_SIZE {
            return Err(MemoryError::SliceTooLarge {
                requested: destination.len(),
            });
        }

        let start = Self::physical_address(segment, offset);
        for (index, byte) in destination.iter_mut().enumerate() {
            *byte = self.read_bus_byte(Self::wrapped_address(start + index));
        }
        Ok(())
    }

    pub fn write_slice(
        &mut self,
        segment: u16,
        offset: u16,
        source: &[u8],
    ) -> Result<(), MemoryError> {
        if source.len() > MEMORY_SIZE {
            return Err(MemoryError::SliceTooLarge {
                requested: source.len(),
            });
        }

        let start = Self::physical_address(segment, offset);
        for (index, &byte) in source.iter().enumerate() {
            self.write_bus_byte(Self::wrapped_address(start + index), byte);
        }
        Ok(())
    }

    pub fn take_video_dirty(&mut self) -> [bool; TEXT_ROWS] {
        let dirty = self.video_dirty_rows;
        self.video_dirty_rows = [false; TEXT_ROWS];
        dirty
    }

    pub fn take_graphics_dirty(&mut self) -> [bool; VGA_HEIGHT] {
        let dirty = self.graphics_dirty_rows;
        self.graphics_dirty_rows = [false; VGA_HEIGHT];
        dirty
    }

    pub fn graphics_dirty_rows(&self) -> &[bool; VGA_HEIGHT] {
        &self.graphics_dirty_rows
    }

    pub const fn framebuffer_generation(&self) -> u64 {
        self.framebuffer_generation
    }

    /// Mode 13h policy: selecting the mode clears all 64,000 visible indexed
    /// bytes and requests one complete conversion, even when memory was already
    /// black. The generation advances only for bytes whose value changed.
    pub fn clear_graphics_framebuffer(&mut self) {
        if !self.vga_addressing_enabled() {
            return;
        }
        for offset in 0..VGA_FRAMEBUFFER_BYTES {
            self.write_u8(VGA_FRAMEBUFFER_SEGMENT, offset as u16, 0);
        }
        self.graphics_dirty_rows = [true; VGA_HEIGHT];
    }

    fn mark_video_dirty(&mut self, address: usize) {
        if (VIDEO_PHYSICAL..VIDEO_PHYSICAL + VIDEO_BYTES).contains(&address) {
            self.video_dirty_rows[(address - VIDEO_PHYSICAL) / (80 * 2)] = true;
        }
        if (VGA_FRAMEBUFFER_PHYSICAL..VGA_FRAMEBUFFER_PHYSICAL + VGA_FRAMEBUFFER_BYTES)
            .contains(&address)
        {
            self.graphics_dirty_rows[(address - VGA_FRAMEBUFFER_PHYSICAL) / VGA_WIDTH] = true;
            self.framebuffer_generation = self.framebuffer_generation.wrapping_add(1);
        }
    }
}

impl Default for GuestMemory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{GuestMemory, MEMORY_SIZE};
    use crate::{VGA_FRAMEBUFFER_BYTES, VGA_FRAMEBUFFER_SEGMENT, VGA_HEIGHT};

    #[test]
    fn segment_address_uses_twenty_bit_wrapping() {
        assert_eq!(GuestMemory::physical_address(0x1000, 0x0100), 0x10100);
        assert_eq!(GuestMemory::physical_address(0xffff, 0x0010), 0);
    }

    #[test]
    fn words_wrap_at_the_end_of_a_segment_and_memory() {
        let mut memory = GuestMemory::new();
        memory.write_u16(0xffff, 0x000f, 0xabcd);

        assert_eq!(memory.read_u8(0xffff, 0x000f), 0xcd);
        assert_eq!(memory.read_u8(0, 0), 0xab);
        assert_eq!(memory.read_u16(0xffff, 0x000f), 0xabcd);
    }

    #[test]
    fn slices_are_bounded_and_wrap_safely() {
        let mut memory = GuestMemory::new();
        memory.write_slice(0xffff, 0x000f, &[1, 2, 3]).unwrap();
        let mut output = [0; 3];
        memory.read_slice(0xffff, 0x000f, &mut output).unwrap();
        assert_eq!(output, [1, 2, 3]);

        let oversized = vec![0; MEMORY_SIZE + 1];
        assert!(memory.write_slice(0, 0, &oversized).is_err());
    }

    #[test]
    fn graphics_damage_is_clamped_to_the_visible_a0000_range() {
        let mut memory = GuestMemory::new();
        assert!(!memory.take_graphics_dirty().iter().any(|dirty| *dirty));

        memory.write_u8(0x9fff, 0x000f, 1);
        memory.write_u8(VGA_FRAMEBUFFER_SEGMENT, 0, 2);
        memory.write_u8(
            VGA_FRAMEBUFFER_SEGMENT,
            (VGA_FRAMEBUFFER_BYTES - 1) as u16,
            3,
        );
        memory.write_u8(VGA_FRAMEBUFFER_SEGMENT, VGA_FRAMEBUFFER_BYTES as u16, 4);

        let dirty = memory.take_graphics_dirty();
        assert!(dirty[0]);
        assert!(dirty[VGA_HEIGHT - 1]);
        assert_eq!(dirty.iter().filter(|dirty| **dirty).count(), 2);
        assert_eq!(memory.framebuffer_generation(), 2);
    }

    #[test]
    fn word_and_slice_damage_cover_crossed_rows_and_visible_end_safely() {
        let mut memory = GuestMemory::new();
        memory.write_u16(VGA_FRAMEBUFFER_SEGMENT, 319, 0x0201);
        let dirty = memory.take_graphics_dirty();
        assert!(dirty[0] && dirty[1]);

        memory
            .write_slice(VGA_FRAMEBUFFER_SEGMENT, 638, &[3, 4, 5, 6])
            .unwrap();
        let dirty = memory.take_graphics_dirty();
        assert!(dirty[1] && dirty[2]);

        let generation = memory.framebuffer_generation();
        memory.write_u16(
            VGA_FRAMEBUFFER_SEGMENT,
            (VGA_FRAMEBUFFER_BYTES - 1) as u16,
            0x0807,
        );
        let dirty = memory.take_graphics_dirty();
        assert!(dirty[VGA_HEIGHT - 1]);
        assert_eq!(dirty.iter().filter(|dirty| **dirty).count(), 1);
        assert_eq!(memory.framebuffer_generation(), generation + 1);
        assert_eq!(
            memory.read_u8(VGA_FRAMEBUFFER_SEGMENT, VGA_FRAMEBUFFER_BYTES as u16),
            8
        );
    }

    #[test]
    fn unchanged_pixels_do_not_advance_generation_or_recreate_damage() {
        let mut memory = GuestMemory::new();
        memory.write_u8(VGA_FRAMEBUFFER_SEGMENT, 1234, 9);
        let generation = memory.framebuffer_generation();
        assert!(memory.take_graphics_dirty()[3]);

        memory.write_u8(VGA_FRAMEBUFFER_SEGMENT, 1234, 9);
        assert_eq!(memory.framebuffer_generation(), generation);
        assert!(!memory.take_graphics_dirty().iter().any(|dirty| *dirty));
    }

    #[test]
    fn mode_clear_marks_all_rows_without_allocating_per_pixel_damage() {
        let mut memory = GuestMemory::new();
        memory.write_u8(VGA_FRAMEBUFFER_SEGMENT, 0, 7);
        memory.take_graphics_dirty();
        memory.clear_graphics_framebuffer();

        assert_eq!(memory.read_u8(VGA_FRAMEBUFFER_SEGMENT, 0), 0);
        assert!(memory.take_graphics_dirty().iter().all(|dirty| *dirty));
    }
}
