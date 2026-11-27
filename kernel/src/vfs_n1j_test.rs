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
    #[cfg(feature = "vfs_c1e1_test")]
    storage: Option<StorageReservation>,
    registration: Registration,
    session: Session,
    pending: Option<(Lease, Operation, sunlight_block::contract::RequestId)>,
    retirement: Option<Transition>,
    image_retired: bool,
    #[cfg(feature = "vfs-n1m-test")]
    active: Option<Lease>,
    #[cfg(feature = "vfs-n1m-test")]
    settled: Option<Transition>,
}

#[cfg(feature = "vfs_c1e1_test")]
struct StorageReservation {
    caller: Caller,
    request: u64,
    source: u64,
    media: Media,
    generation: u64,
    cancelled: bool,
    #[cfg(feature = "vfs_c1e2_test")]
    received: bool,
    #[cfg(feature = "vfs_c1e2_test")]
    ready: bool,
}

struct Qualification {
    probe: Probe,
    media: Media,
    #[cfg(feature = "vfs_c1c_test")]
    source: sunlight_block::registry::DeviceInfo,
    #[cfg(feature = "vfs_c1c_test")]
    physical: sunlight_block::registry::DeviceInfo,
    binding: Option<Binding>,
    queue: PhysAddr,
    bounce: PhysAddr,
    done: bool,
}

static TEST: Mutex<Option<Qualification>> = Mutex::new(None);

#[cfg(not(feature = "vfs_c1c_test"))]
const WHOLE: Extent = Extent {
    offset: 0,
    length: BLOCKS * 512,
};
#[cfg(not(feature = "vfs_c1c_test"))]
const BLOCKS: u64 = if cfg!(feature = "vfs-n1o-test") {
    131072
} else {
    8192
};
const READ_WRITE: Rights = Rights {
    read: true,
    write: true,
    flush: true,
};

pub(crate) fn install(
    device: VirtioBlk,
    queue: PhysAddr,
    bounce: PhysAddr,
    #[cfg(feature = "vfs_c1c_test")] source: u64,
) {
    assert!(crate::MP_REQ
        .response()
        .is_none_or(|response| response.cpus().len() == 1));
    #[cfg(feature = "vfs_c1c_test")]
    let (physical, source) = crate::storage::DEVICES.lock().resolve_extent(source).unwrap();
    #[cfg(feature = "vfs_c1c_test")]
    assert_eq!(physical.blocks, device.block_count());
    let probe = initialize(device);
    let media = probe
        .authority
        .register_media(Geometry {
            #[cfg(feature = "vfs_c1c_test")]
            blocks: physical.blocks,
            #[cfg(not(feature = "vfs_c1c_test"))]
            blocks: BLOCKS,
            block_bytes: 512,
        })
        .unwrap();
    assert!(TEST
        .lock()
        .replace(Qualification {
            probe,
            media,
            #[cfg(feature = "vfs_c1c_test")]
            physical,
            #[cfg(feature = "vfs_c1c_test")]
            source,
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
        #[cfg(feature = "vfs_c1e1_test")]
        storage: None,
        registration,
        session,
        pending: None,
        retirement: None,
        image_retired: false,
        #[cfg(feature = "vfs-n1m-test")]
        active: None,
        #[cfg(feature = "vfs-n1m-test")]
        settled: None,
    });
    crate::serial_println!("[VFS-N1J] authenticated {:?}", registration);
}

#[cfg(feature = "vfs_c1e1_test")]
pub(crate) fn acquire_storage(
    registration: Registration,
    caller: Caller,
    request: u64,
    source: u64,
) -> Result<u64, Error> {
    let mut slot = TEST.lock();
    let test = slot.as_mut().filter(|test| !test.done).ok_or(Error::Unauthorized)?;
    let binding = test.binding.as_mut()
        .filter(|binding| binding.registration == registration && binding.retirement.is_none())
        .ok_or(Error::Unauthorized)?;
    if binding.active.is_some() || binding.settled.is_some() || binding.storage.is_some() {
        return Err(Error::Busy);
    }
    if test.source.id != source
        || crate::storage::DEVICES.lock().resolve_extent(source) != Some((test.physical, test.source))
    {
        return Err(Error::Stale);
    }
    let lease = test.probe.authority.acquire_authenticated(
        &test.media,
        &binding.session,
        Extent { offset: 0, length: test.physical.blocks.checked_mul(512).ok_or(Error::Unauthorized)? },
        READ_WRITE,
    )?;
    let generation = lease.generation();
    binding.storage = Some(StorageReservation {
        caller, request, source, media: test.media.clone(), generation, cancelled: false,
        #[cfg(feature = "vfs_c1e2_test")]
        received: false,
        #[cfg(feature = "vfs_c1e2_test")]
        ready: false,
    });
    binding.active = Some(lease);
    Ok(generation)
}

#[cfg(feature = "vfs_c1e1_test")]
pub(crate) fn storage_reply(
    registration: Registration,
    caller: Caller,
    request: u64,
    source: u64,
    generation: u64,
    errno: u64,
    consume: bool,
) -> bool {
    let mut slot = TEST.lock();
    let Some(test) = slot.as_mut() else { return false };
    let Some(binding) = test.binding.as_mut()
        .filter(|binding| binding.registration == registration && binding.retirement.is_none())
    else { return false };
    let Some(reservation) = binding.storage.as_ref().filter(|entry| entry.request == request)
    else { return errno != 0 };
    let retained = cfg!(feature = "vfs_c1e2_test") && errno == 0;
    let snapshot = test.probe.authority.snapshot();
    let ownership_valid = if retained {
        #[cfg(feature = "vfs_c1e2_test")]
        if !reservation.ready || test.probe.device.mapping_users() != Some(0)
            || test.probe.device.outstanding().is_some()
        {
            return false;
        }
        binding.active.as_ref().is_some_and(|lease| lease.generation() == generation)
            && snapshot.status == Status::Active
            && snapshot.outstanding == 0 && snapshot.retained_data == 0 && !snapshot.flush_pending
    } else {
        binding.active.is_none() && snapshot.status == Status::Vacant
    };
    if reservation.cancelled || reservation.caller != caller || reservation.source != source
        || !ownership_valid || binding.settled.is_some()
        || (errno == 0 && (reservation.generation != generation
            || reservation.media != test.media
            || crate::storage::DEVICES.lock().resolve_extent(source) != Some((test.physical, test.source))))
    {
        return false;
    }
    #[cfg(feature = "vfs_c1e2_test")]
    if consume && retained {
        binding.storage.as_mut().unwrap().received = true;
    }
    if consume && !retained {
        binding.storage = None;
    }
    true
}

#[cfg(feature = "vfs_c1e1_test")]
pub(crate) fn cancel_storage(registration: Registration, caller: Caller, request: u64) {
    let mut slot = TEST.lock();
    let Some(binding) = slot.as_mut().and_then(|test| test.binding.as_mut())
        .filter(|binding| binding.registration == registration)
    else { return };
    if let Some(reservation) = binding.storage.as_mut()
        .filter(|reservation| reservation.caller == caller && reservation.request == request)
    {
        if binding.active.is_some() || binding.settled.is_some() {
            reservation.cancelled = true;
        } else {
            binding.storage = None;
        }
    }
}

#[cfg(feature = "vfs_c1e2_test")]
pub(crate) fn validate_storage_publication(
    registration: Registration,
    caller: Caller,
    cookie: u64,
) -> bool {
    let slot = TEST.lock();
    let Some(test) = slot.as_ref().filter(|test| !test.done) else { return false };
    let Some(binding) = test.binding.as_ref().filter(|binding| {
        binding.registration == registration && binding.retirement.is_none()
    }) else { return false };
    let Some(entry) = binding.storage.as_ref() else { return false };
    let snapshot = test.probe.authority.snapshot();
    entry.caller == caller && entry.request == cookie && entry.received && entry.ready
        && !entry.cancelled && entry.media == test.media && entry.source == test.source.id
        && binding.settled.is_none()
        && binding.active.as_ref().is_some_and(|lease| {
            lease.generation() == entry.generation && lease.owner() == registration
        })
        && snapshot.status == Status::Active && snapshot.outstanding == 0
        && snapshot.retained_data == 0 && !snapshot.flush_pending
        && test.probe.device.mapping_users() == Some(0)
        && test.probe.device.outstanding().is_none()
        && crate::storage::DEVICES.lock().resolve_extent(entry.source)
            == Some((test.physical, test.source))
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
    let test = slot
        .as_mut()
        .filter(|test| !test.done)
        .ok_or(Error::Unauthorized)?;
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
        if cfg!(feature = "vfs_c1e1_test") {
            return Err(Error::Unauthorized);
        }
        if generation != 0 || binding.active.is_some() {
            return Err(Error::Busy);
        }
        #[cfg(feature = "vfs_c1c_test")]
        let whole = {
            let registry = crate::storage::DEVICES.lock();
            if registry.resolve_extent(test.source.id) != Some((test.physical, test.source)) {
                return Err(Error::Stale);
            }
            Extent {
                offset: 0,
                length: test.physical.blocks.checked_mul(512).ok_or(Error::Unauthorized)?,
            }
        };
        #[cfg(not(feature = "vfs_c1c_test"))]
        let whole = WHOLE;
        let lease = probe.authority.acquire_authenticated(
            &test.media,
            &binding.session,
            whole,
            READ_WRITE,
        )?;
        let generation = lease.generation();
        binding.active = Some(lease);
        #[cfg(feature = "vfs_c1c_test")]
        crate::serial_println!(
            "[VFS-C1C] lease={} disk={} source={} start={} blocks={} whole={}",
            generation, test.physical.id, test.source.id, test.source.start,
            test.source.blocks, test.physical.blocks
        );
        return Ok(generation);
    }
    let lease = binding
        .active
        .as_ref()
        .filter(|lease| lease.generation() == generation)
        .ok_or(Error::Stale)?;
    if cfg!(feature = "vfs-n1o-test") && matches!(op, 0x104 | 0x105) {
        return Err(Error::Unauthorized);
    }
    match op {
        0x101..=0x102 | 0x105 => {
            #[cfg(feature = "vfs_c1c_test")]
            let window = test.source.start..test.source.start + test.source.blocks;
            #[cfg(not(feature = "vfs_c1c_test"))]
            let window = if cfg!(feature = "vfs-n1o-test") {
                2048..BLOCKS
            } else {
                128..192
            };
            if !window.contains(&lba) || (op == 0x105 && lba != 191) {
                return Err(Error::Unauthorized);
            }
            let range = sector(lba);
            if op == 0x101 {
                let ticket = probe
                    .authority
                    .read_authenticated(&binding.session, lease, range)?;
                *bytes = probe.execute(ticket);
            } else {
                let ticket =
                    probe
                        .authority
                        .write_authenticated(&binding.session, lease, range, *bytes)?;
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
        0x104 if !cfg!(feature = "vfs_c1e1_test") => {
            let release = probe.authority.begin_release(lease)?;
            probe.barrier(&release);
            probe.authority.finish(&release)?;
            binding.active = None;
        }
        0x106 => {
            let release = probe.authority.begin_release(lease)?;
            probe.barrier(&release);
            let snapshot = probe.authority.snapshot();
            assert_eq!(
                (
                    snapshot.outstanding,
                    snapshot.retained_data,
                    snapshot.flush_pending
                ),
                (0, 0, false)
            );
            assert_eq!(probe.device.outstanding(), None);
            assert_eq!(probe.device.mapping_users(), Some(0));
            binding.settled = Some(release);
            crate::serial_println!(
                "[VFS-N1O] settled lease={} pending=0 mapping_users=0 queue={:#x} bounce={:#x} retained_bytes={}",
                generation, test.queue.as_u64(), test.bounce.as_u64(), (QUEUE_PAGES + 1) * 4096
            );
        }
        0x107 => {
            probe
                .authority
                .finish(binding.settled.as_ref().ok_or(Error::Stale)?)?;
            assert_eq!(probe.authority.snapshot().status, Status::Vacant);
            binding.settled = None;
            binding.active = None;
            #[cfg(feature = "vfs_c1e1_test")]
            if binding.storage.as_ref().is_some_and(|entry| {
                entry.cancelled || {
                    #[cfg(feature = "vfs_c1e2_test")]
                    { entry.received }
                    #[cfg(not(feature = "vfs_c1e2_test"))]
                    { false }
                }
            }) {
                binding.storage = None;
            }
            crate::serial_println!("[VFS-N1O] released lease={}", generation);
        }
        0x108 => probe.authority.transport_lost(lease)?,
        0x109 => {
            probe.authority.with_attachment_pin(&binding.session, lease, || ())?;
            #[cfg(feature = "vfs_c1e2_test")]
            {
                let entry = binding.storage.as_mut().ok_or(Error::Stale)?;
                if entry.cancelled {
                    return Err(Error::Stale);
                }
                entry.ready = true;
            }
        }
        #[cfg(feature = "vfs_c1e2_test")]
        0x10b => {
            let entry = binding.storage.as_ref().ok_or(Error::Stale)?;
            return Ok(u64::from(entry.cancelled));
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
        if binding.settled.is_some() || matches!(authority.snapshot().status, Status::Fenced(_)) {
            let _ = authority.transport_lost(&lease);
        } else {
            binding.retirement = Some(authority.retire(&lease).unwrap());
        }
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
        #[cfg(feature = "vfs_c1e2_test")]
        {
            let snapshot = test.probe.authority.snapshot();
            assert_eq!(snapshot.status, Status::Vacant);
            assert_eq!((snapshot.outstanding, snapshot.retained_data, snapshot.flush_pending), (0, 0, false));
            assert_eq!(test.probe.device.outstanding(), None);
            assert_eq!(test.probe.device.mapping_users(), Some(0));
            crate::serial_println!(
                "[VFS-C1E2] exact service image retired lease=vacant pending=0 mapping_users=0 queue={:#x} bounce={:#x} retained_bytes={}",
                test.queue.as_u64(), test.bounce.as_u64(), (QUEUE_PAGES + 1) * 4096
            );
        }
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
