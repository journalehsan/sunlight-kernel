//! M5 debug-only word backend. Existing SCHEDULER serializes each admitted
//! access against M2 view withdrawal and backing retirement. No retained pointer,
//! Rust reference into shared bytes, allocation, or new lock. This deliberately
//! favors a small lifetime proof over syscall performance.
use super::{
    channel::{Error, Handle, State},
    delivery::RecipientIdentity,
    CAP_BROKER,
};
use crate::sched::Scheduler;
use core::sync::atomic::{compiler_fence, Ordering};

impl Scheduler {
    /// Fixed geometry: three pages of 512 aligned words. The selector is a word
    /// index, never a pointer/byte length. r8 carries a store value, not a session.
    pub(crate) fn fastlane_word(
        &mut self,
        h: Handle,
        store: bool,
        index: u64,
        value: u64,
    ) -> Result<u64, Error> {
        self.fastlane_mapping_sweep();
        let who = RecipientIdentity::of(self.current_process());
        if !self.channel_vm_exclusive(who) || index >= 3 * 512 {
            return Err(Error::Denied);
        }
        let region = (index / 512) as usize;
        let word = (index % 512) as usize;
        // M5 admits only descriptor zero, consumer cursor zero and payload
        // zero. The M3.2 ring's other slots are deliberately unavailable.
        if (region == 0 && word > 5) || (region == 1 && word != 0) || (region == 2 && word >= 32) {
            return Err(Error::Denied);
        }
        let mut caps = CAP_BROKER.lock();
        let row = caps.channels.row_mut(h)?;
        let role = if row.creator == who {
            0
        } else if row.consumer == Some(who) {
            1
        } else {
            return Err(Error::Denied);
        };
        if !row.live() {
            return Err(Error::Terminal);
        }
        if row.state != State::Attached || !row.mapped[role].iter().all(|v| *v) {
            return Err(Error::Incomplete);
        }
        if store && (if region == 1 { role != 1 } else { role != 0 }) {
            return Err(Error::Denied);
        }
        // The cursor stores are the only admitted ownership transitions. One
        // message, sequence 0 / slot generation 1, for this fresh M1 generation.
        // A hostile direct writer may corrupt bytes but cannot advance this
        // private state, add authority, or cause a second admitted publication.
        let transition = store && word == 0 && region != 2;
        if transition {
            if value != 1 {
                return Err(Error::Denied);
            }
            if row.transfer != if region == 0 { 0 } else { 1 } {
                return Err(Error::Duplicate);
            }
        }
        // Peer-written cursor bytes alone do not admit a message. Even a
        // hostile direct M2 writer must pass the authenticated publication
        // transition before the consumer representation can observe work.
        if !store && role == 1 && region == 0 && word == 0 && row.transfer == 0 {
            return Ok(0);
        }
        let token = row.backing[region].ok_or(Error::Incomplete)?;
        let p = self.current_process();
        if !p
            .mapped_shared
            .iter()
            .any(|(t, _, size)| *t == token && *size == 4096)
        {
            return Err(Error::Terminal);
        }
        let backing = caps
            .shared_region_for_cleanup(token)
            .ok_or(Error::Terminal)?;
        if backing.frames.len() != 1 || !caps.preflight_shared_mapping(token) {
            return Err(Error::Terminal);
        }
        let hhdm = crate::HHDM_REQ.response().ok_or(Error::Capacity)?.offset;
        let address = hhdm
            .checked_add(backing.frames[0].start_address().as_u64())
            .and_then(|a| a.checked_add((word * 8) as u64))
            .ok_or(Error::Denied)?;
        // SAFETY: this is the exact still-live M2 backing for this authenticated
        // process's recorded view. SCHEDULER and CAP_BROKER remain held through
        // the access, preventing unmap/reuse; the canonical aligned offset is
        // wholly inside its one initialized page. x86-64 aligned MOV is atomic.
        // Compiler barriers plus x86 TSO implement Release stores / Acquire
        // loads without constructing an AtomicU64 or any Rust shared reference.
        let result;
        unsafe {
            if store {
                compiler_fence(Ordering::Release);
                core::arch::asm!("mov qword ptr [{address}], {value}", address=in(reg) address,
                    value=in(reg) value, options(nostack));
                result = 0;
            } else {
                core::arch::asm!("mov {value}, qword ptr [{address}]", address=in(reg) address,
                    value=out(reg) result, options(nostack, readonly));
                compiler_fence(Ordering::Acquire);
            }
        }
        if transition {
            caps.channels.row_mut(h)?.transfer = if region == 0 { 1 } else { 2 };
            crate::serial_println!("[FL-TRANSFER] event={} generation={} message=0 slot_generation=1 pid={} inc={} vm={} cpu={}",
                if region == 0 { "publish" } else { "ack" }, h.generation, who.pid,
                who.incarnation, who.vm_generation, crate::sched::current_cpu_id());
        }
        Ok(result)
    }
}
