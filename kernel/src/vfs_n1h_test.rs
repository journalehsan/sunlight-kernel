//! Isolated ordinary settlement probe. The device never enters the disk registry.
#[cfg(not(feature = "vfs-n1i-test"))]
use sunlight_block::contract::Durability;
use sunlight_block::{
    contract::{Amount, Completion, IoOutcome, Operation, Ownership, RequestId, StatusSource},
    quiesce::{DrainOutcome, SlotState},
};
use sunlight_virtio::VirtioBlk;

pub(crate) fn verify(
    device: &VirtioBlk,
    previous: &mut Option<RequestId>,
    receipt: IoOutcome,
    operation: Operation,
    lba: u64,
) -> RequestId {
    let id = receipt.request.expect("N1H request identity");
    assert_eq!(id.device, device.geometry().identity);
    if let Some(last) = *previous {
        assert_eq!(id.device, last.device);
        assert_eq!(id.queue_incarnation, last.queue_incarnation);
        assert_eq!(id.generation, last.generation + 1);
    }
    assert_eq!(receipt.device, id.device);
    assert_eq!(receipt.operation, operation);
    assert_eq!(receipt.lba, lba);
    assert_eq!(receipt.completion, Completion::Completed);
    assert_eq!(receipt.source, StatusSource::DeviceStatus(0));
    if operation != Operation::Flush {
        assert_eq!(receipt.completed, Amount::Exact(512));
    }
    assert_eq!(device.ownership(), Ownership::Retired);
    assert_eq!(device.outstanding(), None);
    assert!(matches!(
        device.callback_state(),
        Some(SlotState::Terminal(_))
    ));
    assert_eq!(device.mapping_users(), Some(0));
    assert_eq!(device.mapping_retired(), Some(true));
    assert_eq!(device.drain(1), DrainOutcome::Complete);
    *previous = Some(id);
    id
}

#[cfg(not(feature = "vfs-n1i-test"))]
pub fn run(mut device: VirtioBlk) {
    assert_eq!(device.block_count(), 8192);
    assert_eq!(device.geometry().logical_block, 512);
    assert!(device.supports_write() && device.supports_flush());
    let mut previous = None;
    let mut readback = [0; 512];
    let read = unsafe { device.read_typed(0, &mut readback) };
    verify(&device, &mut previous, read, Operation::Read, 0);
    assert!(readback.starts_with(b"VFS-N1H-DISPOSABLE-ONLY!"));
    crate::serial_println!("[VFS-N1H] ready geometry={:?}", device.geometry());

    for cycle in 0..64u8 {
        let lba = 128 + u64::from(cycle);
        let payload = core::array::from_fn(|index| cycle ^ index as u8);
        let write_id = unsafe { device.start_write(lba, &payload) }.expect("N1H submit write");
        assert_eq!(device.outstanding(), Some(write_id));
        assert_eq!(device.ownership(), Ownership::Submitted);
        assert_eq!(device.mapping_users(), Some(1));
        assert_eq!(device.drain(0), DrainOutcome::Bounded { pending: 1 });
        let write = (0..1_000_000)
            .find_map(|_| unsafe { device.poll_write(write_id) })
            .expect("N1H write did not settle; DMA remains pinned");
        assert_eq!(
            verify(&device, &mut previous, write, Operation::Write, lba),
            write_id
        );
        assert_eq!(write.durability, Durability::NotEstablished);

        let flush = unsafe { device.flush_typed() };
        verify(&device, &mut previous, flush, Operation::Flush, 0);
        assert_eq!(
            flush.durability,
            Durability::VirtualBarrier {
                through_generation: write_id.generation,
            }
        );
        let read = unsafe { device.read_typed(lba, &mut readback) };
        verify(&device, &mut previous, read, Operation::Read, lba);
        assert_eq!(payload, readback);
    }
    assert!(!device.has_uncertain_io());
    crate::serial_println!(
        "[VFS-N1H] PASS cycles=64 completed=193 pending=0 mapping_users=0 barriers=64"
    );
}
