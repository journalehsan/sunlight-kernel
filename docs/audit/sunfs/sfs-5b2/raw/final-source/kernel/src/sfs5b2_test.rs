//! Disposable-device terminal reset observation; no writable filesystem admission.
use sunlight_block::contract::{Completion, Durability, Ownership};
use sunlight_virtio::{ResetState, VirtioBlk, RESET_READ_BUDGET};

pub fn run(d: &mut VirtioBlk) {
    assert_eq!(d.block_count(), 8192, "SFS5B2 disposable capacity guard");
    let mut marker = [0; 512];
    assert_eq!(
        unsafe { d.read_typed(0, &mut marker) }.completion,
        Completion::Completed
    );
    assert_eq!(&marker[..34], b"SFS5A-ONLY-7b93143a-disposable-v1!");
    crate::serial_println!(
        "[SFS5B2] features={:?} geometry={:?}",
        d.features(),
        d.geometry()
    );
    let write = unsafe { d.write_typed(61, &[0xb2; 512]) };
    assert_eq!(write.completion, Completion::Completed);
    assert_eq!(write.durability, Durability::NotEstablished);
    let flush = unsafe { d.flush_typed() };
    assert_eq!(flush.completion, Completion::Completed);
    let mut data = [0; 512];
    assert_eq!(
        unsafe { d.read_typed(61, &mut data) }.completion,
        Completion::Completed
    );
    assert_eq!(data, [0xb2; 512]);
    assert_eq!(d.ownership(), Ownership::Retired);
    crate::serial_println!("[SFS5B2] write={:?} flush={:?}", write, flush);
    crate::serial_println!("[SFS5B2] reset-before={:?}", d.reset_observation());
    crate::serial_println!("[SFS5B2] reset-begin={:?}", unsafe {
        d.begin_terminal_reset()
    });
    for _ in 0..RESET_READ_BUDGET {
        if unsafe { d.poll_terminal_reset() }.state != ResetState::Waiting {
            break;
        }
    }
    let done = d.reset_observation();
    crate::serial_println!("[SFS5B2] reset-after={:?}", done);
    assert_eq!(done.state, ResetState::VerifiedRetained);
    assert_eq!(unsafe { d.begin_terminal_reset() }, done);
    let refused = unsafe { d.write_typed(62, &[0x99; 512]) };
    assert_eq!(refused.completion, Completion::Unavailable);
    assert_eq!(d.reset_observation(), done);
    assert_eq!(d.outstanding(), None);
    crate::serial_println!("[SFS5B2] RESET-OBSERVED pages-pinned=5 released=0 active-dma-at-reset=false execution=BSP-before-AP-startup");
    crate::serial_println!("[SFS5B2] READY-FOR-HOST-OBSERVER");
    loop {
        x86_64::instructions::hlt();
    }
}
