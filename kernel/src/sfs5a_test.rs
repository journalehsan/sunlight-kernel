//! Opt-in ring-0 qualification on an explicitly marked disposable disk only.
use sunlight_virtio::VirtioBlk;

pub fn run(d: &mut VirtioBlk) {
    let mut marker = [0; 512];
    assert_eq!(d.block_count(), 8192, "SFS5A capacity guard");
    assert!(unsafe { d.read_block(0, &mut marker) });
    assert_eq!(&marker[..34], b"SFS5A-ONLY-7b93143a-disposable-v1!");
    crate::serial_println!(
        "[SFS5A] identity=7b93143a sectors=8192 sector=512 flush={} write={}",
        d.supports_flush(),
        d.supports_write()
    );
    crate::serial_println!("[SFS5A] B1 PASS");

    let mut state = [0; 512];
    assert!(unsafe { d.read_block(1, &mut state) });
    if state == [0x5a; 512] {
        // Cold guest restart observes the complete acknowledged durable cut.
        verify(d, 4, 0x42);
        for lba in 8..16 {
            verify(d, lba, 0xd1);
        }
        verify(d, 16, 0xc1);
        verify(d, 17, 0);
        verify(d, 18, 0);
        for i in 0..16 {
            verify(d, 32 + i, i as u8);
        }
        crate::serial_println!("[SFS5A] B13 RESTART PASS");
    } else {
        assert_eq!(state, [0; 512], "unexpected media state");
        assert!(d.supports_flush(), "unqualified no-flush test device");
        put(d, 4, 0x42);
        verify(d, 4, 0x42);
        crate::serial_println!("[SFS5A] B2 PASS");
        for lba in 8..16 {
            put(d, lba, 0xd1);
            verify(d, lba, 0xd1);
        }
        crate::serial_println!("[SFS5A] B3 PASS sequential-eight-sectors");
        crate::serial_println!("[SFS5A] writes-completed durability-not-yet-acknowledged");
        assert!(unsafe { d.flush() });
        crate::serial_println!("[SFS5A] B4 D-flush-ack PASS");
        put(d, 16, 0xc1);
        assert!(unsafe { d.flush() });
        crate::serial_println!("[SFS5A] P-flush-ack");
        put(d, 17, 0);
        assert!(unsafe { d.flush() });
        crate::serial_println!("[SFS5A] R-flush-ack");
        put(d, 18, 0);
        assert!(unsafe { d.flush() });
        crate::serial_println!("[SFS5A] R0-flush-ack");
        for i in 0..16 {
            put(d, 32 + i, i as u8);
            assert!(unsafe { d.flush() });
            verify(d, 32 + i, i as u8);
        }
        crate::serial_println!("[SFS5A] B12 PASS cycles=16");
        let mut untouched = [0x99; 512];
        assert!(!unsafe { d.read_block(8192, &mut untouched) });
        assert_eq!(untouched, [0x99; 512]);
        assert!(!unsafe { d.write_block(8192, &[0x99; 512]) });
        assert!(!d.has_uncertain_io());
        crate::serial_println!("[SFS5A] B6 bounds-only PASS");
        put(d, 1, 0x5a);
        assert!(unsafe { d.flush() });
        crate::serial_println!("[SFS5A] B14 serialized-barriers PASS");
    }
    crate::serial_println!("[SFS5A] READY-FOR-HOST-KILL");
    loop {
        x86_64::instructions::hlt();
    }
}

fn put(d: &mut VirtioBlk, lba: u64, value: u8) {
    assert!(unsafe { d.write_block(lba, &[value; 512]) });
}
fn verify(d: &mut VirtioBlk, lba: u64, value: u8) {
    let mut bytes = [0; 512];
    assert!(unsafe { d.read_block(lba, &mut bytes) });
    assert_eq!(bytes, [value; 512]);
}
