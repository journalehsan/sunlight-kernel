//! Boot-published topology plus monotonic startup readiness, without hotplug.
use super::cpu_topology::{CpuTopology, MAX_CPUS};
use core::sync::atomic::{AtomicU64, Ordering};

static TOPOLOGY: spin::Once<CpuTopology> = spin::Once::new();
static ONLINE: AtomicU64 = AtomicU64::new(0);

pub fn topology() -> &'static CpuTopology {
    TOPOLOGY
        .get()
        .expect("CPU topology must precede per-CPU initialization")
}

pub fn validate_controller() {
    let base = unsafe { x86_64::registers::model_specific::Msr::new(0x1b).read() };
    assert_ne!(base & (1 << 11), 0, "LAPIC disabled");
    assert_eq!(base & (1 << 10), 0, "x2APIC mode unsupported");
}

pub fn initialize() {
    validate_controller();
    assert!(TOPOLOGY.get().is_none(), "CPU topology already published");
    let mut ids = [0; MAX_CPUS];
    let (count, bsp) = if let Some(mp) = crate::MP_REQ.response() {
        let cpus = mp.cpus();
        assert!(
            !cpus.is_empty() && cpus.len() <= MAX_CPUS,
            "unsupported CPU count"
        );
        for (slot, cpu) in cpus.iter().enumerate() {
            ids[slot] = cpu.lapic_id;
        }
        (cpus.len(), mp.bsp_lapic_id)
    } else {
        ids[0] = initial_apic_id();
        (1, ids[0])
    };
    assert_eq!(initial_apic_id(), bsp, "Limine BSP identity mismatch");
    let map = CpuTopology::new(&ids[..count], bsp).expect("unsupported CPU topology");
    TOPOLOGY.call_once(|| map);
    mark_online(0);
    for logical in 0..count {
        crate::serial_println!(
            "[CPU-ID] logical={} apic={} bsp={}",
            logical,
            topology().apic_id(logical).unwrap(),
            logical == 0
        );
    }
}

fn initial_apic_id() -> u32 {
    core::arch::x86_64::__cpuid(1).ebx >> 24
}

pub fn current() -> usize {
    topology()
        .logical_id(initial_apic_id())
        .expect("unmapped current APIC identity")
}

pub fn mark_online(logical: usize) {
    assert!(topology().apic_id(logical).is_some(), "invalid online CPU");
    assert_eq!(
        ONLINE.fetch_or(1u64 << logical, Ordering::Release) & (1u64 << logical),
        0,
        "CPU already online"
    );
}

pub fn ipi_destination(logical: usize) -> u32 {
    topology()
        .destination(logical, ONLINE.load(Ordering::Acquire))
        .expect("invalid/offline IPI destination")
}
