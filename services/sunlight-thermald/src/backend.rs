//! Sensor and cooling-device backends.
//!
//! Production rule: manual fan control is available only when a low-level
//! driver owns a lease timeout that restores firmware-auto after service
//! failure. That kernel EC lease does not exist yet, so all real backends
//! report monitoring-only or unavailable for writes.

#![allow(dead_code)]

use sunlight_ipc::FanLevel;
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
    /// Acquire managed-fan lease owned by the backend/driver. Must install a
    /// timeout that restores firmware-auto without the service process.
    fn acquire_lease(&mut self, now_ms: u64) -> Result<(), BackendError>;
    fn renew_lease(&mut self, now_ms: u64) -> Result<(), BackendError>;
    fn release_lease(&mut self) -> Result<(), BackendError>;
    fn set_fan_level(&mut self, level: FanLevel) -> Result<(), BackendError>;
    fn restore_firmware_auto(&mut self) -> Result<(), BackendError>;
    fn control_available(&self) -> bool;
    fn monitoring_available(&self) -> bool;
}

/// Default backend: no sensors, no fan control (VMware / unknown hardware).
pub struct NullBackend;

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
}

/// Placeholder T440p backend.
///
/// Detection and EC register access are not available in the current kernel.
/// This backend never enables control; when DMI/EC drivers land, wire them
/// behind a true lease-owning driver and flip `control_available` only then.
pub struct ThinkPadT440pBackend {
    identified: bool,
}

impl ThinkPadT440pBackend {
    pub const fn new_unidentified() -> Self {
        Self { identified: false }
    }

    /// Only call after positive DMI product match AND verified EC interface.
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
        // No ACPI thermal / EC sensor path yet.
        Err(SensorError::Missing)
    }
    fn read_fan(&mut self) -> Result<FanSnapshot, BackendError> {
        Err(BackendError::Unsupported)
    }
    fn acquire_lease(&mut self, _now_ms: u64) -> Result<(), BackendError> {
        // Safety blocker: no kernel-owned EC lease/watchdog.
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

/// T480: monitoring may be enabled later; manual control stays disabled.
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

/// In-process mock backend for host unit tests and VMware failure drills.
/// The lease timeout is enforced by the backend object itself (simulating a
/// driver-owned watchdog). A service that stops calling renew_lease will see
/// expiry on the next backend check — this is only a simulation; production
/// must use a kernel timer outside the service process.
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
        // Simulate T440p-ish RPM for validation reference (not a universal target).
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
}
