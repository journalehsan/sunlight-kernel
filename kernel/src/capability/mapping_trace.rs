//! Debug-only observations. Never drive authority or lifetime decisions.
//! ISR acknowledgement observations use atomics and never print or take locks.
use crate::process::{
    mm2b_state::{AddressSpaceIdentity, MailboxRequest},
    Process,
};
use core::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(1);
static VM: AtomicU64 = AtomicU64::new(0);
static VA: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static REGION: AtomicU64 = AtomicU64::new(0);
static ACK: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
static FLUSHED: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
struct Frame {
    address: AtomicU64,
    generation: AtomicU64,
    region: AtomicU64,
    live: AtomicU64,
}
static FRAMES: [Frame; 24] = [const {
    Frame {
        address: AtomicU64::new(0),
        generation: AtomicU64::new(0),
        region: AtomicU64::new(0),
        live: AtomicU64::new(0),
    }
}; 24];
fn seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}
pub(crate) fn backing(generation: u64, region: usize, frame: u64) {
    let row = FRAMES
        .iter()
        .find(|f| f.live.load(Ordering::Relaxed) == 0)
        .expect("bounded mapping trace capacity");
    row.address.store(frame, Ordering::Relaxed);
    row.generation.store(generation, Ordering::Relaxed);
    row.region.store(region as u64, Ordering::Relaxed);
    row.live.store(1, Ordering::Relaxed);
    crate::serial_println!(
        "[FL-FRAME] seq={} event=backing generation={} region={} frame={}",
        seq(),
        generation,
        region,
        frame
    );
}
pub(crate) fn physical(frame: u64, event: &str) {
    for f in &FRAMES {
        if f.address.load(Ordering::Relaxed) != frame {
            continue;
        }
        let live = f.live.load(Ordering::Relaxed);
        if event == "free" && live == 1 {
            crate::serial_println!(
                "[FL-FRAME] seq={} event=free generation={} region={} frame={}",
                seq(),
                f.generation.load(Ordering::Relaxed),
                f.region.load(Ordering::Relaxed),
                frame
            );
            f.live.store(0, Ordering::Relaxed);
        } else if event == "free" {
            panic!("tracked physical backing freed twice before reuse");
        } else if event == "claim" && live == 1 {
            panic!("tracked physical backing reused before release");
        } else if event == "claim" && live == 0 {
            crate::serial_println!(
                "[FL-FRAME] seq={} event=reuse generation={} region={} frame={}",
                seq(),
                f.generation.load(Ordering::Relaxed),
                f.region.load(Ordering::Relaxed),
                frame
            );
            f.address.store(0, Ordering::Relaxed);
        }
    }
}
pub(crate) fn mapped(
    generation: u64,
    region: usize,
    role: usize,
    p: &Process,
    va: u64,
    frame: u64,
    writable: bool,
) {
    crate::serial_println!("[FL-MAP] seq={} event=map generation={} region={} role={} pid={} inc={} vm={} va={} frame={} write={} nx=1 cpu={}", seq(), generation, region, role, p.pid, p.foreground.incarnation, p.address_space.identity().generation, va, frame, u8::from(writable), crate::sched::current_cpu_id());
}
pub(crate) fn unmap(generation: u64, region: usize, p: &Process, va: u64, event: &str) {
    if event == "request" {
        GENERATION.store(generation, Ordering::Relaxed);
        REGION.store(region as u64, Ordering::Relaxed);
        VM.store(p.address_space.identity().generation, Ordering::Relaxed);
        VA.store(va, Ordering::Release);
    }
    crate::serial_println!(
        "[FL-MAP] seq={} event={} generation={} region={} pid={} inc={} vm={} va={} cpu={}",
        seq(),
        event,
        generation,
        region,
        p.pid,
        p.foreground.incarnation,
        p.address_space.identity().generation,
        va,
        crate::sched::current_cpu_id()
    );
    if event == "complete" {
        VA.store(0, Ordering::Release);
    }
}
pub(crate) fn ack(request: MailboxRequest, flushed: bool) {
    if request.target.generation == VM.load(Ordering::Relaxed)
        && request.start == VA.load(Ordering::Acquire)
    {
        let cpu = crate::sched::current_cpu_id();
        FLUSHED[cpu].store(u64::from(flushed), Ordering::Relaxed);
        ACK[cpu].store(request.sequence, Ordering::Release);
    }
}
pub(crate) fn tlb(
    id: AddressSpaceIdentity,
    start: u64,
    event: &str,
    request: u64,
    targets: u64,
    cpu: usize,
) {
    if start != VA.load(Ordering::Acquire)
        || id.generation != VM.load(Ordering::Relaxed)
        || start == 0
    {
        return;
    }
    let observed = if event == "ack" {
        ACK[cpu].load(Ordering::Acquire)
    } else {
        0
    };
    crate::serial_println!("[FL-TLB] seq={} event={} generation={} region={} vm={} va={} request={} targets={} cpu={} observed={} flushed={}", seq(), event, GENERATION.load(Ordering::Relaxed), REGION.load(Ordering::Relaxed), id.generation, start, request, targets, cpu, observed, if event == "ack" { FLUSHED[cpu].load(Ordering::Relaxed) } else {0});
}
