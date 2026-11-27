//! Serialized lease adapter and qualification on the N1H disposable disk.
use crate::{device_lease::*, vfs_n1h_test::verify};
use sunlight_block::{
    contract::{Durability, IoOutcome, Operation as WireOp, RequestId},
    quiesce::DrainOutcome,
};
#[cfg(not(feature = "vfs-n1j-test"))]
use sunlight_ipc::vfs_rpc::registration::{EndpointOwner, Registry};
#[cfg(not(feature = "vfs-n1j-test"))]
use sunlight_virtio::ResetState;
use sunlight_virtio::VirtioBlk;

pub(crate) struct Probe {
    pub(crate) device: VirtioBlk,
    pub(crate) authority: DeviceLease,
    pub(crate) previous: Option<RequestId>,
    pub(crate) data: usize,
    pub(crate) barriers: usize,
}

impl Probe {
    pub(crate) fn fence_transport(&self) {
        let lease = self.authority.snapshot().lease.unwrap();
        self.authority.transport_lost(&lease).unwrap();
    }

    pub(crate) fn confirm(&mut self, receipt: IoOutcome, operation: WireOp, lba: u64) {
        if receipt.legacy().is_err() || self.device.has_uncertain_io() {
            self.fence_transport();
            panic!("VFS: transport did not confirm settlement; medium remains fenced");
        }
        verify(&self.device, &mut self.previous, receipt, operation, lba);
    }

    pub(crate) fn execute(&mut self, ticket: Operation) -> Block {
        let (lba, payload) = self.authority.command(&ticket).unwrap();
        let mut bytes = [0; 512];
        let (receipt, operation) = if let Some(payload) = payload {
            let id = unsafe { self.device.start_write(lba, &payload) }.unwrap();
            assert_eq!(self.device.outstanding(), Some(id));
            assert_eq!(self.device.mapping_users(), Some(1));
            assert_eq!(self.device.drain(0), DrainOutcome::Bounded { pending: 1 });
            let snapshot = self.authority.snapshot();
            assert_eq!((snapshot.outstanding, snapshot.retained_data), (1, 1));
            let Some(receipt) = (0..1_000_000).find_map(|_| unsafe { self.device.poll_write(id) })
            else {
                self.fence_transport();
                panic!("write did not settle; DMA remains pinned");
            };
            assert_eq!(receipt.request, Some(id));
            assert_eq!(receipt.durability, Durability::NotEstablished);
            (receipt, WireOp::Write)
        } else {
            (
                unsafe { self.device.read_typed(lba, &mut bytes) },
                WireOp::Read,
            )
        };
        self.confirm(receipt, operation, lba);
        let data = (operation == WireOp::Read).then_some(bytes);
        self.authority.complete_live(&ticket, data).unwrap();
        assert_eq!(
            self.authority.complete_live(&ticket, data),
            Err(Error::Stale)
        );
        let snapshot = self.authority.snapshot();
        assert_eq!((snapshot.outstanding, snapshot.retained_data), (0, 0));
        self.data += 1;
        bytes
    }

    pub(crate) fn barrier(&mut self, transition: &Transition) {
        let ticket = self.authority.start_flush(transition).unwrap();
        assert!(self.authority.snapshot().flush_pending);
        assert_eq!(self.device.drain(1), DrainOutcome::Complete);
        assert_eq!(self.device.outstanding(), None);
        let through_generation = self.previous.unwrap().generation;
        let receipt = unsafe { self.device.flush_typed() };
        self.confirm(receipt, WireOp::Flush, 0);
        assert_eq!(
            receipt.durability,
            Durability::VirtualBarrier { through_generation }
        );
        self.authority
            .complete_flush(&ticket, Outcome::Success)
            .unwrap();
        assert_eq!(
            self.authority.complete_flush(&ticket, Outcome::Success),
            Err(Error::Stale)
        );
        assert!(!self.authority.snapshot().flush_pending);
        self.barriers += 1;
    }
}

pub(crate) fn sector(lba: u64) -> Extent {
    Extent {
        offset: lba * 512,
        length: 512,
    }
}

pub(crate) fn initialize(mut device: VirtioBlk) -> Probe {
    let (blocks, marker_prefix): (u64, &[u8]) = if cfg!(feature = "vfs-n1o-test") {
        (131072, b"VFS-N1O-DISPOSABLE-ONLY!")
    } else {
        (8192, b"VFS-N1H-DISPOSABLE-ONLY!")
    };
    assert_eq!(device.block_count(), blocks);
    assert_eq!(device.geometry().logical_block, 512);
    assert!(device.supports_write() && device.supports_flush());
    let mut marker = [0; 512];
    let receipt = unsafe { device.read_typed(0, &mut marker) };
    let mut previous = None;
    verify(&device, &mut previous, receipt, WireOp::Read, 0);
    assert!(marker.starts_with(marker_prefix));

    Probe {
        device,
        authority: DeviceLease::new(),
        previous,
        data: 0,
        barriers: 0,
    }
}

#[cfg(not(feature = "vfs-n1j-test"))]
pub fn run(device: VirtioBlk) {
    let mut probe = initialize(device);
    let media = probe
        .authority
        .register_media(Geometry {
            blocks: 8192,
            block_bytes: 512,
        })
        .unwrap();
    let bind = |registry: &mut Registry, endpoint| {
        let registration = registry
            .register(EndpointOwner {
                endpoint,
                pid: endpoint,
                incarnation: 1,
                address_space_generation: 1,
            })
            .unwrap();
        (
            probe.authority.bind_registration(registration).unwrap(),
            registration,
        )
    };
    let mut writer_registry = Registry::new();
    let mut reader_registry = Registry::new();
    let (writer, writer_registration) = bind(&mut writer_registry, 1);
    let (reader, reader_registration) = bind(&mut reader_registry, 2);
    let whole = Extent {
        offset: 0,
        length: 8192 * 512,
    };
    let rw = Rights {
        read: true,
        write: true,
        flush: true,
    };
    let ro = Rights {
        read: true,
        write: false,
        flush: false,
    };
    let mut last_lease_generation = 0;
    for cycle in 0..64u8 {
        let extent = sector(128 + u64::from(cycle));
        let lease = probe
            .authority
            .acquire_authenticated(&media, &writer, whole, rw)
            .unwrap();
        assert_eq!(lease.owner(), writer_registration);
        assert!(lease.generation() > last_lease_generation);
        assert_eq!(
            probe
                .authority
                .acquire_authenticated(&media, &reader, extent, ro),
            Err(Error::Occupied)
        );
        assert_eq!(
            probe
                .authority
                .read_authenticated(&reader, &lease, extent)
                .unwrap_err(),
            Error::Unauthorized
        );

        let payload = core::array::from_fn(|index| cycle ^ index as u8);
        let write = probe
            .authority
            .write_authenticated(&writer, &lease, extent, payload)
            .unwrap();
        assert_eq!(
            probe
                .authority
                .read_authenticated(&writer, &lease, extent)
                .unwrap_err(),
            Error::NoCredit
        );
        let flush = probe.authority.request_flush(&writer, &lease).unwrap();
        assert_eq!(
            probe.authority.start_flush(&flush).unwrap_err(),
            Error::Busy
        );
        assert_eq!(
            probe
                .authority
                .read_authenticated(&writer, &lease, extent)
                .unwrap_err(),
            Error::AdmissionClosed
        );
        probe.execute(write);
        probe.barrier(&flush);

        let transfer = probe
            .authority
            .begin_authenticated_transfer(&writer, &lease, &reader, extent, ro)
            .unwrap();
        assert_eq!(probe.authority.finish(&transfer), Err(Error::Busy));
        probe.barrier(&transfer);
        let successor = probe.authority.finish(&transfer).unwrap().unwrap();
        assert_eq!(successor.owner(), reader_registration);
        assert!(successor.generation() > lease.generation());
        last_lease_generation = successor.generation();
        assert_eq!(
            probe
                .authority
                .read_authenticated(&writer, &lease, extent)
                .unwrap_err(),
            Error::Stale
        );
        assert_eq!(
            probe
                .authority
                .read_authenticated(&reader, &successor, sector(0))
                .unwrap_err(),
            Error::OutOfRange
        );
        assert_eq!(
            probe
                .authority
                .write_authenticated(&reader, &successor, extent, payload)
                .unwrap_err(),
            Error::Unauthorized
        );
        assert_eq!(
            probe
                .authority
                .request_flush(&reader, &successor)
                .unwrap_err(),
            Error::Unauthorized
        );
        let read = probe
            .authority
            .read_authenticated(&reader, &successor, extent)
            .unwrap();
        assert_eq!(probe.execute(read), payload);

        let release = probe.authority.begin_release(&successor).unwrap();
        probe.barrier(&release);
        assert_eq!(probe.authority.finish(&release), Ok(None));
        assert_eq!(probe.authority.snapshot().status, Status::Vacant);
    }
    probe.authority.withdraw_registration(&writer).unwrap();
    probe.authority.withdraw_registration(&reader).unwrap();
    assert!(writer_registry.retire(writer_registration));
    assert!(reader_registry.retire(reader_registration));
    probe.authority.remove_media(&media).unwrap();
    assert!(!probe.device.has_uncertain_io());
    let mut reset = unsafe { probe.device.begin_terminal_reset() };
    while reset.state == ResetState::Waiting {
        reset = unsafe { probe.device.poll_terminal_reset() };
    }
    assert_eq!(reset.state, ResetState::VerifiedRetained);
    assert_eq!(reset.outstanding, None);
    assert_eq!(
        reset.queue_incarnation,
        probe.previous.unwrap().queue_incarnation
    );
    assert_eq!(reset.device, probe.previous.unwrap().device);
    assert_eq!(probe.device.mapping_users(), Some(0));
    assert_eq!((probe.data, probe.barriers), (128, 192));
    crate::serial_println!("[VFS-N1I] PASS cycles=64 data=128 barriers=192 pending=0 mapping_users=0 reset=VerifiedRetained");
}
