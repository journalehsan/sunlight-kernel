//! Intel Digital Thermal Sensor (DTS) backend — read-only.
//!
//! Safety policy:
//! - GenuineIntel + CPUID.06H DTS + strict family/model allowlist before any RDMSR.
//! - No general userspace MSR API.
//! - No WRMSR of thermal/power registers.
//! - No recoverable #GP path exists; unknown models never probe MSRs.
//! - Per-core IA32_THERM_STATUS is sampled on the owning CPU from the timer ISR
//!   (each core updates its own slot) — never by remote BSP-only RDMSR labeled
//!   as another core.
//!
//! Spec references:
//! - Intel 64 and IA-32 Architectures Software Developer's Manual
//!   Volume 3: System Programming Guide — thermal management chapter
//! - Volume 4: Model-Specific Registers
//! - CPUID leaf 06H thermal/power management features
//!
//! ## Allowlisted models (family 6) and MSR evidence
//!
//! | Display model | Microarchitecture | MSRs read | Official evidence |
//! |---------------|-------------------|-----------|-------------------|
//! | 0x3C | Haswell (client) | 0x19C, 0x1A2, 0x1B1* | SDM Vol. 4 model 06_3CH; Vol. 3 DTS; CPUID.06H |
//! | 0x45 | Haswell ULT | 0x19C, 0x1A2, 0x1B1* | SDM Vol. 4 model 06_45H; Vol. 3 DTS; CPUID.06H |
//! | 0x46 | Haswell H | 0x19C, 0x1A2, 0x1B1* | SDM Vol. 4 model 06_46H; Vol. 3 DTS; CPUID.06H |
//!
//! \* Package MSR 0x1B1 only when CPUID.06H:EAX[6]=1.
//!
//! **Not allowlisted:** 0x3E (Ivy Bridge-E/EN/EP) — wrong generation for T440p
//! and not independently required for Phase 1 validation.
//!
//! Gate order: GenuineIntel → CPUID.06H DTS bit → model allowlist → then RDMSR.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use spin::Mutex;
use sunlight_sensors::intel_dts::{
    decode_family_model, feature_gate, is_genuine_intel, temperature_from_therm_status,
    DtsProbeResult, MSR_IA32_PACKAGE_THERM_STATUS, MSR_IA32_TEMPERATURE_TARGET,
    MSR_IA32_THERM_STATUS,
};
use sunlight_sensors::{
    SensorClass, SensorId, SensorReading, SensorScope, SensorSource, SensorStatus, SensorUnit,
    MAX_SENSORS,
};

const MAX_LOGICAL: usize = 64;
/// Sample local core DTS at most every N timer ticks (~100 Hz → ~1 Hz when N=100).
const SAMPLE_EVERY_TICKS: u64 = 100;

#[derive(Clone, Copy)]
struct CoreSlot {
    reading_mc: i32,
    status: SensorStatus,
    mono_ms: u64,
    /// Initial APIC id observed when this slot was last written.
    apic_id: u8,
}

impl CoreSlot {
    const fn empty() -> Self {
        Self {
            reading_mc: 0,
            status: SensorStatus::Unavailable,
            mono_ms: 0,
            apic_id: 0xFF,
        }
    }
}

struct ThermalHwState {
    enabled: bool,
    package_ok: bool,
    tj_max_c: u8,
    family: u32,
    model: u32,
    package: CoreSlot,
    /// Indexed by logical CPU id (scheduler cpu index).
    cores: [CoreSlot; MAX_LOGICAL],
    /// Physical core mapping: if topology unknown, treat as logical.
    physical_core_of: [u8; MAX_LOGICAL],
    logical_count: u8,
    topology_reliable: bool,
    last_package_sample_ms: u64,
}

impl ThermalHwState {
    const fn new() -> Self {
        Self {
            enabled: false,
            package_ok: false,
            tj_max_c: 0,
            family: 0,
            model: 0,
            package: CoreSlot::empty(),
            cores: [CoreSlot::empty(); MAX_LOGICAL],
            physical_core_of: [0xFF; MAX_LOGICAL],
            logical_count: 0,
            topology_reliable: false,
            last_package_sample_ms: 0,
        }
    }
}

static STATE: Mutex<ThermalHwState> = Mutex::new(ThermalHwState::new());
static INIT_DONE: AtomicBool = AtomicBool::new(false);
static LOCAL_TICK: [AtomicU64; MAX_LOGICAL] = [const { AtomicU64::new(0) }; MAX_LOGICAL];
static BACKEND_STATUS: AtomicU8 = AtomicU8::new(0); // 0=uninit 1=ready 2=unsupported

/// Probe CPUID and, only if allowlisted, read temperature target once.
pub fn init() {
    if INIT_DONE.swap(true, Ordering::SeqCst) {
        return;
    }

    let vendor = core::arch::x86_64::__cpuid(0);
    let intel = is_genuine_intel(vendor.ebx, vendor.ecx, vendor.edx);
    let max_leaf = vendor.eax;

    let feat = core::arch::x86_64::__cpuid(1);
    let (family, model, _stepping) = decode_family_model(feat.eax);

    let thermal_eax = if max_leaf >= 6 {
        core::arch::x86_64::__cpuid(6).eax
    } else {
        0
    };

    // Do not RDMSR until the pure gate would accept a target.
    // First gate without target to avoid speculative RDMSR on bad models.
    let pre = feature_gate(intel, max_leaf, thermal_eax, family, model, Some(100 << 16));
    if !matches!(pre, DtsProbeResult::Ready { .. }) {
        crate::serial_println!(
            "[THERMAL-HW] Intel DTS unsupported (intel={} family={} model={:#x} dts={})",
            intel,
            family,
            model,
            thermal_eax & 1
        );
        BACKEND_STATUS.store(2, Ordering::Release);
        return;
    }

    // Allowlisted: read temperature target once (architectural on these models).
    let tt = unsafe { read_msr(MSR_IA32_TEMPERATURE_TARGET) };
    let gate = feature_gate(intel, max_leaf, thermal_eax, family, model, Some(tt));
    let DtsProbeResult::Ready { package, tj_max_c } = gate else {
        crate::serial_println!("[THERMAL-HW] temperature target rejected");
        BACKEND_STATUS.store(2, Ordering::Release);
        return;
    };

    let mut st = STATE.lock();
    st.enabled = true;
    st.package_ok = package;
    st.tj_max_c = tj_max_c;
    st.family = family;
    st.model = model;
    st.logical_count = 1; // refined when SMP known
                          // Topology leaf 0xB not fully wired — label as logical-CPU until reliable.
    st.topology_reliable = false;
    for i in 0..MAX_LOGICAL {
        st.physical_core_of[i] = i as u8;
    }

    crate::serial_println!(
        "[THERMAL-HW] Intel DTS ready family=6 model={:#x} tj_max={}C package={}",
        model,
        tj_max_c,
        package
    );
    BACKEND_STATUS.store(1, Ordering::Release);
}

/// Update logical CPU count after SMP bring-up.
pub fn set_logical_cpu_count(n: usize) {
    let mut st = STATE.lock();
    st.logical_count = n.min(MAX_LOGICAL) as u8;
}

/// Called from each CPU's timer path (~100 Hz). Samples at ~1 Hz per core.
/// Must not allocate or take long-held locks beyond the short STATE lock.
pub fn on_timer_tick(cpu_id: usize) {
    if BACKEND_STATUS.load(Ordering::Acquire) != 1 {
        return;
    }
    if cpu_id >= MAX_LOGICAL {
        return;
    }
    let ticks = LOCAL_TICK[cpu_id].fetch_add(1, Ordering::Relaxed) + 1;
    if ticks % SAMPLE_EVERY_TICKS != 0 {
        return;
    }

    let mono = crate::timekeeping::monotonic_ms();
    let apic = (core::arch::x86_64::__cpuid(1).ebx >> 24) as u8;

    // Per-core THERM_STATUS on *this* CPU only.
    let therm = unsafe { read_msr(MSR_IA32_THERM_STATUS) };
    let mut st = STATE.lock();
    if !st.enabled {
        return;
    }
    let tj = st.tj_max_c;
    let status = match temperature_from_therm_status(therm, tj) {
        Ok(mc) => {
            st.cores[cpu_id] = CoreSlot {
                reading_mc: mc,
                status: SensorStatus::Valid,
                mono_ms: mono,
                apic_id: apic,
            };
            SensorStatus::Valid
        }
        Err(s) => {
            st.cores[cpu_id].status = s;
            st.cores[cpu_id].mono_ms = mono;
            st.cores[cpu_id].apic_id = apic;
            s
        }
    };
    let _ = status;

    // Package thermal: sample from BSP primarily to avoid redundant RDMSR storms.
    if st.package_ok && cpu_id == 0 {
        if mono.saturating_sub(st.last_package_sample_ms) >= 1000 {
            let pkg = unsafe { read_msr(MSR_IA32_PACKAGE_THERM_STATUS) };
            st.package = match temperature_from_therm_status(pkg, tj) {
                Ok(mc) => CoreSlot {
                    reading_mc: mc,
                    status: SensorStatus::Valid,
                    mono_ms: mono,
                    apic_id: apic,
                },
                Err(s) => CoreSlot {
                    reading_mc: 0,
                    status: s,
                    mono_ms: mono,
                    apic_id: apic,
                },
            };
            st.last_package_sample_ms = mono;
        }
    }
}

/// Whether DTS backend is active.
pub fn is_ready() -> bool {
    BACKEND_STATUS.load(Ordering::Acquire) == 1
}

pub fn is_unsupported() -> bool {
    BACKEND_STATUS.load(Ordering::Acquire) == 2
}

pub fn tj_max_c() -> Option<u8> {
    let st = STATE.lock();
    if st.enabled {
        Some(st.tj_max_c)
    } else {
        None
    }
}

/// Fill a user-facing sensor list (descriptors + readings). Returns count.
pub fn snapshot_sensors(out: &mut [SensorExport], now_ms: u64) -> usize {
    let st = STATE.lock();
    if !st.enabled {
        return 0;
    }
    let mut n = 0usize;

    // Package sensor first when available.
    if st.package_ok && n < out.len() {
        let r = export_slot(
            SensorId::new(SensorClass::CpuPackage, 0),
            SensorClass::CpuPackage,
            SensorScope::Package,
            0, // label: package
            &st.package,
            now_ms,
        );
        out[n] = r;
        n += 1;
    }

    // Per-logical or per-physical core sensors.
    let logical = st.logical_count as usize;
    if st.topology_reliable {
        // Emit unique physical cores only.
        let mut seen = [false; MAX_LOGICAL];
        for cpu in 0..logical {
            let phys = st.physical_core_of[cpu] as usize;
            if phys >= MAX_LOGICAL || seen[phys] {
                continue;
            }
            seen[phys] = true;
            if n >= out.len() {
                break;
            }
            out[n] = export_slot(
                SensorId::new(SensorClass::CpuCore, phys as u16),
                SensorClass::CpuCore,
                SensorScope::Core,
                phys as u8,
                &st.cores[cpu],
                now_ms,
            );
            n += 1;
        }
    } else {
        for cpu in 0..logical {
            if n >= out.len() {
                break;
            }
            out[n] = export_slot(
                SensorId::new(SensorClass::CpuLogical, cpu as u16),
                SensorClass::CpuLogical,
                SensorScope::LogicalCpu,
                cpu as u8,
                &st.cores[cpu],
                now_ms,
            );
            n += 1;
        }
    }
    n.min(MAX_SENSORS)
}

fn export_slot(
    id: SensorId,
    class: SensorClass,
    scope: SensorScope,
    location: u8,
    slot: &CoreSlot,
    now_ms: u64,
) -> SensorExport {
    let mut reading = SensorReading {
        sensor_id: id,
        value: slot.reading_mc,
        monotonic_ms: slot.mono_ms,
        status: slot.status,
    };
    if reading.status == SensorStatus::Valid {
        reading = reading.mark_stale_if_older_than(now_ms, 5_000);
    }
    SensorExport {
        id: id.0,
        class: class as u8,
        label: location,
        location,
        unit: SensorUnit::MilliCelsius as u8,
        scope: scope as u8,
        source: SensorSource::IntelDts as u8,
        read_only: 1,
        status: reading.status as u8,
        value: if reading.status == SensorStatus::Valid || reading.status == SensorStatus::Stale {
            reading.value
        } else {
            0
        },
        mono_ms: reading.monotonic_ms,
        reserved: 0,
    }
}

/// Fixed C-layout export for syscalls (no heap).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SensorExport {
    pub id: u32,
    pub class: u8,
    pub label: u8,
    pub location: u8,
    pub unit: u8,
    pub scope: u8,
    pub source: u8,
    pub read_only: u8,
    pub status: u8,
    pub value: i32,
    pub mono_ms: u64,
    pub reserved: u32,
}

impl SensorExport {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            class: 0,
            label: 0,
            location: 0,
            unit: 0,
            scope: 0,
            source: 0,
            read_only: 1,
            status: SensorStatus::Unavailable as u8,
            value: 0,
            mono_ms: 0,
            reserved: 0,
        }
    }
}

/// Maximum valid CPU temperature across sensors, if any Valid reading exists.
pub fn max_valid_temp_mc(now_ms: u64) -> Option<i32> {
    let mut buf = [SensorExport::empty(); MAX_SENSORS];
    let n = snapshot_sensors(&mut buf, now_ms);
    let mut max = None;
    for s in buf.iter().take(n) {
        if s.status == SensorStatus::Valid as u8 {
            max = Some(max.map_or(s.value, |m: i32| m.max(s.value)));
        }
    }
    max
}

/// Narrow MSR read — kernel internal only. Callers must have passed the allowlist.
unsafe fn read_msr(index: u32) -> u64 {
    // Only the three thermal MSRs are ever used from this module.
    debug_assert!(
        index == MSR_IA32_THERM_STATUS
            || index == MSR_IA32_TEMPERATURE_TARGET
            || index == MSR_IA32_PACKAGE_THERM_STATUS
    );
    x86_64::registers::model_specific::Msr::new(index).read()
}

// Explicitly no write function for thermal MSRs exists in this module.
