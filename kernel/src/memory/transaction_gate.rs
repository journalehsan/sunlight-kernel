use super::{pmm::PhysicalMemoryManager, retirement::OWNERS};
use crate::process::mmap::{
    sys_mmap, MmapError, MAP_ANONYMOUS, MAP_FIXED, MAP_PRIVATE, PROT_READ, PROT_WRITE,
};
use crate::sched::Scheduler;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use x86_64::{
    structures::paging::{Page, Size4KiB},
    PhysAddr, VirtAddr,
};

static INSTALL: AtomicUsize = AtomicUsize::new(usize::MAX);
static RESTORE: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
pub(crate) fn fail_install(index: usize) -> bool {
    INSTALL.load(Ordering::Acquire) == index
}
pub(crate) fn fail_restore(index: usize) -> bool {
    index == 0 && RESTORE.load(Ordering::Acquire)
}
pub(crate) fn fault_armed() -> bool {
    ARMED.load(Ordering::Acquire)
}

fn map(
    sched: &mut Scheduler,
    pmm: &mut PhysicalMemoryManager,
    address: u64,
    count: u64,
) -> Result<u64, MmapError> {
    sys_mmap(
        address,
        count * 4096,
        PROT_READ | PROT_WRITE,
        MAP_FIXED | MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
        pmm,
        sched,
    )
}
fn frame(sched: &Scheduler, address: u64, hhdm: VirtAddr) -> PhysAddr {
    unsafe {
        sched
            .current_process()
            .address_space
            .lookup_entry(
                Page::<Size4KiB>::from_start_address(VirtAddr::new(address)).unwrap(),
                hhdm,
            )
            .unwrap()
            .0
    }
}
fn unchanged(sched: &Scheduler, address: u64, original: &[PhysAddr], hhdm: VirtAddr) {
    for (index, old) in original.iter().enumerate() {
        assert_eq!(frame(sched, address + index as u64 * 4096, hhdm), *old);
        assert_eq!(
            unsafe { (hhdm + old.as_u64()).as_ptr::<u64>().read_volatile() },
            0x51525354
        );
    }
    assert!(unsafe {
        sched
            .current_process()
            .address_space
            .validate_ledger_ptes(hhdm)
    });
}

pub(crate) fn run(pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) {
    const BASE: u64 = 0x0000_0030_001f_f000;
    let baseline = pmm.free_page_count();
    let process =
        unsafe { crate::process::Process::new(0xB2D5, 0, "map-fixed-transaction", pmm, hhdm) };
    let mut sched = Scheduler::new();
    sched.processes.push(process);
    sched.current_process_mut().system_allocation = true;
    let identity = sched.current_process().address_space.identity();
    assert_eq!(map(&mut sched, pmm, BASE, 4), Ok(BASE));
    let original: [PhysAddr; 4] =
        core::array::from_fn(|index| frame(&sched, BASE + index as u64 * 4096, hhdm));
    for old in original {
        unsafe {
            (hhdm + old.as_u64())
                .as_mut_ptr::<u64>()
                .write_volatile(0x51525354);
        }
    }
    crate::serial_println!(
        "[MT-CASE] id=MT-11 phase=prepared root={} vm={} start={} pages=4",
        identity.pml4_phys,
        identity.generation,
        BASE
    );

    let reservations: [Option<super::retirement::Token>; 8] = core::array::from_fn(|index| {
        OWNERS
            .lock()
            .reserve((index as u64 + 1) * 4096, u64::MAX - index as u64)
    });
    assert!(reservations.iter().all(Option::is_some));
    let before = pmm.free_page_count();
    assert_eq!(map(&mut sched, pmm, BASE, 4), Err(MmapError::NoMemory));
    unchanged(&sched, BASE, &original, hhdm);
    assert_eq!(before, pmm.free_page_count());
    for token in reservations.iter().flatten() {
        assert!(OWNERS.lock().finish(token));
    }
    assert_eq!(map(&mut sched, pmm, BASE, 33), Err(MmapError::NoMemory));
    unchanged(&sched, BASE, &original, hhdm);
    crate::serial_println!(
        "[MT-CASE] id=MT-2 status=PASS root={} vm={} journal-full=1 request-capacity=1 unchanged=1",
        identity.pml4_phys,
        identity.generation
    );

    pmm.fail_test_allocations_after(1);
    assert_eq!(map(&mut sched, pmm, BASE, 4), Err(MmapError::NoMemory));
    pmm.clear_test_allocation_failure();
    unchanged(&sched, BASE, &original, hhdm);
    assert_eq!(before, pmm.free_page_count());
    crate::serial_println!(
        "[MT-CASE] id=MT-3 status=PASS root={} vm={} unchanged=1",
        identity.pml4_phys,
        identity.generation
    );

    for _ in 0..2 {
        INSTALL.store(1, Ordering::Release);
        if option_env!("KFP2A5_RESTORE_FAIL") == Some("1") {
            RESTORE.store(true, Ordering::Release);
        }
        assert_eq!(map(&mut sched, pmm, BASE, 4), Err(MmapError::NoMemory));
        INSTALL.store(usize::MAX, Ordering::Release);
        unchanged(&sched, BASE, &original, hhdm);
        assert_eq!(before, pmm.free_page_count());
        assert!(!OWNERS
            .lock()
            .owns_vm(identity.pml4_phys, identity.generation));
    }
    crate::serial_println!(
        "[MT-CASE] id=MT-4 status=PASS root={} vm={} rollback=exact",
        identity.pml4_phys,
        identity.generation
    );
    crate::serial_println!(
        "[MT-CASE] id=MT-10 status=PASS root={} vm={} cancellations=2",
        identity.pml4_phys,
        identity.generation
    );

    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(BASE)).unwrap();
    let block = unsafe {
        super::swap::swap_out_page(
            &mut sched.current_process_mut().address_space,
            page,
            original[0],
            hhdm,
            pmm,
        )
    }
    .unwrap();
    assert!(super::zram::block_exists(block));
    assert_eq!(map(&mut sched, pmm, BASE, 4), Ok(BASE));
    assert!(!super::zram::block_exists(block));
    crate::serial_println!(
        "[MT-CASE] id=MT-12 status=PASS root={} vm={} swap={} exists=0 shared-quota=not-applicable",
        identity.pml4_phys,
        identity.generation,
        block
    );
    for index in 0..4 {
        assert_eq!(
            unsafe {
                (hhdm + frame(&sched, BASE + index * 4096, hhdm).as_u64())
                    .as_ptr::<u64>()
                    .read_volatile()
            },
            0
        );
    }
    assert!(unsafe {
        sched
            .current_process()
            .address_space
            .validate_ledger_ptes(hhdm)
    });
    crate::serial_println!(
        "[MT-CASE] id=MT-11 status=PASS root={} vm={} pages=4",
        identity.pml4_phys,
        identity.generation
    );

    let online = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
    let targets = ((1u64 << online) - 1) & !(1u64 << crate::sched::current_cpu_id());
    unsafe {
        sched.current_process().address_space.activate();
    }
    super::tlb::test_activate_and_read(identity, BASE, targets);
    ARMED.store(true, Ordering::Release);
    assert_eq!(map(&mut sched, pmm, BASE, 4), Ok(BASE));
    ARMED.store(false, Ordering::Release);
    super::tlb::test_read(identity, BASE, targets);
    for cpu in 0..online {
        if targets & (1 << cpu) != 0 {
            assert_eq!(super::tlb::test_result(cpu), 0);
        }
    }
    super::tlb::test_leave(targets);
    unsafe {
        super::tlb::activate_kernel_root();
    }
    assert!(!OWNERS
        .lock()
        .owns_vm(identity.pml4_phys, identity.generation));
    crate::serial_println!(
        "[MT-CASE] id=MT-1 status=PASS root={} vm={} targets={}",
        identity.pml4_phys,
        identity.generation,
        targets
    );
    unsafe {
        sched
            .current_process_mut()
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(baseline, pmm.free_page_count());
    crate::serial_println!(
        "[MT-GATE] complete baseline-restored=1 concurrent-exit=NOT_PROVEN overlap=NOT_PROVEN"
    );
}
