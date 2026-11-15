//! Off-by-default disposable active-request observer. Never mounts a filesystem.
use core::sync::atomic::{AtomicU64, Ordering};
use sunlight_block::contract::{Completion, Durability};
use sunlight_virtio::{ResetState, VirtioBlk, RESET_READ_BUDGET};

static TICKS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

pub fn on_tick(cpu: usize) {
    if let Some(ticks) = TICKS.get(cpu) {
        ticks.fetch_add(1, Ordering::Relaxed);
    }
}

fn progress() -> [u64; 4] {
    core::array::from_fn(|cpu| TICKS[cpu].load(Ordering::Relaxed))
}

pub fn run(d: &mut VirtioBlk, queue_phys: u64, req_phys: u64) {
    assert_eq!(d.block_count(), 8192, "SFS5B3 disposable capacity guard");
    let mut marker = [0; 512];
    assert_eq!(
        unsafe { d.read_typed(0, &mut marker) }.completion,
        Completion::Completed
    );
    assert_eq!(&marker[..34], b"SFS5A-ONLY-7b93143a-disposable-v1!");

    // Mirror the normal prerequisites for AP startup; this fixture never returns
    // to normal boot, so neither MSRs nor APs are initialized a second time.
    unsafe {
        crate::syscall::setup_syscall_msrs(x86_64::VirtAddr::new(
            crate::syscall::syscall_entry as *const () as u64,
        ));
    }
    crate::memory::tlb::register_kernel_root();
    let mp = crate::MP_REQ.response().expect("SFS5B3 MP response");
    crate::arch::x86_64::smp::start_aps(mp.cpus(), mp.bsp_lapic_id);
    crate::sched::init_cores(mp.cpus().len());
    x86_64::instructions::interrupts::enable();
    crate::serial_println!(
        "[SFS5B3] post-AP cpus={} features={:?} queue-phys={:#x} request-phys={:#x} pages=5",
        mp.cpus().len(),
        d.features(),
        queue_phys,
        req_phys
    );
    let before = progress();
    let free_before = crate::PMM.lock().free_page_count();
    crate::serial_println!(
        "[SFS5B3] submit-before progress={:?} free={}",
        before,
        free_before
    );
    let outcome = unsafe { d.write_typed(63, &[0xb3; 512]) };
    crate::serial_println!(
        "[SFS5B3] submit-after outcome={:?} owner={:?} outstanding={:?} progress={:?}",
        outcome,
        d.ownership(),
        d.outstanding(),
        progress()
    );
    if outcome.completion == Completion::Completed {
        assert_eq!(unsafe { d.flush_typed() }.completion, Completion::Completed);
        let mut data = [0; 512];
        assert_eq!(
            unsafe { d.read_typed(63, &mut data) }.completion,
            Completion::Completed
        );
        assert_eq!(data, [0xb3; 512]);
        crate::serial_println!("[SFS5B3] NORMAL-READY progress={:?}", progress());
        loop {
            x86_64::instructions::hlt();
        }
    }
    assert_eq!(outcome.completion, Completion::Uncertain);
    assert_eq!(outcome.durability, Durability::Uncertain);
    assert!(d.outstanding().is_some());
    crate::serial_println!("[SFS5B3] reset-begin={:?}", unsafe {
        d.begin_terminal_reset()
    });
    for _ in 0..RESET_READ_BUDGET {
        if unsafe { d.poll_terminal_reset() }.state != ResetState::Waiting {
            break;
        }
    }
    let reset = d.reset_observation();
    let free_after = crate::PMM.lock().free_page_count();
    crate::serial_println!(
        "[SFS5B3] reset-after={:?} first-uncertain={:?} free-before={} free-after={} progress={:?}",
        reset,
        d.first_uncertain(),
        free_before,
        free_after,
        progress()
    );
    assert_eq!(reset.state, ResetState::VerifiedRetained);
    assert_eq!(d.first_uncertain(), Some(outcome));
    assert_eq!(unsafe { d.begin_terminal_reset() }, reset);
    assert_eq!(
        unsafe { d.write_typed(64, &[0xcc; 512]) }.completion,
        Completion::Unavailable
    );
    assert_eq!(d.reset_observation(), reset);
    assert!(d.outstanding().is_some());
    crate::serial_println!(
        "[SFS5B3] READY pages-pinned=5 released=0 new-epoch=none scheduler-ready={} progress={:?}",
        crate::sched::SCHEDULER_READY.load(Ordering::Acquire),
        progress()
    );
    loop {
        x86_64::instructions::hlt();
    }
}
