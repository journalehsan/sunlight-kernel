//! Guest BIOS font ROM, independent of host Unicode rendering.
//! Asset provenance and CC0/public-domain notices: ../fonts/README.md.
use crate::{Runtime, Trap};

pub(crate) const ROM_SEGMENT: u16 = 0xc000;
pub(crate) const ROM_START: usize = 0xc0000;
pub(crate) const ROM_END: usize = ROM_START + 2048 + 4096;
pub(crate) const FONT_8: &[u8; 2048] = include_bytes!("../fonts/cp437-8.bin");
pub(crate) const FONT_16: &[u8; 4096] = include_bytes!("../fonts/cp437-16.bin");

pub(crate) fn get_information(runtime: &mut Runtime) -> Result<(), Trap> {
    // RBIL AX=1130: CX describes the on-screen font, not the selected table.
    // Alternate 9-dot fonts are replacement lists, not ordinary glyph arrays;
    // do not pretend a normal 8-dot table implements those selectors.
    let pointer = match runtime.cpu.bh() {
        0 => runtime.interrupt_vector(0x1f),
        1 => runtime.interrupt_vector(0x43),
        3 => (ROM_SEGMENT, 0),
        4 => (ROM_SEGMENT, 0x400),
        6 => (ROM_SEGMENT, 0x800),
        _ => {
            return Err(Trap::UnsupportedInterrupt {
                interrupt: 0x10,
                function: 0x11,
            })
        }
    };
    runtime.cpu.es = pointer.0;
    runtime.cpu.bp = pointer.1;
    runtime.cpu.cx = runtime.video_mode().character_height();
    runtime
        .cpu
        .set_dl(runtime.video_mode().character_rows() - 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GuestMemory, GuestState, GuestVideoMode, DOS_ARENA_END_SEGMENT};

    fn query(runtime: &mut Runtime, selector: u8) {
        runtime.cpu.ax = 0x1130;
        runtime.cpu.set_bh(selector);
        runtime.cpu.ip = 0x100;
        runtime.step();
        assert_eq!(runtime.state(), &GuestState::Running);
    }

    #[test]
    fn guest_int_queries_all_supported_selectors_and_preserves_registers() {
        for (selector, offset, bytes) in [
            (0, 0x400, &FONT_8[1024..]),
            (1, 0x800, &FONT_16[..]),
            (3, 0, &FONT_8[..]),
            (4, 0x400, &FONT_8[1024..]),
            (6, 0x800, &FONT_16[..]),
        ] {
            let mut r = Runtime::from_com(&[0xcd, 0x10]).unwrap();
            for index in 0..8 {
                r.cpu.set_reg32(index, 0x12345678 + index as u32);
            }
            r.cpu.ax = 0x1130;
            r.cpu.bx = (selector as u16) << 8 | 0x55;
            r.cpu.flags = 0xf6d7;
            let mut expected = r.cpu;
            expected.es = ROM_SEGMENT;
            expected.bp = offset;
            expected.cx = 16;
            expected.set_dl(24);
            expected.ip += 2;
            r.step();
            assert_eq!(r.cpu, expected);
            let mut actual = alloc::vec![0; bytes.len()];
            r.memory
                .read_slice(r.cpu.es, r.cpu.bp, &mut actual)
                .unwrap();
            assert_eq!(actual, bytes);
            assert!(bytes.iter().any(|b| *b != 0));
        }
    }

    #[test]
    fn mode_changes_synchronize_bda_and_graphics_vector_repeatedly() {
        let mut r = Runtime::from_program(&[0xcd, 0x10], &[]).unwrap();
        for mode in [
            GuestVideoMode::Vga320x200x256,
            GuestVideoMode::Text80x25Color,
        ] {
            r.cpu.ax = mode.bios_mode() as u16;
            r.cpu.ip = 0x100;
            r.step(); // real INT 10 mode selection
            for _ in 0..3 {
                query(&mut r, 1);
                assert_eq!(r.cpu.cx, mode.character_height());
                assert_eq!(r.cpu.dl(), mode.character_rows() - 1);
                assert_eq!(r.memory.read_u16(0x40, 0x85), r.cpu.cx);
                assert_eq!(r.memory.read_u8(0x40, 0x84), r.cpu.dl());
                assert_eq!((r.cpu.es, r.cpu.bp), r.interrupt_vector(0x43));
                assert_eq!(
                    r.cpu.bp,
                    if mode == GuestVideoMode::Text80x25Color {
                        0x800
                    } else {
                        0
                    }
                );
                query(&mut r, 6); // requested 16-row font, active may be 8
                assert_eq!(r.cpu.cx, mode.character_height());
            }
        }
    }

    #[test]
    fn ivt_selectors_follow_guest_relocated_vectors() {
        let mut r = Runtime::from_com(&[0xcd, 0x10]).unwrap();
        for (selector, vector) in [(0, 0x1f), (1, 0x43)] {
            r.memory.write_u16(0, vector * 4, 0x1234);
            r.memory.write_u16(0, vector * 4 + 2, 0x5678);
            query(&mut r, selector);
            assert_eq!((r.cpu.es, r.cpu.bp), (0x5678, 0x1234));
        }
    }

    #[test]
    fn guest_reads_bitmap_and_cannot_write_rom_through_aliases() {
        // INT 10; MOV AL,ES:[BP+1]; MOV ES:[BP+1],0; REP STOSB
        let mut r = Runtime::from_com(&[
            0xcd, 0x10, 0x26, 0x8a, 0x46, 1, 0x26, 0xc6, 0x46, 1, 0, 0xf3, 0xaa,
        ])
        .unwrap();
        query(&mut r, 4);
        let expected = FONT_8[1025];
        r.step();
        assert_eq!(r.cpu.al(), expected);
        r.step();
        assert_eq!(r.memory.read_u8(ROM_SEGMENT, 0x401), expected);
        r.cpu.di = 0x400;
        r.cpu.cx = 1024;
        r.step();
        let mut actual = [0; 1024];
        r.memory
            .read_slice(ROM_SEGMENT, 0x400, &mut actual)
            .unwrap();
        assert_eq!(&actual, &FONT_8[1024..]);
        r.memory.write_slice(0xbfff, 0x10, &[0xff; 2048]).unwrap();
        r.memory.write_u16(ROM_SEGMENT, 0x800, 0xffff);
        assert_eq!(r.memory.read_u8(ROM_SEGMENT, 0), FONT_8[0]);
        assert_eq!(
            r.memory.read_u16(ROM_SEGMENT, 0x800),
            u16::from_le_bytes([FONT_16[0], FONT_16[1]])
        );
    }

    #[test]
    fn rom_boundaries_do_not_reserve_adjacent_memory_or_dos_arena() {
        let mut r = Runtime::from_com(&[0xcd, 0x10]).unwrap();
        assert!((DOS_ARENA_END_SEGMENT as usize) << 4 <= ROM_START);
        for address in [ROM_START - 1, ROM_END] {
            let seg = (address >> 4) as u16;
            let off = (address & 15) as u16;
            r.memory.write_u8(seg, off, 0xa5);
            assert_eq!(r.memory.read_u8(seg, off), 0xa5);
        }
        assert_eq!(
            GuestMemory::physical_address(ROM_SEGMENT, 0x17ff),
            ROM_END - 1
        );
        assert_eq!(r.memory.read_u8(ROM_SEGMENT, 0x17ff), FONT_16[4095]);
        let psp = r.current_psp();
        assert_eq!(r.memory.read_u16(psp, 0), 0x20cd);
    }

    #[test]
    fn unavailable_font_selectors_trap_without_fabricating_pointers() {
        for selector in [2, 5, 7, 8, 0xff] {
            let mut r = Runtime::from_com(&[0xcd, 0x10]).unwrap();
            r.cpu.ax = 0x1130;
            r.cpu.set_bh(selector);
            let before = r.cpu;
            r.step();
            assert!(matches!(
                r.state(),
                GuestState::Trapped(Trap::UnsupportedInterrupt {
                    interrupt: 0x10,
                    function: 0x11
                })
            ));
            assert_eq!(
                (r.cpu.es, r.cpu.bp, r.cpu.cx, r.cpu.dx),
                (before.es, before.bp, before.cx, before.dx)
            );
        }
    }
}
