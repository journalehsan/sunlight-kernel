//! Sensor and cooling-device backends.
//!
//! Production rule: manual fan control is available only when a low-level
//! driver owns a lease timeout that restores firmware-auto after service
//! failure. That kernel EC lease does not exist yet, so all real backends
//! report monitoring-only or unavailable for writes.
//!
//! Live power constraints from real DTS are disabled until physical T440p
//! validation (see docs/HARDWARE_IDENTITY_AND_THERMAL_TELEMETRY.md).

#![allow(dead_code)]

use sunlight_ipc::FanLevel;
use sunlight_ipc::{system_identity, thermal_sensors, SystemIdentityRecord, ThermalSensorRecord};
use sunlight_thermald::{HardwareModel, MilliC, SensorError, LEASE_TIMEOUT_MS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanSnapshot {
    pub rpm: Option<u32>,
    pub level: Option<FanLevel>,
    pub firmware_auto: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendError {
    Unsupported,
    Io,
    Lease,
    InvalidLevel,
}

/// Capability surface for a platform thermal backend.
pub trait ThermalBackend {
    fn model(&self) -> HardwareModel;
    fn read_cpu_temp_mc(&mut self, now_ms: u64) -> Result<(MilliC, u64), SensorError>;
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError>;
    fn acquire_lease(&mut self, now_ms: u64) -> Result<(), BackendError>;
    fn renew_lease(&mut self, now_ms: u64) -> Result<(), BackendError>;
    fn release_lease(&mut self) -> Result<(), BackendError>;
    fn set_fan_level(&mut self, level: FanLevel) -> Result<(), BackendError>;
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError>;
    fn control_available(&self) -> bool;
    fn monitoring_available(&self) -> bool;
    /// When false, thermald must not publish live thermal constraints to powerd.
    fn power_constraints_allowed(&self) -> bool {
        false
    }
    fn sensor_count(&self) -> usize {
        0
    }
    fn sensor_at(&self, _index: usize) -> Option<ThermalSensorRecord> {
        None
    }
    fn identity(&self) -> Option<SystemIdentityRecord> {
        None
    }
    fn fan_unavailable_reason(&self) -> &'static str {
        "Managed fan control: Disabled — safe EC backend not implemented"
    }
    fn sensor_unavailable_reason(&self) -> &'static str {
        "No thermal telemetry"
    }
}

/// Default backend: no sensors, no fan control (VMware / unknown hardware).
pub struct NullBackend {
    identity: Option<SystemIdentityRecord>,
}

impl NullBackend {
    pub fn discover() -> Self {
        Self {
            identity: system_identity(),
        }
    }
}

impl ThermalBackend for NullBackend {
    fn model(&self) -> HardwareModel {
        HardwareModel::Unknown
    }
    fn read_cpu_temp_mc(&mut self, _now_ms: u64) -> Result<(MilliC, u64), SensorError> {
        Err(SensorError::Missing)
    }
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError> {
        Err(BackendError::Unsupported)
    }
    fn acquire_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn renew_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn release_lease(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn set_fan_level(&mut self, _level: FanLevel) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn control_available(&self) -> bool {
        false
    }
    fn monitoring_available(&self) -> bool {
        false
    }
    fn identity(&self) -> Option<SystemIdentityRecord> {
        self.identity
    }
    fn fan_unavailable_reason(&self) -> &'static str {
        "Fan: Unavailable"
    }
    fn sensor_unavailable_reason(&self) -> &'static str {
        "No virtual thermal telemetry"
    }
}

/// Kernel-backed Intel DTS sensors + SMBIOS identity. Read-only.
///
/// Fan remains Firmware Auto; managed control disabled (no EC lease).
/// Live power constraints stay off until physical validation.
pub struct KernelHwBackend {
    model: HardwareModel,
    identity: Option<SystemIdentityRecord>,
    sensors: [ThermalSensorRecord; 16],
    sensor_count: usize,
    last_max: Option<(MilliC, u64)>,
}

impl KernelHwBackend {
    pub fn discover() -> Option<Self> {
        let identity = system_identity();
        let mut sensors = [ThermalSensorRecord::empty(); 16];
        let (filled, _total) = thermal_sensors(&mut sensors)?;
        if filled == 0 {
            // No DTS sensors — still useful if we only want identity via Null.
            return None;
        }
        // Count valid-or-present sensors (including Unavailable slots that kernel enumerated).
        let mut model = HardwareModel::Generic;
        if let Some(id) = identity {
            let mfr = id.manufacturer_str();
            let prod = id.product_name_str();
            // Exact allowlist only; do not enable fan. Record model for UI once
            // product strings are observed — never guess T440p without match.
            if eq_ignore_ascii(mfr, "LENOVO") {
                // Product match deferred: only set ThinkPadT440p after exact
                // product string is recorded from physical hardware. Until then
                // Generic with monitoring is correct.
                let _ = prod;
                model = HardwareModel::Generic;
            }
        }
        Some(Self {
            model,
            identity,
            sensors,
            sensor_count: filled,
            last_max: None,
        })
    }

    fn refresh_sensors(&mut self, now_ms: u64) {
        let mut buf = [ThermalSensorRecord::empty(); 16];
        if let Some((filled, _)) = thermal_sensors(&mut buf) {
            self.sensors = buf;
            self.sensor_count = filled;
            let mut max: Option<MilliC> = None;
            for s in self.sensors.iter().take(self.sensor_count) {
                if let Some(t) = s.temp_milli_c() {
                    max = Some(max.map_or(t, |m| m.max(t)));
                }
            }
            self.last_max = max.map(|t| (t, now_ms));
        }
    }
}

fn eq_ignore_ascii(a: &str, b: &str) -> bool {
    a.as_bytes().len() == b.as_bytes().len()
        && a.as_bytes()
            .iter()
            .zip(b.as_bytes())
            .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

impl ThermalBackend for KernelHwBackend {
    fn model(&self) -> HardwareModel {
        self.model
    }
    fn read_cpu_temp_mc(&mut self, now_ms: u64) -> Result<(MilliC, u64), SensorError> {
        self.refresh_sensors(now_ms);
        match self.last_max {
            Some((t, ts)) => Ok((t, ts)),
            None => Err(SensorError::Missing),
        }
    }
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError> {
        // Firmware still owns the fan; we do not read RPM without EC backend.
        Ok(FanSnapshot {
            rpm: None,
            level: None,
            firmware_auto: true,
        })
    }
    fn acquire_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn renew_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn release_lease(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn set_fan_level(&mut self, _level: FanLevel) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn control_available(&self) -> bool {
        false
    }
    fn monitoring_available(&self) -> bool {
        true
    }
    fn power_constraints_allowed(&self) -> bool {
        // Explicit: do not auto-clamp power from live DTS until physical validation.
        false
    }
    fn sensor_count(&self) -> usize {
        self.sensor_count
    }
    fn sensor_at(&self, index: usize) -> Option<ThermalSensorRecord> {
        if index < self.sensor_count {
            Some(self.sensors[index])
        } else {
            None
        }
    }
    fn identity(&self) -> Option<SystemIdentityRecord> {
        self.identity
    }
    fn fan_unavailable_reason(&self) -> &'static str {
        "Managed fan control: Disabled — EC lease backend unavailable"
    }
    fn sensor_unavailable_reason(&self) -> &'static str {
        "Intel DTS unavailable"
    }
}

/// Placeholder T440p backend (EC path not implemented).
pub struct ThinkPadT440pBackend {
    identified: bool,
}

impl ThinkPadT440pBackend {
    pub const fn new_unidentified() -> Self {
        Self { identified: false }
    }

    pub const fn new_identified_for_future_use() -> Self {
        Self { identified: true }
    }
}

impl ThermalBackend for ThinkPadT440pBackend {
    fn model(&self) -> HardwareModel {
        if self.identified {
            HardwareModel::ThinkPadT440p
        } else {
            HardwareModel::Unknown
        }
    }
    fn read_cpu_temp_mc(&mut self, _now_ms: u64) -> Result<(MilliC, u64), SensorError> {
        Err(SensorError::Missing)
    }
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError> {
        Err(BackendError::Unsupported)
    }
    fn acquire_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn renew_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn release_lease(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn set_fan_level(&mut self, _level: FanLevel) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn control_available(&self) -> bool {
        false
    }
    fn monitoring_available(&self) -> bool {
        self.identified
    }
}

/// T480: architecture only; not claimed supported.
pub struct ThinkPadT480Backend;

impl ThermalBackend for ThinkPadT480Backend {
    fn model(&self) -> HardwareModel {
        HardwareModel::ThinkPadT480
    }
    fn read_cpu_temp_mc(&mut self, _now_ms: u64) -> Result<(MilliC, u64), SensorError> {
        Err(SensorError::Missing)
    }
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError> {
        Err(BackendError::Unsupported)
    }
    fn acquire_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn renew_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn release_lease(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn set_fan_level(&mut self, _level: FanLevel) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn control_available(&self) -> bool {
        false
    }
    fn monitoring_available(&self) -> bool {
        false
    }
}

/// In-process mock backend for host unit tests.
pub struct MockBackend {
    pub model: HardwareModel,
    pub temp_mc: Option<MilliC>,
    pub rpm: Option<u32>,
    pub level: FanLevel,
    pub firmware_auto: bool,
    pub lease_until: u64,
    pub write_fail: bool,
    pub control: bool,
}

impl MockBackend {
    pub fn new_t440p_sim() -> Self {
        Self {
            model: HardwareModel::ThinkPadT440p,
            temp_mc: Some(45_000),
            rpm: Some(3100),
            level: FanLevel::Level1,
            firmware_auto: true,
            lease_until: 0,
            write_fail: false,
            control: true,
        }
    }

    pub fn lease_alive(&self, now_ms: u64) -> bool {
        self.lease_until > now_ms
    }
}

impl ThermalBackend for MockBackend {
    fn model(&self) -> HardwareModel {
        self.model
    }
    fn read_cpu_temp_mc(&mut self, now_ms: u64) -> Result<(MilliC, u64), SensorError> {
        match self.temp_mc {
            Some(t) => Ok((t, now_ms)),
            None => Err(SensorError::Missing),
        }
    }
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError> {
        Ok(FanSnapshot {
            rpm: self.rpm,
            level: Some(self.level),
            firmware_auto: self.firmware_auto,
        })
    }
    fn acquire_lease(&mut self, now_ms: u64) -> Result<(), BackendError> {
        if !self.control {
            return Err(BackendError::Unsupported);
        }
        self.lease_until = now_ms.saturating_add(LEASE_TIMEOUT_MS);
        self.firmware_auto = false;
        Ok(())
    }
    fn renew_lease(&mut self, now_ms: u64) -> Result<(), BackendError> {
        if !self.lease_alive(now_ms) {
            self.firmware_auto = true;
            return Err(BackendError::Lease);
        }
        self.lease_until = now_ms.saturating_add(LEASE_TIMEOUT_MS);
        Ok(())
    }
    fn release_lease(&mut self) -> Result<(), BackendError> {
        self.lease_until = 0;
        self.firmware_auto = true;
        Ok(())
    }
    fn set_fan_level(&mut self, level: FanLevel) -> Result<(), BackendError> {
        if self.write_fail {
            return Err(BackendError::Io);
        }
        if self.firmware_auto {
            return Err(BackendError::Lease);
        }
        self.level = level;
        self.rpm = Some(match level {
            FanLevel::Level0 => 0,
            FanLevel::Level1 => 2000,
            FanLevel::Level2 => 2500,
            FanLevel::Level3 => 2800,
            FanLevel::Level4 => 3100,
            FanLevel::Level5 => 3400,
            FanLevel::Level6 => 3800,
            FanLevel::Level7 => 4200,
            FanLevel::FullSpeed => 5000,
        });
        Ok(())
    }
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError> {
        self.firmware_auto = true;
        self.lease_until = 0;
        Ok(())
    }
    fn control_available(&self) -> bool {
        self.control
    }
    fn monitoring_available(&self) -> bool {
        true
    }
    fn power_constraints_allowed(&self) -> bool {
        // Mock continues to exercise powerd constraint paths in tests.
        true
    }
}

/// Select production backend: Kernel DTS if sensors exist, else Null with identity.
pub enum ProductionBackend {
    Kernel(KernelHwBackend),
    Null(NullBackend),
}

impl ProductionBackend {
    pub fn discover() -> Self {
        if let Some(k) = KernelHwBackend::discover() {
            ProductionBackend::Kernel(k)
        } else {
            ProductionBackend::Null(NullBackend::discover())
        }
    }
}

impl ThermalBackend for ProductionBackend {
    fn model(&self) -> HardwareModel {
        match self {
            Self::Kernel(b) => b.model(),
            Self::Null(b) => b.model(),
        }
    }
    fn read_cpu_temp_mc(&mut self, now_ms: u64) -> Result<(MilliC, u64), SensorError> {
        match self {
            Self::Kernel(b) => b.read_cpu_temp_mc(now_ms),
            Self::Null(b) => b.read_cpu_temp_mc(now_ms),
        }
    }
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError> {
        match self {
            Self::Kernel(b) => b.read_fan(),
            Self::Null(b) => b.read_fan(),
        }
    }
    fn acquire_lease(&mut self, now_ms: u64) -> Result<(), BackendError> {
        match self {
            Self::Kernel(b) => b.acquire_lease(now_ms),
            Self::Null(b) => b.acquire_lease(now_ms),
        }
    }
    fn renew_lease(&mut self, now_ms: u64) -> Result<(), BackendError> {
        match self {
            Self::Kernel(b) => b.renew_lease(now_ms),
            Self::Null(b) => b.renew_lease(now_ms),
        }
    }
    fn release_lease(&mut self) -> Result<(), BackendError> {
        match self {
            Self::Kernel(b) => b.release_lease(),
            Self::Null(b) => b.release_lease(),
        }
    }
    fn set_fan_level(&mut self, level: FanLevel) -> Result<(), BackendError> {
        match self {
            Self::Kernel(b) => b.set_fan_level(level),
            Self::Null(b) => b.set_fan_level(level),
        }
    }
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError> {
        match self {
            Self::Kernel(b) => b.restore_firmware_auto(),
            Self::Null(b) => b.restore_firmware_auto(),
        }
    }
    fn control_available(&self) -> bool {
        match self {
            Self::Kernel(b) => b.control_available(),
            Self::Null(b) => b.control_available(),
        }
    }
    fn monitoring_available(&self) -> bool {
        match self {
            Self::Kernel(b) => b.monitoring_available(),
            Self::Null(b) => b.monitoring_available(),
        }
    }
    fn power_constraints_allowed(&self) -> bool {
        match self {
            Self::Kernel(b) => b.power_constraints_allowed(),
            Self::Null(b) => b.power_constraints_allowed(),
        }
    }
    fn sensor_count(&self) -> usize {
        match self {
            Self::Kernel(b) => b.sensor_count(),
            Self::Null(b) => b.sensor_count(),
        }
    }
    fn sensor_at(&self, index: usize) -> Option<ThermalSensorRecord> {
        match self {
            Self::Kernel(b) => b.sensor_at(index),
            Self::Null(b) => b.sensor_at(index),
        }
    }
    fn identity(&self) -> Option<SystemIdentityRecord> {
        match self {
            Self::Kernel(b) => b.identity(),
            Self::Null(b) => b.identity(),
        }
    }
    fn fan_unavailable_reason(&self) -> &'static str {
        match self {
            Self::Kernel(b) => b.fan_unavailable_reason(),
            Self::Null(b) => b.fan_unavailable_reason(),
        }
    }
    fn sensor_unavailable_reason(&self) -> &'static str {
        match self {
            Self::Kernel(b) => b.sensor_unavailable_reason(),
            Self::Null(b) => b.sensor_unavailable_reason(),
        }
    }
}
