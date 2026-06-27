//! Pointer tuning profiles for different VM/input environments.
//!
//! Detection uses CPUID hypervisor vendor strings (leaf 0x40000000).
//! To force a profile at compile time, set PROFILE_OVERRIDE to Some(...).
//! Future work: read override from /etc/sunlightos/pointer_profile via VFS,
//! matching the env hint SUNLIGHT_POINTER_PROFILE=vmware_ps2|qemu_ps2|...

pub const FP_SHIFT: i32 = 16;
pub const FP_ONE: i32 = 1 << FP_SHIFT;

/// Set to Some(PointerProfile::VmwarePs2) etc. to bypass CPUID auto-detection.
pub const PROFILE_OVERRIDE: Option<PointerProfile> = None;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
pub enum PointerProfile {
    Generic,
    QemuPs2,
    VmwarePs2,
    RealHardwarePs2,
    /// TODO: VirtioInput delivers absolute position events — no relative accel.
    VirtioInput,
    /// TODO: AbsoluteTablet delivers absolute coordinates — bypass all accel.
    AbsoluteTablet,
}

impl PointerProfile {
    pub fn name(self) -> &'static str {
        match self {
            PointerProfile::Generic => "Generic",
            PointerProfile::QemuPs2 => "QemuPs2",
            PointerProfile::VmwarePs2 => "VmwarePs2",
            PointerProfile::RealHardwarePs2 => "RealHardwarePs2",
            PointerProfile::VirtioInput => "VirtioInput (TODO)",
            PointerProfile::AbsoluteTablet => "AbsoluteTablet (TODO)",
        }
    }

    pub fn tuning(self) -> PointerTuning {
        match self {
            // Balanced baseline — works acceptably on unknown environments.
            PointerProfile::Generic => PointerTuning {
                base_sensitivity_fp: FP_ONE * 5 / 4,   // 1.25×
                precise_max_magnitude: 2,
                low_speed_scale_fp: FP_ONE,             // 1.0×
                normal_max_magnitude: 8,
                medium_accel_fp: FP_ONE * 5 / 4,       // 1.25×
                max_accel_fp: FP_ONE * 7 / 4,          // 1.75×
                reversal_threshold: 3,
                clear_reversal_threshold: 5,
                click_stabilize_ms: 40,
                drag_threshold_px: 3,
            },
            // QEMU/KVM PS/2: emulated PS/2 feels sluggish — boost base + fast gain.
            PointerProfile::QemuPs2 => PointerTuning {
                base_sensitivity_fp: FP_ONE * 3 / 2,   // 1.5×
                precise_max_magnitude: 2,
                low_speed_scale_fp: FP_ONE,             // 1.0× — keep slow zone unscaled
                normal_max_magnitude: 8,
                medium_accel_fp: FP_ONE * 3 / 2,       // 1.5×
                max_accel_fp: FP_ONE * 2,              // 2.0×
                reversal_threshold: 3,
                clear_reversal_threshold: 4,
                click_stabilize_ms: 50,                 // higher latency — stabilize longer
                drag_threshold_px: 3,
            },
            // VMware PS/2: very low latency causes overshoot — pull back sensitivity
            // and cap fast-zone gain hard.
            PointerProfile::VmwarePs2 => PointerTuning {
                base_sensitivity_fp: FP_ONE,            // 1.0× — VMware already fast
                precise_max_magnitude: 3,               // wider slow zone for fine clicks
                low_speed_scale_fp: FP_ONE,             // 1.0×
                normal_max_magnitude: 7,
                medium_accel_fp: FP_ONE,               // 1.0× — no extra push in normal zone
                max_accel_fp: FP_ONE * 5 / 4,         // 1.25× — hard cap on fast sweeps
                reversal_threshold: 2,
                clear_reversal_threshold: 4,
                click_stabilize_ms: 30,
                drag_threshold_px: 2,
            },
            // Real PS/2 hardware: conservative baseline, slightly lower fast ceiling
            // than Generic to avoid overshoot on varied hardware mice.
            PointerProfile::RealHardwarePs2 => PointerTuning {
                base_sensitivity_fp: FP_ONE * 5 / 4,   // 1.25×
                precise_max_magnitude: 2,
                low_speed_scale_fp: FP_ONE,             // 1.0×
                normal_max_magnitude: 8,
                medium_accel_fp: FP_ONE * 5 / 4,       // 1.25×
                max_accel_fp: FP_ONE * 3 / 2,         // 1.5× — lower ceiling than Generic
                reversal_threshold: 3,
                clear_reversal_threshold: 5,
                click_stabilize_ms: 35,
                drag_threshold_px: 2,
            },
            // TODO: VirtioInput uses absolute position — acceleration is meaningless.
            PointerProfile::VirtioInput => PointerTuning {
                base_sensitivity_fp: FP_ONE,
                precise_max_magnitude: 1,
                low_speed_scale_fp: FP_ONE,
                normal_max_magnitude: 4,
                medium_accel_fp: FP_ONE,
                max_accel_fp: FP_ONE,
                reversal_threshold: 0,
                clear_reversal_threshold: 0,
                click_stabilize_ms: 0,
                drag_threshold_px: 0,
            },
            // TODO: AbsoluteTablet sends absolute coords — bypass all relative accel.
            PointerProfile::AbsoluteTablet => PointerTuning {
                base_sensitivity_fp: FP_ONE,
                precise_max_magnitude: 0,
                low_speed_scale_fp: FP_ONE,
                normal_max_magnitude: 0,
                medium_accel_fp: FP_ONE,
                max_accel_fp: FP_ONE,
                reversal_threshold: 0,
                clear_reversal_threshold: 0,
                click_stabilize_ms: 0,
                drag_threshold_px: 0,
            },
        }
    }
}

#[allow(dead_code)]
pub struct PointerTuning {
    /// Base sensitivity (Q16.16; FP_ONE = 1.0).
    pub base_sensitivity_fp: i32,
    /// |dx|+|dy| at or below which the precise/slow-zone gain applies.
    pub precise_max_magnitude: i32,
    /// Gain multiplier in the precise zone (applied on top of base_sensitivity_fp).
    pub low_speed_scale_fp: i32,
    /// |dx|+|dy| at or below which the normal-zone gain applies (above precise).
    pub normal_max_magnitude: i32,
    /// Gain multiplier in the normal zone.
    pub medium_accel_fp: i32,
    /// Gain multiplier when magnitude > normal_max_magnitude (fast zone).
    pub max_accel_fp: i32,
    /// Consecutive opposing-direction packets before reversal damping kicks in (future).
    pub reversal_threshold: i32,
    /// Consecutive same-direction packets required to exit reversal damping (future).
    pub clear_reversal_threshold: i32,
    /// ms to suppress sub-pixel motion after a button event (future).
    pub click_stabilize_ms: u64,
    /// Pixel distance below which post-click motion is suppressed (future).
    pub drag_threshold_px: i32,
}

/// Resolve the active profile: compile-time override first, then CPUID detection.
pub fn select_profile() -> PointerProfile {
    if let Some(p) = PROFILE_OVERRIDE {
        return p;
    }
    detect_via_cpuid()
}

/// Execute CPUID for `leaf`, returning (eax, ebx, ecx, edx).
/// rbx is callee-saved by the ABI and used internally by LLVM, so we
/// push/pop it manually and route the result through a scratch register.
fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let (a, b, c, d): (u32, u32, u32, u32);
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {tmp:e}, ebx",
            "pop rbx",
            inout("eax") leaf => a,
            tmp = out(reg) b,
            out("ecx") c,
            out("edx") d,
        );
    }
    (a, b, c, d)
}

fn detect_via_cpuid() -> PointerProfile {
    // CPUID.1:ECX bit 31 — hypervisor present
    let (_, _, ecx1, _) = cpuid(1);
    if ecx1 & (1 << 31) == 0 {
        return PointerProfile::RealHardwarePs2;
    }

    // Hypervisor vendor string — CPUID leaf 0x40000000 → EBX:ECX:EDX
    let (_, ebx, ecx, edx) = cpuid(0x4000_0000);

    // "VMwareVMware" (EBX=0x61774D56 ECX=0x4D566572 EDX=0x65726177)
    if ebx == 0x6177_4D56 && ecx == 0x4D56_6572 && edx == 0x6572_6177 {
        return PointerProfile::VmwarePs2;
    }
    // "KVMKVMKVM\0\0\0" — QEMU + KVM acceleration
    if ebx == 0x4B4D_564B && ecx == 0x564B_4D56 && edx == 0x0000_004D {
        return PointerProfile::QemuPs2;
    }
    // "TCGTCGTCGTCG" — QEMU/TCG (no KVM)
    if ebx == 0x5447_4354 && ecx == 0x5447_4354 && edx == 0x5447_4354 {
        return PointerProfile::QemuPs2;
    }

    PointerProfile::Generic
}
