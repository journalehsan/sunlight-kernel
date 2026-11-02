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

impl Scheduler {
    /// Expiry, logical death, image replacement, session withdrawal and explicit
    /// revoke all converge here before another view can be admitted.
    pub(crate) fn fastlane_mapping_sweep(&mut self) {
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
                if let Some(frames) = caps.revoke_shared(token) {
                    for f in frames {
                        pmm.free_frame(f.start_address());
                    }
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
        let old = caps.channels.row_mut(h)?.backing[region];
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
            let token = prepared.commit(frames, shared::PAGE_SIZE, p.pid);
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
    if let Some((row, region)) = caps.channels.rows.iter().find_map(|row| {
        row.backing
            .iter()
            .position(|t| *t == Some(token))
            .map(|region| (row, region))
    }) {
        super::mapping_trace::unmap(row.handle.generation, region, p, va, event);
    }
}
