//! Mapping-only debug adapter. SCHEDULER -> PMM -> CAP_BROKER; no user pointers.
//! Each page is a normal SHM object and uses the normal verified final-view path.
use super::{
    channel::{Error, Handle, State, Terminal},
    delivery::RecipientIdentity,
    CapabilityBroker, CapabilityToken, CAP_BROKER,
};
use crate::{
    memory::{pmm::PhysicalMemoryManager, shared},
    process::{
        region::{RegionBacking, RegionProtection},
        Process,
    },
    sched::Scheduler,
};
use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::{structures::paging::PhysFrame, VirtAddr};

static ACTIVE: AtomicBool = AtomicBool::new(false);
// One isolated diagnostic owner; no Account charge or channel receipt is forged.
// The original backing has been freed through the ordinary path before this
// private test owner is created. SCHEDULER serializes acquisition and release.
#[cfg(all(debug_assertions, feature = "restricted_remote_test"))]
static REUSE: spin::Mutex<Option<(RecipientIdentity, Handle, u64, u64)>> = spin::Mutex::new(None);
#[cfg(all(debug_assertions, feature = "restricted_remote_test"))]
static REUSE_GENERATION: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

impl Scheduler {
    /// Expiry, logical death, image replacement, session withdrawal and explicit
    /// revoke all converge here before another view can be admitted.
    pub(crate) fn fastlane_mapping_sweep(&mut self) {
        #[cfg(all(debug_assertions, feature = "fastlane_contract_test"))]
        self.fastlane_hybrid_sweep(&mut CAP_BROKER.lock());
        #[cfg(all(debug_assertions, feature = "fastlane_doorbell_test"))]
        self.fastlane_bell_sweep();
        if !ACTIVE.load(Ordering::Acquire) {
            return;
        }
        let mut pmm = crate::PMM.lock();
        let mut caps = CAP_BROKER.lock();
        self.fastlane_drain_mappings(&mut pmm, &mut caps);
    }

    pub(crate) fn fastlane_drain_mappings(
        &mut self,
        pmm: &mut PhysicalMemoryManager,
        caps: &mut CapabilityBroker,
    ) {
        caps.expire_sessions(self.global_tick);
        caps.channels.expire(self.global_tick);
        for row in &mut caps.channels.rows {
            if self.channel_process(row.creator).is_none()
                || row
                    .consumer
                    .is_some_and(|c| self.channel_process(c).is_none())
                || !self
                    .channel_process(row.issuer)
                    .is_some_and(|p| p.trusted_auth_broker)
                || !self
                    .channel_process(row.creator)
                    .is_some_and(|p| p.uid == 0)
                || row
                    .consumer
                    .is_some_and(|c| !self.channel_process(c).is_some_and(|p| p.uid == 0))
            {
                row.retire(Terminal::IdentityChanged);
            }
        }
        let hhdm = VirtAddr::new(
            crate::HHDM_REQ
                .response()
                .expect("restricted cleanup HHDM")
                .offset,
        );
        // Eight channel rows, three backing objects, two authorized VMs. No
        // allocation while retiring. Rows remain unrecyclable until fully drained.
        for i in 0..caps.channels.rows.len() {
            if caps.channels.rows[i].state != State::Retired {
                continue;
            }
            for region in 0..3 {
                let Some(token) = caps.channels.rows[i].backing[region] else {
                    continue;
                };
                if let Some(backing) = caps.revoke_shared(token) {
                    backing.reclaim(pmm);
                }
                for p in &mut self.processes {
                    while p.mapped_shared.iter().any(|(t, _, _)| *t == token) {
                        shared::free_shared_page(p, token, pmm, caps, hhdm).expect(
                            "restricted SHM preflight failed; retain backing and fail stop",
                        );
                    }
                }
                assert!(
                    caps.shared_region_for_cleanup(token).is_none(),
                    "restricted backing retains an unaccounted view"
                );
                caps.channels.rows[i].backing[region] = None;
            }
        }
        ACTIVE.store(
            caps.channels
                .rows
                .iter()
                .any(|r| r.backing.iter().any(Option::is_some)),
            Ordering::Release,
        );
    }

    pub(crate) fn fastlane_mapping_inventory(&mut self, checkpoint: u64) -> (u64, u64) {
        self.fastlane_mapping_sweep();
        let pmm = crate::PMM.lock();
        let caps = CAP_BROKER.lock();
        let objects = caps.shared_regions.iter().filter(|e| e.restricted).count();
        let views: usize = caps
            .shared_regions
            .iter()
            .filter(|e| e.restricted)
            .map(|e| e.lifetime.views())
            .sum();
        let frames: usize = caps
            .shared_regions
            .iter()
            .filter(|e| e.restricted)
            .map(|e| e.obj.frames.len())
            .sum();
        let pending = self
            .processes
            .iter()
            .filter(|p| {
                p.name_str() == "session-racer"
                    && p.is_exiting()
                    && p.state != crate::process::ProcessState::Reaped
            })
            .count();
        if pending != 0 {
            return (objects as u64, views as u64 | ((pending as u64) << 32));
        }
        #[cfg(all(debug_assertions, feature = "fastlane_census_trace"))]
        {
            let (bytes, entries) = crate::memory::shared_quota::account_observation();
            let receipts = caps
                .shared_regions
                .iter()
                .filter(|e| e.restricted && e.quota.is_some())
                .count();
            crate::serial_println!("[FL-QUOTA] event=checkpoint checkpoint={} account_bytes={} account_entries={} restricted_receipts={} objects={} views={} frames={}", checkpoint, bytes, entries, receipts, objects, views, frames);
        }
        #[cfg(all(debug_assertions, feature = "fastlane_census_trace"))]
        {
            use crate::memory::census_trace::{self, VIEW};
            let hhdm = VirtAddr::new(crate::HHDM_REQ.response().expect("census HHDM").offset);
            pmm.trace_census(checkpoint);
            for entry in &caps.shared_regions {
                entry.trace(census_trace::SHM_LIVE, checkpoint);
                for row in &caps.channels.rows {
                    for (region, backing) in row.backing.iter().enumerate() {
                        if *backing == Some(entry.token) {
                            census_trace::record(
                                VIEW,
                                [
                                    entry.trace_id,
                                    row.handle.generation,
                                    region as u64,
                                    0,
                                    0,
                                    0,
                                    checkpoint,
                                    1,
                                ],
                            );
                        }
                    }
                }
            }
            for p in &self.processes {
                unsafe {
                    p.address_space.trace_tables(
                        checkpoint,
                        (p.pid, p.foreground.incarnation),
                        hhdm,
                    );
                }
                for (token, va, size) in &p.mapped_shared {
                    if let Some(entry) = caps.shared_regions.iter().find(|e| e.token == *token) {
                        census_trace::record(
                            VIEW,
                            [
                                entry.trace_id,
                                p.pid as u64,
                                p.foreground.incarnation,
                                p.address_space.identity().generation,
                                va.as_u64(),
                                *size as u64,
                                checkpoint,
                                0,
                            ],
                        );
                    }
                }
            }
            census_trace::checkpoint(checkpoint);
        }
        crate::serial_println!(
            "[FL-CENSUS] checkpoint={} objects={} views={} frames={} shm={} tables={} free={}",
            checkpoint,
            objects,
            views,
            frames,
            crate::memory::accounting::class_frame_count(
                crate::memory::accounting::PhysicalMemoryClass::SharedMemory
            ),
            crate::memory::accounting::class_frame_count(
                crate::memory::accounting::PhysicalMemoryClass::PageTable
            ),
            pmm.free_page_count()
        );
        (objects as u64, views as u64)
    }

    /// Fixture-only retryable rendezvous. The existing SCHEDULER guard pins
    /// process selection once real remote residency is observed. No extra
    /// delay, global flush, VM activation or admission override is introduced.
    #[cfg(all(debug_assertions, feature = "restricted_remote_test"))]
    pub(crate) fn fastlane_remote_revoke(&mut self, h: Handle) -> Result<(), Error> {
        let who = RecipientIdentity::of(self.current_process());
        let local = crate::sched::current_cpu_id();
        let (consumer, identity, va, frame, targets) = {
            let mut caps = CAP_BROKER.lock();
            let row = caps.channels.row_mut(h)?;
            if row.creator != who || row.state != State::Attached {
                return Err(Error::Denied);
            }
            let consumer = row.consumer.ok_or(Error::Incomplete)?;
            let token = row.backing[0].ok_or(Error::Incomplete)?;
            let p = self.channel_process(consumer).ok_or(Error::Denied)?;
            let va = p
                .mapped_shared
                .iter()
                .find(|(t, _, _)| *t == token)
                .ok_or(Error::Incomplete)?
                .1
                .as_u64();
            let identity = p.address_space.identity();
            let targets = crate::memory::tlb::active_cpu_mask(identity) & !(1u64 << local);
            if targets == 0
                || p.state != crate::process::ProcessState::Running
                || p.owning_core == u8::MAX
                || targets != 1u64 << p.owning_core
            {
                return Err(Error::Pending);
            }
            let frame = caps
                .shared_region_for_cleanup(token)
                .ok_or(Error::Terminal)?
                .frames[0]
                .start_address()
                .as_u64();
            (consumer, identity, va, frame, targets)
        };
        crate::serial_println!("[R5-REMOTE] event=resident generation={} pid={} incarnation={} vm={} root={} va={} frame={} targets={} cpu={}", h.generation, consumer.pid, consumer.incarnation, identity.generation, identity.pml4_phys, va, frame, targets, local);
        // Re-enter the ordinary exact-identity and role-authorized revoke API.
        self.fastlane_channel(
            4,
            h,
            0,
            sunlight_ipc::account_session::SessionRef { id: 0, epoch: 0 },
        )?;
        crate::serial_println!(
            "[R5-REMOTE] event=revoke-committed generation={} vm={} frame={} cpu={}",
            h.generation,
            identity.generation,
            frame,
            local
        );
        self.fastlane_mapping_sweep();
        {
            let mut witness = REUSE.lock();
            if witness.is_some() {
                return Err(Error::Capacity);
            }
            let mut pmm = crate::PMM.lock();
            let claimed = pmm
                .claim_retired_test_frame(frame)
                .ok_or(Error::Incomplete)?;
            let generation = REUSE_GENERATION
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .expect("reuse generation exhausted");
            let hhdm = crate::HHDM_REQ.response().expect("reuse HHDM").offset;
            unsafe {
                ((hhdm + claimed.as_u64()) as *mut u64).write_volatile(generation);
            }
            *witness = Some((who, h, frame, generation));
            crate::serial_println!("[R51-REUSE] event=claimed frame={} channel_generation={} owner_kind=isolated-kernel-test owner_generation={} pid={} incarnation={} vm={}",
                frame, h.generation, generation, who.pid, who.incarnation, who.vm_generation);
        }
        crate::serial_println!(
            "[R5-REMOTE] event=retired generation={} vm={} frame={} cpu={}",
            h.generation,
            identity.generation,
            frame,
            local
        );
        Ok(())
    }

    #[cfg(all(debug_assertions, feature = "restricted_remote_test"))]
    pub(crate) fn fastlane_remote_reuse_release(&mut self, h: Handle) -> Result<(), Error> {
        let who = RecipientIdentity::of(self.current_process());
        let mut witness = REUSE.lock();
        let (owner, original, frame, generation) = witness.as_ref().copied().ok_or(Error::Stale)?;
        if who != owner || h != original {
            return Err(Error::Denied);
        }
        let mut pmm = crate::PMM.lock();
        let address = x86_64::PhysAddr::new(frame);
        assert_eq!(
            pmm.owner_of(address),
            Some(crate::memory::pmm::PMM_OWNER_KERNEL)
        );
        assert_eq!(
            pmm.class_of(address),
            Some(crate::memory::accounting::PhysicalMemoryClass::KernelCore)
        );
        let hhdm = crate::HHDM_REQ.response().expect("reuse HHDM").offset;
        assert_eq!(
            unsafe { ((hhdm + frame) as *const u64).read_volatile() },
            generation,
            "isolated reused backing modified through stale mapping"
        );
        pmm.free_frame(address);
        *witness = None;
        crate::serial_println!(
            "[R51-REUSE] event=released frame={} channel_generation={} owner_generation={}",
            frame,
            h.generation,
            generation
        );
        Ok(())
    }

    /// arg = role * 3 + region. Maps exactly one page, once per role/region.
    pub(crate) fn fastlane_map(&mut self, h: Handle, arg: u64) -> Result<(u64, u64), Error> {
        self.fastlane_mapping_sweep();
        let who = RecipientIdentity::of(self.current_process());
        if !self.channel_vm_exclusive(who) {
            return Err(Error::Denied);
        }
        let mut pmm = crate::PMM.lock();
        let mut caps = CAP_BROKER.lock();
        self.fastlane_drain_mappings(&mut pmm, &mut caps);
        let (role, region, writable) = caps.channel_mapping_authorize(h, who, arg)?;
        let row = caps.channels.row_mut(h)?;
        let old = row.backing[region];
        let quota_owner = crate::memory::shared_quota::ProcessIdentity {
            pid: row.creator.pid,
            incarnation: row.creator.incarnation,
        };
        let quota_session = crate::memory::shared_quota::SessionIdentity::Account {
            id: row.session.id,
            epoch: row.session.epoch,
        };
        let p = self.current_process_mut();
        p.mapped_shared
            .try_reserve(1)
            .map_err(|_| Error::Capacity)?;
        let hhdm = VirtAddr::new(crate::HHDM_REQ.response().ok_or(Error::Capacity)?.offset);
        let protection = if writable {
            RegionProtection::READ_WRITE
        } else {
            RegionProtection::READ_ONLY
        };
        let (token, va) = if let Some(token) = old {
            // Restricted cleanup lookup is usable only after exact channel,
            // incarnation, VM, attachment and role checks above.
            if !caps.preflight_shared_mapping(token) {
                return Err(Error::Terminal);
            }
            let obj = caps
                .shared_region_for_cleanup(token)
                .ok_or(Error::Terminal)?;
            let va = unsafe {
                p.address_space.map_shared_region_protected(
                    &obj.frames,
                    RegionBacking::SharedMemory(token.0),
                    protection,
                    &mut pmm,
                    hhdm,
                )
            }
            .map_err(|_| Error::Capacity)?;
            (token, va)
        } else {
            let free_before = pmm.free_page_count();
            let quota = crate::memory::shared_quota::reserve(
                quota_owner,
                Some(quota_session),
                shared::PAGE_SIZE,
            )
            .ok_or(Error::Capacity)?;
            let prepared = caps.prepare_shared_region().map_err(|_| Error::Capacity)?;
            let mut frames = alloc::vec::Vec::new();
            frames.try_reserve_exact(1).map_err(|_| Error::Capacity)?;
            let physical = pmm
                .alloc_frame_owned_class(
                    p.pid as u32,
                    crate::memory::accounting::PhysicalMemoryClass::SharedMemory,
                )
                .ok_or(Error::Capacity)?;
            if !crate::memory::security::sanitize_user_frame(physical, hhdm) {
                pmm.free_frame(physical);
                return Err(Error::Capacity);
            }
            frames.push(unsafe { PhysFrame::from_start_address_unchecked(physical) });
            let va = match unsafe {
                p.address_space.map_shared_region_protected(
                    &frames,
                    RegionBacking::None,
                    protection,
                    &mut pmm,
                    hhdm,
                )
            } {
                Ok(va) => va,
                Err(_) => {
                    pmm.free_frame(physical);
                    crate::serial_println!("[FL-ROLLBACK] generation={} region={} pid={} free_before={} free_after={} frame={} published=0", h.generation, region, p.pid, free_before, pmm.free_page_count(), physical.as_u64());
                    return Err(Error::Capacity);
                }
            };
            // Restricted channels use their authenticated session admission,
            // not the legacy bearer SHM authorization contract.
            let token = prepared.commit(frames, shared::PAGE_SIZE, p.pid, Some(quota));
            caps.shared_regions
                .iter_mut()
                .find(|e| e.token == token)
                .expect("new restricted object")
                .restricted = true;
            p.address_space
                .update_region_backing(
                    va.as_u64(),
                    va.as_u64() + 4096,
                    crate::process::region::MappingKind::SharedMemory,
                    RegionBacking::None,
                    RegionBacking::SharedMemory(token.0),
                )
                .expect("restricted publication ledger");
            caps.channels.row_mut(h)?.backing[region] = Some(token);
            super::mapping_trace::backing(h.generation, region, physical.as_u64());
            (token, va)
        };
        caps.increment_map_count(token);
        p.mapped_shared.push((token, va, shared::PAGE_SIZE));
        #[cfg(all(debug_assertions, feature = "fastlane_census_trace"))]
        caps.trace_view_add(p, token, va.as_u64(), shared::PAGE_SIZE);
        caps.channels.row_mut(h)?.mapped[role][region] = true;
        ACTIVE.store(true, Ordering::Release);
        let frame = caps
            .shared_region_for_cleanup(token)
            .expect("restricted frame")
            .frames[0]
            .start_address()
            .as_u64();
        super::mapping_trace::mapped(h.generation, region, role, p, va.as_u64(), frame, writable);
        Ok((va.as_u64(), 4096))
    }
}

/// Called only by the shared release path while SCHEDULER and CAP_BROKER are held.
pub(crate) fn trace_unmap(
    caps: &CapabilityBroker,
    p: &Process,
    token: CapabilityToken,
    va: u64,
    event: &str,
) {
    #[cfg(feature = "fastlane_hybrid_lifecycle_test")]
    if p.pid == 0xF103 && p.name_str() == "f1-remote" {
        super::mapping_trace::unmap(u64::MAX, 0, p, va, event);
        return;
    }
    if let Some((row, region)) = caps.channels.rows.iter().find_map(|row| {
        row.backing
            .iter()
            .position(|t| *t == Some(token))
            .map(|region| (row, region))
    }) {
        super::mapping_trace::unmap(row.handle.generation, region, p, va, event);
    }
}
