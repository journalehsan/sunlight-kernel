//! Generic read-only hardware sensor model and Intel DTS pure helpers.
//!
//! Temperature unit: signed milli-degrees Celsius (`i32`).
//! Missing/unavailable sensors must never become 0°C.

#![no_std]

#[cfg(test)]
extern crate std;

/// Maximum sensors in a bounded enumeration snapshot.
pub const MAX_SENSORS: usize = 32;

/// Temperature in milli-degrees Celsius.
pub type MilliC = i32;

/// Stable sensor identifier for the duration of a boot (not a pointer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SensorId(pub u32);

impl SensorId {
    pub const fn new(class: SensorClass, index: u16) -> Self {
        Self(((class as u32) << 16) | (index as u32))
    }

    pub const fn class(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn index(self) -> u16 {
        self.0 as u16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SensorClass {
    CpuPackage = 1,
    CpuCore = 2,
    CpuLogical = 3,
    Gpu = 4,
    Nvme = 5,
    Battery = 6,
    Board = 7,
    FanRpm = 8,
    Other = 255,
}

impl SensorClass {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::CpuPackage,
            2 => Self::CpuCore,
            3 => Self::CpuLogical,
            4 => Self::Gpu,
            5 => Self::Nvme,
            6 => Self::Battery,
            7 => Self::Board,
            8 => Self::FanRpm,
            _ => Self::Other,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CpuPackage => "cpu_package",
            Self::CpuCore => "cpu_core",
            Self::CpuLogical => "cpu_logical",
            Self::Gpu => "gpu",
            Self::Nvme => "nvme",
            Self::Battery => "battery",
            Self::Board => "board",
            Self::FanRpm => "fan_rpm",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SensorUnit {
    MilliCelsius = 1,
    Rpm = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SensorScope {
    Package = 1,
    Core = 2,
    LogicalCpu = 3,
    Device = 4,
    System = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SensorSource {
    Unknown = 0,
    IntelDts = 1,
    Acpi = 2,
    Ec = 3,
    Mock = 4,
}

impl SensorSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::IntelDts => "Intel DTS",
            Self::Acpi => "ACPI",
            Self::Ec => "EC",
            Self::Mock => "Mock",
        }
    }

    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::IntelDts,
            2 => Self::Acpi,
            3 => Self::Ec,
            4 => Self::Mock,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SensorStatus {
    Valid = 1,
    Unavailable = 2,
    Unsupported = 3,
    Stale = 4,
    Invalid = 5,
    HardwareError = 6,
}

impl SensorStatus {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Valid,
            2 => Self::Unavailable,
            3 => Self::Unsupported,
            4 => Self::Stale,
            5 => Self::Invalid,
            6 => Self::HardwareError,
            _ => Self::Unavailable,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "Valid",
            Self::Unavailable => "Unavailable",
            Self::Unsupported => "Unsupported",
            Self::Stale => "Stale",
            Self::Invalid => "Invalid",
            Self::HardwareError => "HardwareError",
        }
    }

    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Descriptor for a read-only sensor. Stable for one boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorDescriptor {
    pub id: SensorId,
    pub class: SensorClass,
    pub label_tag: u8,
    pub location_tag: u8,
    pub unit: SensorUnit,
    pub scope: SensorScope,
    pub source: SensorSource,
    pub read_only: bool,
}

/// One sensor sample. `value` is meaningful only when `status == Valid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorReading {
    pub sensor_id: SensorId,
    pub value: i32,
    pub monotonic_ms: u64,
    pub status: SensorStatus,
}

impl SensorReading {
    pub const fn unavailable(id: SensorId) -> Self {
        Self {
            sensor_id: id,
            value: 0, // must not be interpreted when status != Valid
            monotonic_ms: 0,
            status: SensorStatus::Unavailable,
        }
    }

    pub const fn unsupported(id: SensorId) -> Self {
        Self {
            sensor_id: id,
            value: 0,
            monotonic_ms: 0,
            status: SensorStatus::Unsupported,
        }
    }

    pub const fn hardware_error(id: SensorId) -> Self {
        Self {
            sensor_id: id,
            value: 0,
            monotonic_ms: 0,
            status: SensorStatus::HardwareError,
        }
    }

    pub fn valid(id: SensorId, value: i32, monotonic_ms: u64) -> Result<Self, SensorModelError> {
        if !is_sane_milli_c(value) {
            return Err(SensorModelError::OutOfRange);
        }
        Ok(Self {
            sensor_id: id,
            value,
            monotonic_ms,
            status: SensorStatus::Valid,
        })
    }

    /// Extract temperature only when Valid — never coerce Unavailable to 0°C.
    pub fn temp_milli_c(self) -> Option<MilliC> {
        if self.status == SensorStatus::Valid {
            Some(self.value)
        } else {
            None
        }
    }

    pub fn mark_stale_if_older_than(self, now_ms: u64, max_age_ms: u64) -> Self {
        if self.status != SensorStatus::Valid {
            return self;
        }
        if self.monotonic_ms > now_ms || now_ms.saturating_sub(self.monotonic_ms) > max_age_ms {
            let mut s = self;
            s.status = SensorStatus::Stale;
            // Keep last value for diagnostics but consumers must check status.
            s
        } else {
            self
        }
    }

    pub fn validate_combination(self) -> Result<(), SensorModelError> {
        match self.status {
            SensorStatus::Valid => {
                if self.monotonic_ms == 0 {
                    return Err(SensorModelError::InvalidTimestamp);
                }
                if !is_sane_milli_c(self.value) {
                    return Err(SensorModelError::OutOfRange);
                }
                Ok(())
            }
            SensorStatus::Stale => {
                // Stale may retain a previous value; timestamp must be non-zero.
                if self.monotonic_ms == 0 {
                    return Err(SensorModelError::InvalidTimestamp);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorModelError {
    OutOfRange,
    InvalidTimestamp,
    BoundExceeded,
}

pub const TEMP_MIN_SANE_MC: MilliC = -40_000;
pub const TEMP_MAX_SANE_MC: MilliC = 125_000;
pub const SENSOR_STALE_MS: u64 = 5_000;

pub fn is_sane_milli_c(v: MilliC) -> bool {
    v >= TEMP_MIN_SANE_MC && v <= TEMP_MAX_SANE_MC
}

/// Coherent bounded snapshot for thermald.
#[derive(Debug, Clone, Copy)]
pub struct SensorSnapshot {
    pub count: usize,
    pub readings: [SensorReading; MAX_SENSORS],
    pub captured_mono_ms: u64,
}

impl SensorSnapshot {
    pub const fn empty(now: u64) -> Self {
        Self {
            count: 0,
            readings: [SensorReading::unavailable(SensorId(0)); MAX_SENSORS],
            captured_mono_ms: now,
        }
    }

    pub fn push(&mut self, r: SensorReading) -> Result<(), SensorModelError> {
        r.validate_combination()?;
        if self.count >= MAX_SENSORS {
            return Err(SensorModelError::BoundExceeded);
        }
        self.readings[self.count] = r;
        self.count += 1;
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &SensorReading> {
        self.readings[..self.count].iter()
    }

    pub fn max_valid_temp_mc(&self) -> Option<MilliC> {
        self.iter().filter_map(|r| r.temp_milli_c()).max()
    }
}

// ─── Intel DTS pure helpers (no RDMSR here) ─────────────────────────────────

/// Intel architectural thermal MSRs (IA-32 SDM Vol. 3 / Vol. 4).
/// Read-only use only — no WRMSR paths exist in this crate.
pub mod intel_dts {
    use super::*;

    /// IA32_THERM_STATUS — per-core digital thermal sensor status.
    pub const MSR_IA32_THERM_STATUS: u32 = 0x19C;
    /// IA32_TEMPERATURE_TARGET — temperature target / TjMax field.
    pub const MSR_IA32_TEMPERATURE_TARGET: u32 = 0x1A2;
    /// IA32_PACKAGE_THERM_STATUS — package digital thermal sensor.
    pub const MSR_IA32_PACKAGE_THERM_STATUS: u32 = 0x1B1;

    /// CPUID.01H leaf for vendor/family/model.
    pub const CPUID_LEAF_FEATURES: u32 = 0x1;
    /// CPUID.06H thermal and power management leaf.
    pub const CPUID_LEAF_THERMAL: u32 = 0x6;

    /// CPUID.06H:EAX bit 0 — Digital temperature sensor present.
    pub const CPUID_DTS_BIT: u32 = 1 << 0;
    /// CPUID.06H:EAX bit 6 — Package thermal management.
    pub const CPUID_PTM_BIT: u32 = 1 << 6;

    /// THERM_STATUS bit 31 — Reading valid.
    pub const THERM_STATUS_VALID: u64 = 1 << 31;
    /// THERM_STATUS bits 22:16 — Digital readout (delta from TCC activation).
    pub const THERM_DIGITAL_SHIFT: u32 = 16;
    pub const THERM_DIGITAL_MASK: u64 = 0x7F;
    /// TEMPERATURE_TARGET bits 23:16 — temperature target (TjMax °C).
    pub const TEMP_TARGET_SHIFT: u32 = 16;
    pub const TEMP_TARGET_MASK: u64 = 0xFF;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CpuIdentity {
        pub vendor_intel: bool,
        pub family: u32,
        pub model: u32,
        pub stepping: u32,
    }

    /// Decode display family/model from CPUID.01H:EAX (Intel SDM Vol. 2A).
    pub fn decode_family_model(eax: u32) -> (u32, u32, u32) {
        let stepping = eax & 0xF;
        let model = (eax >> 4) & 0xF;
        let family = (eax >> 8) & 0xF;
        let ext_model = (eax >> 16) & 0xF;
        let ext_family = (eax >> 20) & 0xFF;
        let display_family = if family == 0xF {
            family + ext_family
        } else {
            family
        };
        let display_model = if family == 0x6 || family == 0xF {
            (ext_model << 4) | model
        } else {
            model
        };
        (display_family, display_model, stepping)
    }

    pub fn is_genuine_intel(ebx: u32, ecx: u32, edx: u32) -> bool {
        // "Genu" "ineI" "ntel" in EBX/EDX/ECX order (CPUID.0).
        ebx == u32::from_le_bytes(*b"Genu")
            && edx == u32::from_le_bytes(*b"ineI")
            && ecx == u32::from_le_bytes(*b"ntel")
    }

    /// Strict allowlist of Intel family/model pairs permitted to RDMSR thermal registers.
    ///
    /// Microarchitecture names (Intel product documentation):
    /// - `0x3C`, `0x45`, `0x46`: **Haswell** (4th Gen Core client / ULT / H)
    /// - `0x3E`: **Ivy Bridge-E/EN/EP** — **not allowlisted** (not a T440p CPU;
    ///   not independently required for this phase)
    ///
    /// MSR justification (Intel SDM, public):
    /// - Vol. 3 thermal management + Vol. 4 MSR map:
    ///   - `IA32_THERM_STATUS` (0x19C): present when CPUID.06H:EAX[0]=1
    ///     (architectural DTS status; digital readout + valid bit).
    ///   - `IA32_PACKAGE_THERM_STATUS` (0x1B1): present when CPUID.06H:EAX[6]=1
    ///     (package thermal management).
    ///   - `IA32_TEMPERATURE_TARGET` (0x1A2): listed for Haswell display models
    ///     06_3CH / 06_45H / 06_46H in Vol. 4 model-specific tables (TjMax field
    ///     bits 23:16). Read only after CPUID DTS + model allowlist.
    ///
    /// T440p ships Haswell mobile (typically 06_3CH). Models outside this list
    /// return Unsupported with **no** RDMSR.
    pub fn model_allows_dts(family: u32, model: u32) -> bool {
        if family != 6 {
            return false;
        }
        matches!(model, 0x3C | 0x45 | 0x46)
    }

    /// Models known to support package thermal status when CPUID PTM is set.
    /// Same Haswell set: package MSR is gated by CPUID.06H:EAX[6] at runtime.
    pub fn model_allows_package_therm(family: u32, model: u32) -> bool {
        model_allows_dts(family, model)
    }

    /// Human-readable microarchitecture label for documentation / logs.
    pub fn model_uarch_name(family: u32, model: u32) -> &'static str {
        if family != 6 {
            return "unknown";
        }
        match model {
            0x3C | 0x45 | 0x46 => "Haswell",
            0x3E => "Ivy Bridge-E/EN/EP (not allowlisted)",
            _ => "unknown",
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DtsProbeResult {
        Ready {
            package: bool,
            tj_max_c: u8,
        },
        Unsupported,
    }

    /// Pure feature gate — no MSR access. Callers must not RDMSR on Unsupported.
    pub fn feature_gate(
        vendor_intel: bool,
        max_cpuid_leaf: u32,
        thermal_eax: u32,
        family: u32,
        model: u32,
        temperature_target_msr: Option<u64>,
    ) -> DtsProbeResult {
        if !vendor_intel {
            return DtsProbeResult::Unsupported;
        }
        if max_cpuid_leaf < CPUID_LEAF_THERMAL {
            return DtsProbeResult::Unsupported;
        }
        if thermal_eax & CPUID_DTS_BIT == 0 {
            return DtsProbeResult::Unsupported;
        }
        if !model_allows_dts(family, model) {
            return DtsProbeResult::Unsupported;
        }
        let Some(tt) = temperature_target_msr else {
            return DtsProbeResult::Unsupported;
        };
        let tj = ((tt >> TEMP_TARGET_SHIFT) & TEMP_TARGET_MASK) as u8;
        // Conservative sanity: desktop/mobile TjMax typically 80–110°C.
        if tj < 60 || tj > 120 {
            return DtsProbeResult::Unsupported;
        }
        let package = (thermal_eax & CPUID_PTM_BIT) != 0 && model_allows_package_therm(family, model);
        DtsProbeResult::Ready {
            package,
            tj_max_c: tj,
        }
    }

    /// Compute absolute temperature from validated THERM_STATUS + TjMax.
    ///
    /// Formula (Intel SDM Vol. 3, thermal management):
    ///   absolute_°C = temperature_target − digital_readout
    /// where digital_readout is bits 22:16 and valid bit 31 must be set.
    pub fn temperature_from_therm_status(
        therm_status: u64,
        tj_max_c: u8,
    ) -> Result<MilliC, SensorStatus> {
        if therm_status & THERM_STATUS_VALID == 0 {
            return Err(SensorStatus::Invalid);
        }
        let delta = ((therm_status >> THERM_DIGITAL_SHIFT) & THERM_DIGITAL_MASK) as i32;
        // Ignore reserved bits by masking only defined fields above.
        let tj = tj_max_c as i32;
        let temp_c = tj.checked_sub(delta).ok_or(SensorStatus::Invalid)?;
        let milli = temp_c.checked_mul(1000).ok_or(SensorStatus::Invalid)?;
        if !is_sane_milli_c(milli) {
            return Err(SensorStatus::Invalid);
        }
        Ok(milli)
    }

    /// Confirm no WRMSR constants are exported for thermal control.
    pub fn thermal_msrs_read_only_list() -> &'static [u32] {
        &[
            MSR_IA32_THERM_STATUS,
            MSR_IA32_TEMPERATURE_TARGET,
            MSR_IA32_PACKAGE_THERM_STATUS,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::intel_dts::*;
    use super::*;

    #[test]
    fn valid_milli_degree() {
        let r = SensorReading::valid(SensorId::new(SensorClass::CpuPackage, 0), 61_000, 1000)
            .unwrap();
        assert_eq!(r.temp_milli_c(), Some(61_000));
    }

    #[test]
    fn unavailable_not_zero() {
        let r = SensorReading::unavailable(SensorId::new(SensorClass::CpuPackage, 0));
        assert_eq!(r.temp_milli_c(), None);
        assert_ne!(r.status, SensorStatus::Valid);
        // value field may be 0 but must not be consumed.
        assert_eq!(r.value, 0);
        assert!(r.temp_milli_c().is_none());
    }

    #[test]
    fn stale_remains_stale() {
        let r = SensorReading::valid(SensorId::new(SensorClass::CpuCore, 0), 50_000, 100)
            .unwrap()
            .mark_stale_if_older_than(10_000, SENSOR_STALE_MS);
        assert_eq!(r.status, SensorStatus::Stale);
        assert_eq!(r.mark_stale_if_older_than(20_000, SENSOR_STALE_MS).status, SensorStatus::Stale);
    }

    #[test]
    fn invalid_timestamp_rejected() {
        let r = SensorReading {
            sensor_id: SensorId(1),
            value: 40_000,
            monotonic_ms: 0,
            status: SensorStatus::Valid,
        };
        assert_eq!(r.validate_combination(), Err(SensorModelError::InvalidTimestamp));
    }

    #[test]
    fn bounded_enumeration() {
        let mut snap = SensorSnapshot::empty(1);
        for i in 0..MAX_SENSORS {
            snap.push(
                SensorReading::valid(SensorId::new(SensorClass::CpuCore, i as u16), 40_000, 1)
                    .unwrap(),
            )
            .unwrap();
        }
        assert!(snap
            .push(SensorReading::valid(SensorId::new(SensorClass::CpuCore, 99), 40_000, 1).unwrap())
            .is_err());
    }

    #[test]
    fn stable_ids() {
        let a = SensorId::new(SensorClass::CpuCore, 3);
        let b = SensorId::new(SensorClass::CpuCore, 3);
        assert_eq!(a, b);
        assert_eq!(a.index(), 3);
    }

    #[test]
    fn coherent_snapshot_max() {
        let mut snap = SensorSnapshot::empty(50);
        snap.push(
            SensorReading::valid(SensorId::new(SensorClass::CpuCore, 0), 59_000, 50).unwrap(),
        )
        .unwrap();
        snap.push(
            SensorReading::valid(SensorId::new(SensorClass::CpuCore, 1), 61_000, 50).unwrap(),
        )
        .unwrap();
        snap.push(SensorReading::unavailable(SensorId::new(SensorClass::CpuPackage, 0)))
            .unwrap();
        assert_eq!(snap.max_valid_temp_mc(), Some(61_000));
    }

    #[test]
    fn non_intel_unsupported() {
        let r = feature_gate(false, 20, CPUID_DTS_BIT, 6, 0x3C, Some(0x64 << 16));
        assert_eq!(r, DtsProbeResult::Unsupported);
    }

    #[test]
    fn missing_dts_feature() {
        let r = feature_gate(true, 20, 0, 6, 0x3C, Some(0x64 << 16));
        assert_eq!(r, DtsProbeResult::Unsupported);
    }

    #[test]
    fn unknown_model_no_msr() {
        let r = feature_gate(true, 20, CPUID_DTS_BIT | CPUID_PTM_BIT, 6, 0x9E, Some(0x64 << 16));
        assert_eq!(r, DtsProbeResult::Unsupported);
    }

    #[test]
    fn ivy_bridge_3e_not_allowlisted() {
        // 0x3E is Ivy Bridge-E/EN/EP, not Haswell; not justified for this phase.
        assert!(!model_allows_dts(6, 0x3E));
        let r = feature_gate(true, 20, CPUID_DTS_BIT | CPUID_PTM_BIT, 6, 0x3E, Some(0x64 << 16));
        assert_eq!(r, DtsProbeResult::Unsupported);
        assert_eq!(model_uarch_name(6, 0x3C), "Haswell");
        assert!(model_uarch_name(6, 0x3E).contains("Ivy Bridge"));
    }

    #[test]
    fn haswell_models_allowlisted() {
        assert!(model_allows_dts(6, 0x3C));
        assert!(model_allows_dts(6, 0x45));
        assert!(model_allows_dts(6, 0x46));
    }

    #[test]
    fn invalid_valid_bit() {
        let status = 0x0010_0000u64; // delta=16, valid=0
        assert_eq!(
            temperature_from_therm_status(status, 100),
            Err(SensorStatus::Invalid)
        );
    }

    #[test]
    fn known_target_delta() {
        // TjMax 100, delta 39 → 61°C
        let status = THERM_STATUS_VALID | (39u64 << THERM_DIGITAL_SHIFT);
        assert_eq!(temperature_from_therm_status(status, 100), Ok(61_000));
    }

    #[test]
    fn checked_arithmetic_underflow() {
        // delta > tj_max
        let status = THERM_STATUS_VALID | (120u64 << THERM_DIGITAL_SHIFT);
        // 100-120 = -20°C which is sane; use extreme
        let status = THERM_STATUS_VALID | (127u64 << THERM_DIGITAL_SHIFT);
        let r = temperature_from_therm_status(status, 60);
        // 60-127 = -67 → out of sane range
        assert!(r.is_err());
    }

    #[test]
    fn reserved_bits_ignored() {
        let status = THERM_STATUS_VALID
            | (39u64 << THERM_DIGITAL_SHIFT)
            | (0xFFFF_u64 << 32) // upper reserved-ish
            | 0x7FFF; // lower status flags
        assert_eq!(temperature_from_therm_status(status, 100), Ok(61_000));
    }

    #[test]
    fn family_model_decode_haswell() {
        // Example: family 6, model 0x3C, stepping 3
        // EAX = stepping | (model<<4) | (family<<8) | (ext_model<<16)
        let eax = 0x3 | (0xC << 4) | (0x6 << 8) | (0x3 << 16);
        let (f, m, s) = decode_family_model(eax);
        assert_eq!(f, 6);
        assert_eq!(m, 0x3C);
        assert_eq!(s, 3);
    }

    #[test]
    fn no_wrmsr_list() {
        // Documentation/test: only the three read MSRs.
        assert_eq!(thermal_msrs_read_only_list().len(), 3);
        assert!(!thermal_msrs_read_only_list().is_empty());
    }

    #[test]
    fn package_fallback_label_is_max_core() {
        // Architectural note exercised by consumers: if package unavailable,
        // max core must be labeled "Maximum core temperature".
        let mut snap = SensorSnapshot::empty(1);
        snap.push(
            SensorReading::valid(SensorId::new(SensorClass::CpuCore, 0), 59_000, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(snap.max_valid_temp_mc(), Some(59_000));
        // No package sensor present.
        assert!(!snap.iter().any(|r| r.sensor_id.class() == SensorClass::CpuPackage as u8
            && r.status == SensorStatus::Valid));
    }
}
