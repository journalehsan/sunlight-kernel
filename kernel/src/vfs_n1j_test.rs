//! One disposable queue, driven by the existing authenticated VFS lifecycle.
use crate::{
    capability::delivery::RecipientIdentity,
    device_lease::*,
    vfs_n1i_test::{initialize, sector, Probe},
};
use spin::Mutex;
use sunlight_block::{contract::Operation as WireOp, quiesce::DrainOutcome};
use sunlight_ipc::vfs_rpc::{lifecycle::Caller, registration::Registration};
use sunlight_virtio::{ResetState, VirtioBlk, QUEUE_PAGES};
use x86_64::PhysAddr;

struct Binding {
    registration: Registration,
    session: Session,
    pending: Option<(Lease, Operation, sunlight_block::contract::RequestId)>,
    retirement: Option<Transition>,
    image_retired: bool,
    #[cfg(feature = "vfs-n1m-test")]
    active: Option<Lease>,
}

struct Qualification {
    probe: Probe,
    media: Media,
    binding: Option<Binding>,
    queue: PhysAddr,
    bounce: PhysAddr,
    done: bool,
}

static TEST: Mutex<Option<Qualification>> = Mutex::new(None);

const WHOLE: Extent = Extent {
    offset: 0,
    length: 8192 * 512,
};
const READ_WRITE: Rights = Rights {
    read: true,
    write: true,
    flush: true,
};

pub(crate) fn install(device: VirtioBlk, queue: PhysAddr, bounce: PhysAddr) {
    assert!(crate::MP_REQ
        .response()
        .is_none_or(|response| response.cpus().len() == 1));
    let probe = initialize(device);
    let media = probe
        .authority
        .register_media(Geometry {
            blocks: 8192,
            block_bytes: 512,
        })
        .unwrap();
    assert!(TEST
        .lock()
        .replace(Qualification {
            probe,
            media,
            binding: None,
            queue,
            bounce,
            done: false,
        })
        .is_none());
}

pub(crate) fn bind(registration: Registration) {
    let mut slot = TEST.lock();
    let Some(test) = slot.as_mut() else { return };
    if test.binding.is_some() || test.done {
        return;
    }
    let session = test
        .probe
        .authority
        .bind_registration(registration)
        .unwrap();
    test.binding = Some(Binding {
        registration,
        session,
        pending: None,
        retirement: None,
        image_retired: false,
        #[cfg(feature = "vfs-n1m-test")]
        active: None,
    });
    crate::serial_println!("[VFS-N1J] authenticated {:?}", registration);
}

#[cfg(not(feature = "vfs-n1m-test"))]
pub(crate) fn run_registered(caller: Caller) {
    let mut slot = TEST.lock();
    let Some(test) = slot.as_mut().filter(|test| !test.done) else {
        return;
    };
    let Some(binding) = test.binding.as_mut() else {
        return;
    };
    let owner = binding.registration.owner;
    if owner.pid != caller.pid
        || owner.incarnation != caller.incarnation
        || owner.address_space_generation != caller.address_space_generation
        || binding.pending.is_some()
        || binding.retirement.is_some()
    {
        return;
    }
    let probe = &mut test.probe;
    let mut generation = 0;
    let mut previous_write = None;
    for cycle in 0..64u8 {
        let lease = probe
            .authority
            .acquire_authenticated(&test.media, &binding.session, WHOLE, READ_WRITE)
            .unwrap();
        assert_eq!(lease.owner(), binding.registration);
        assert!(lease.generation() > generation);
        generation = lease.generation();
        if let Some((range, bytes)) = previous_write.take() {
            let read = probe
                .authority
                .read_authenticated(&binding.session, &lease, range)
                .unwrap();
            assert_eq!(probe.execute(read), bytes);
        }
        let range = sector(128 + u64::from(cycle));
        let bytes = core::array::from_fn(|index| cycle ^ index as u8);
        let write = probe
            .authority
            .write_authenticated(&binding.session, &lease, range, bytes)
            .unwrap();
        probe.execute(write);
        let flush = probe
            .authority
            .request_flush(&binding.session, &lease)
            .unwrap();
        probe.barrier(&flush);
        let release = probe.authority.begin_release(&lease).unwrap();
        assert_eq!(probe.authority.finish(&release), Err(Error::Busy));
        probe.barrier(&release);
        assert_eq!(probe.authority.finish(&release), Ok(None));
        assert_eq!(probe.authority.snapshot().status, Status::Vacant);
        previous_write = Some((range, bytes));
    }

    let lease = probe
        .authority
        .acquire_authenticated(&test.media, &binding.session, WHOLE, READ_WRITE)
        .unwrap();
    assert_eq!(lease.owner(), binding.registration);
    assert!(lease.generation() > generation);
    let (range, bytes) = previous_write.unwrap();
    let read = probe
        .authority
        .read_authenticated(&binding.session, &lease, range)
        .unwrap();
    assert_eq!(probe.execute(read), bytes);
    let write = probe
        .authority
        .write_authenticated(&binding.session, &lease, range, bytes)
        .unwrap();
    let (lba, payload) = probe.authority.command(&write).unwrap();
    let id = unsafe { probe.device.start_write(lba, &payload.unwrap()) }.unwrap();
    assert_eq!(probe.device.outstanding(), Some(id));
    assert_eq!(probe.device.mapping_users(), Some(1));
    binding.pending = Some((lease, write, id));
}

#[cfg(feature = "vfs-n1m-test")]
pub(crate) fn native_block(
    caller: Caller,
    op: u64,
    lba: u64,
    generation: u64,
    bytes: &mut Block,
) -> Result<u64, Error> {
    let mut slot = TEST.lock();
    let test = slot.as_mut().filter(|test| !test.done).ok_or(Error::Unauthorized)?;
    let binding = test.binding.as_mut().ok_or(Error::Unauthorized)?;
    let owner = binding.registration.owner;
    if owner.pid != caller.pid
        || owner.incarnation != caller.incarnation
        || owner.address_space_generation != caller.address_space_generation
        || binding.pending.is_some()
        || binding.retirement.is_some()
    {
        return Err(Error::Unauthorized);
    }
    let probe = &mut test.probe;
    if op == 0x100 {
        if generation != 0 || binding.active.is_some() {
            return Err(Error::Busy);
        }
        let lease = probe.authority.acquire_authenticated(
            &test.media, &binding.session, WHOLE, READ_WRITE,
        )?;
        let generation = lease.generation();
        binding.active = Some(lease);
        return Ok(generation);
    }
    let lease = binding.active.as_ref().filter(|lease| lease.generation() == generation)
        .ok_or(Error::Stale)?;
    match op {
        0x101..=0x102 | 0x105 => {
            if !(128..192).contains(&lba) || (op == 0x105 && lba != 191) {
                return Err(Error::Unauthorized);
            }
            let range = sector(lba);
            if op == 0x101 {
                let ticket = probe.authority.read_authenticated(&binding.session, lease, range)?;
                *bytes = probe.execute(ticket);
            } else {
                let ticket = probe.authority.write_authenticated(&binding.session, lease, range, *bytes)?;
                if op == 0x105 {
                    let (lba, payload) = probe.authority.command(&ticket)?;
                    let id = unsafe { probe.device.start_write(lba, &payload.unwrap()) }
                        .unwrap_or_else(|_| {
                            probe.fence_transport();
                            panic!("VFS-N1M: submission failed; medium remains fenced");
                        });
                    binding.pending = Some((binding.active.take().unwrap(), ticket, id));
                    return Ok(id.generation);
                }
                probe.execute(ticket);
            }
        }
        0x103 => {
            let flush = probe.authority.request_flush(&binding.session, lease)?;
            probe.barrier(&flush);
        }
        0x104 => {
            let release = probe.authority.begin_release(lease)?;
            probe.barrier(&release);
            probe.authority.finish(&release)?;
            binding.active = None;
        }
        _ => return Err(Error::Unauthorized),
    }
    Ok(0)
}

pub(crate) fn withdraw(registration: Registration) {
    let mut slot = TEST.lock();
    let Some(test) = slot.as_mut() else { return };
    let Some(binding) = test
        .binding
        .as_mut()
        .filter(|binding| binding.registration == registration)
    else {
        return;
    };
    let authority = &test.probe.authority;
    authority.withdraw_registration(&binding.session).unwrap();
    #[cfg(feature = "vfs-n1m-test")]
    if let Some(lease) = binding.active.take() {
        binding.retirement = Some(authority.retire(&lease).unwrap());
        return;
    }
    let Some((lease, write, _)) = &binding.pending else {
        test.binding = None;
        return;
    };
    assert_eq!(
        authority
            .read_authenticated(&binding.session, lease, sector(191))
            .unwrap_err(),
        Error::Unauthorized,
    );
    authority.abandon(write).unwrap();
    let snapshot = authority.snapshot();
    assert_eq!((snapshot.outstanding, snapshot.retained_data), (0, 1));
    binding.retirement = Some(authority.retire(lease).unwrap());
    crate::serial_println!("[VFS-N1J] withdrawn credit=0 retained=1");
}

pub(crate) fn image_retired(image: RecipientIdentity) {
    let mut slot = TEST.lock();
    let Some(binding) = slot.as_mut().and_then(|test| test.binding.as_mut()) else {
        return;
    };
    let owner = binding.registration.owner;
    if binding.retirement.is_some()
        && owner.pid == image.pid as u64
        && owner.incarnation == image.incarnation
        && owner.address_space_generation == image.vm_generation
    {
        binding.image_retired = true;
    }
}

pub(crate) fn settle_retired() {
    let mut slot = TEST.lock();
    let Some(test) = slot.as_mut() else { return };
    if !test
        .binding
        .as_ref()
        .is_some_and(|binding| binding.image_retired)
    {
        return;
    }
    let binding = test.binding.as_ref().unwrap();
    #[cfg(feature = "vfs-n1m-test")]
    if binding.pending.is_none() {
        test.probe.barrier(binding.retirement.as_ref().unwrap());
        test.binding = None;
        return;
    }
    let (_, write, id) = binding.pending.as_ref().unwrap();
    let retirement = binding.retirement.as_ref().unwrap();
    let probe = &mut test.probe;
    assert_eq!(
        probe.authority.start_flush(retirement).unwrap_err(),
        Error::Busy
    );
    let receipt = (0..1_000_000).find_map(|_| unsafe { probe.device.poll_write(*id) });
    let Some(receipt) = receipt else {
        probe.fence_transport();
        panic!("N1J: write did not settle; the medium and DMA remain fenced");
    };
    assert_eq!(receipt.request, Some(*id));
    probe.confirm(receipt, WireOp::Write, 191);
    probe.authority.complete_live(write, None).unwrap();
    assert_eq!(
        probe.authority.complete_live(write, None),
        Err(Error::Stale)
    );
    assert_eq!(probe.device.drain(1), DrainOutcome::Complete);
    probe.barrier(retirement);
    let snapshot = probe.authority.snapshot();
    assert_eq!(snapshot.status, Status::Vacant);
    assert_eq!(
        (
            snapshot.outstanding,
            snapshot.retained_data,
            snapshot.flush_pending
        ),
        (0, 0, false)
    );
    probe.authority.remove_media(&test.media).unwrap();
    assert!(!probe.device.has_uncertain_io());
    let mut reset = unsafe { probe.device.begin_terminal_reset() };
    while reset.state == ResetState::Waiting {
        reset = unsafe { probe.device.poll_terminal_reset() };
    }
    let last = probe.previous.unwrap();
    assert_eq!(reset.state, ResetState::VerifiedRetained);
    assert_eq!(
        (reset.device, reset.queue_incarnation, reset.outstanding),
        (last.device, last.queue_incarnation, None)
    );
    assert_eq!(probe.device.mapping_users(), Some(0));
    assert_eq!((probe.data, probe.barriers), (128, 129));
    test.done = true;
    test.binding = None;
    crate::serial_println!(
        "[VFS-N1J] retained queue={:#x} queue_pages={} bounce={:#x} bounce_pages=1",
        test.queue.as_u64(),
        QUEUE_PAGES,
        test.bounce.as_u64()
    );
    #[cfg(not(feature = "vfs-n1m-test"))]
    crate::serial_println!("[VFS-N1J] PASS cycles=64 data=129 barriers=129 image_retired=1 pending=0 mapping_users=0 reset=VerifiedRetained");
    #[cfg(feature = "vfs-n1m-test")]
    crate::serial_println!("[VFS-N1M] PASS cycles=64 data=129 barriers=129 image_retired=1 pending=0 mapping_users=0 reset=VerifiedRetained");
}
