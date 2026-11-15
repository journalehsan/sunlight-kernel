//! Off-by-default typed contract qualification on the SFS disposable image.
use sunlight_block::contract::*;
use sunlight_virtio::VirtioBlk;

pub fn run(d: &mut VirtioBlk) {
    let g = d.geometry();
    g.validate().unwrap();
    assert_eq!(g.blocks, 8192, "SFS5B1 disposable capacity guard");
    let mut marker = [0; 512];
    assert!(unsafe { d.read_typed(0, &mut marker) }.legacy().is_ok());
    assert_eq!(&marker[..34], b"SFS5A-ONLY-7b93143a-disposable-v1!");
    assert_eq!(g.physical_block, Knowledge::Unknown);
    assert_eq!(g.fencing, FenceCapability::Unsupported);
    crate::serial_println!("[SFS5B1] geometry={:?}", g);
    let write = unsafe { d.write_typed(60, &[0xb1; 512]) };
    assert_eq!(write.completion, Completion::Completed);
    assert_eq!(write.durability, Durability::NotEstablished);
    assert_eq!(write.completed, Amount::Exact(512));
    let flush = unsafe { d.flush_typed() };
    assert_eq!(flush.completion, Completion::Completed);
    assert_eq!(
        flush.durability,
        Durability::VirtualBarrier {
            through_generation: write.request.unwrap().generation,
        }
    );
    assert_eq!(flush.source, StatusSource::DeviceStatus(0));
    let mut output = [0; 512];
    let read = unsafe { d.read_typed(60, &mut output) };
    assert_eq!(read.completion, Completion::Completed);
    assert_eq!(output, [0xb1; 512]);
    assert_eq!(d.ownership(), Ownership::Retired);
    assert_eq!(d.outstanding(), None);
    let mut untouched = [0x99; 512];
    let rejected = unsafe { d.read_typed(u64::MAX, &mut untouched) };
    assert_eq!(
        rejected.completion,
        Completion::Rejected(Rejection::Overflow)
    );
    assert_eq!(rejected.request, None);
    assert_eq!(untouched, [0x99; 512]);
    assert!(!d.has_uncertain_io());
    crate::serial_println!("[SFS5B1] write={:?}", write);
    crate::serial_println!("[SFS5B1] flush={:?}", flush);
    crate::serial_println!(
        "[SFS5B1] read={:?} used_len={:?}",
        read,
        d.last_used_length()
    );
    crate::serial_println!("[SFS5B1] rejected={:?}", rejected);
    crate::serial_println!("[SFS5B1] CONTRACT PASS execution=BSP-before-AP-startup");
    // Repeat the unchanged SFS-5A barrier/crash-cut fixture on the final driver.
    crate::sfs5a_test::run(d);
}
