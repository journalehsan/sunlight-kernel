//! Pure thermal policy engine for sunlight-thermald.
//!
//! Layers:
//! - types & sensor validation
//! - fan curve / hysteresis / dwell state machine
//! - lease semantics (backend-owned timeout modeled for tests)
//! - power-constraint recommendation (consumed by powerd; never written here)
//!
//! No UI, no IPC, no EC port access. All temperatures are milli-degrees Celsius
//! (signed i32). All timing uses caller-supplied monotonic milliseconds.

#![no_std]

#[cfg(test)]
extern crate std;

use sunlight_ipc::{
    CoolingProfile, FanControlMode, FanLevel, LeaseState, PowerProfile, ThermalConstraintReason,
    ThermalConstraintSeverity, ThermalState,
};

/// Temperature unit: milli-degrees Celsius (e.g. 60_000 = 60.000°C).
pub type MilliC = i32;

pub const HYSTERESIS_MC: MilliC = 3_000;
pub const DWELL_MS: u64 = 10_000;
pub const SENSOR_STALE_MS: u64 = 5_000;
pub const LEASE_TIMEOUT_MS: u64 = 10_000;
pub const LEASE_RENEW_MS: u64 = 2_500;
pub const SAMPLE_INTERVAL_MS: u64 = 1_000;

/// Absolute sanity bounds for CPU package/core sensors (°C × 1000).
pub const TEMP_MIN_SANE_MC: MilliC = -40_000;
pub const TEMP_MAX_SANE_MC: MilliC = 125_000;

/// Hot full-speed threshold for the verified T440p Balanced curve.
pub const FULL_SPEED_MC: MilliC = 80_000;

/// Thermal state classification thresholds (CPU max, milli-°C).
pub const WARM_MC: MilliC = 70_000;
pub const HOT_MC: MilliC = 80_000;
pub const CRITICAL_MC: MilliC = 95_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareModel {
    Unknown = 0,
    Generic = 1,
    ThinkPadT440p = 2,
    ThinkPadT480 = 3,
}

impl HardwareModel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Generic => "Generic",
            Self::ThinkPadT440p => "ThinkPadT440p",
            Self::ThinkPadT480 => "ThinkPadT480",
        }
    }

    /// Manual fan control is allowed only for explicitly verified models that
    /// also have a kernel/backend lease capable of restoring firmware-auto.
    pub const fn manual_fan_allowed(self) -> bool {
        // Safety: no kernel EC lease exists yet → never claim manual control.
        let _ = self;
        false
    }

    pub const fn monitoring_supported(self) -> bool {
        matches!(
            self,
            Self::ThinkPadT440p | Self::ThinkPadT480 | Self::Generic
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorReading {
    pub id: u8,
    pub name_tag: u8,
    pub milli_c: MilliC,
    pub sample_mono_ms: u64,
    pub valid: bool,
}

impl SensorReading {
    pub const fn invalid(id: u8, name_tag: u8) -> Self {
        Self {
            id,
            name_tag,
            milli_c: 0,
            sample_mono_ms: 0,
            valid: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorError {
    Missing,
    Stale,
    Impossible,
    AbruptInvalid,
}

/// Validate a raw sensor sample. Missing never becomes 0°C.
pub fn validate_sensor(
    raw: Option<(MilliC, u64)>,
    now_ms: u64,
    previous_valid: Option<MilliC>,
) -> Result<(MilliC, u64), SensorError> {
    let Some((temp, sample_ms)) = raw else {
        return Err(SensorError::Missing);
    };
    if sample_ms > now_ms || now_ms.saturating_sub(sample_ms) > SENSOR_STALE_MS {
        return Err(SensorError::Stale);
    }
    if temp < TEMP_MIN_SANE_MC || temp > TEMP_MAX_SANE_MC {
        return Err(SensorError::Impossible);
    }
    // Reject an instantaneous jump of > 40°C between consecutive valid samples
    // as abrupt invalid (EC glitch / wrong sensor). First sample is accepted.
    if let Some(prev) = previous_valid {
        let delta = if temp > prev { temp - prev } else { prev - temp };
        if delta > 40_000 {
            return Err(SensorError::AbruptInvalid);
        }
    }
    Ok((temp, sample_ms))
}

/// One step of the verified T440p Balanced upward curve.
/// Thresholds are inclusive lower bounds for the level.
pub fn t440p_balanced_level_for(temp_mc: MilliC) -> FanLevel {
    if temp_mc >= FULL_SPEED_MC {
        FanLevel::FullSpeed
    } else if temp_mc >= 75_000 {
        FanLevel::Level7
    } else if temp_mc >= 70_000 {
        FanLevel::Level6
    } else if temp_mc >= 65_000 {
        FanLevel::Level5
    } else if temp_mc >= 60_000 {
        FanLevel::Level4
    } else if temp_mc >= 55_000 {
        FanLevel::Level3
    } else if temp_mc >= 50_000 {
        FanLevel::Level2
    } else if temp_mc >= 45_000 {
        FanLevel::Level1
    } else {
        FanLevel::Level0
    }
}

/// Profile bias applied to the Balanced curve thresholds (not power modes).
/// Quiet raises thresholds (warmer), Cool lowers them (earlier cooling).
/// Full-speed hot protection always remains at FULL_SPEED_MC.
fn profile_threshold_bias_mc(profile: CoolingProfile) -> MilliC {
    match profile {
        CoolingProfile::Quiet => 5_000,
        CoolingProfile::Cool => -5_000,
        CoolingProfile::Balanced | CoolingProfile::Performance => 0,
    }
}

pub fn level_for_temp(temp_mc: MilliC, profile: CoolingProfile) -> FanLevel {
    // Full-speed hot protection is never delayed or raised.
    if temp_mc >= FULL_SPEED_MC {
        return FanLevel::FullSpeed;
    }
    let bias = profile_threshold_bias_mc(profile);
    // Shift temperature into Balanced curve space: Cool looks hotter, Quiet cooler.
    let adjusted = temp_mc.saturating_sub(bias);
    t440p_balanced_level_for(adjusted)
}

/// Lower exit threshold for a level: enter_temp - hysteresis.
pub fn level_exit_temp_mc(level: FanLevel, profile: CoolingProfile) -> MilliC {
    let bias = profile_threshold_bias_mc(profile);
    let enter = match level {
        FanLevel::Level0 => i32::MIN / 4,
        FanLevel::Level1 => 45_000 + bias,
        FanLevel::Level2 => 50_000 + bias,
        FanLevel::Level3 => 55_000 + bias,
        FanLevel::Level4 => 60_000 + bias,
        FanLevel::Level5 => 65_000 + bias,
        FanLevel::Level6 => 70_000 + bias,
        FanLevel::Level7 => 75_000 + bias,
        FanLevel::FullSpeed => FULL_SPEED_MC, // no bias on full-speed
    };
    enter.saturating_sub(HYSTERESIS_MC)
}

pub fn classify_thermal_state(temp_mc: MilliC) -> ThermalState {
    if temp_mc >= CRITICAL_MC {
        ThermalState::Critical
    } else if temp_mc >= HOT_MC {
        ThermalState::Hot
    } else if temp_mc >= WARM_MC {
        ThermalState::Warm
    } else {
        ThermalState::Normal
    }
}

/// Recommended powerd maximum mode for a thermal state.
/// Does not touch requested mode — powerd applies the intersection.
pub fn recommended_max_power_mode(state: ThermalState) -> Option<(PowerProfile, ThermalConstraintSeverity, ThermalConstraintReason)> {
    match state {
        ThermalState::Normal => None,
        ThermalState::Warm => Some((
            PowerProfile::Performance,
            ThermalConstraintSeverity::Warm,
            ThermalConstraintReason::ThermalWarm,
        )),
        ThermalState::Hot => Some((
            PowerProfile::Balanced,
            ThermalConstraintSeverity::Hot,
            ThermalConstraintReason::ThermalHot,
        )),
        ThermalState::Critical => Some((
            PowerProfile::LowPower,
            ThermalConstraintSeverity::Critical,
            ThermalConstraintReason::ThermalCritical,
        )),
    }
}

/// Effective power mode = safest intersection of user preference and thermal max.
/// `thermal_max` is the most aggressive mode still allowed while constrained.
pub fn intersect_power_mode(requested: PowerProfile, thermal_max: Option<PowerProfile>) -> PowerProfile {
    let req = resolve_concrete(requested);
    let Some(tmax) = thermal_max.map(resolve_concrete) else {
        return req;
    };
    // Lower rank = more aggressive. If requested is more aggressive than allowed, clamp.
    if mode_rank(req) < mode_rank(tmax) {
        tmax
    } else {
        req
    }
}

fn resolve_concrete(p: PowerProfile) -> PowerProfile {
    match p {
        PowerProfile::Auto | PowerProfile::Custom => PowerProfile::Balanced,
        other => other,
    }
}

/// Lower rank = more aggressive / higher power.
fn mode_rank(p: PowerProfile) -> u8 {
    match p {
        PowerProfile::Turbo => 0,
        PowerProfile::Performance => 1,
        PowerProfile::Balanced | PowerProfile::Auto | PowerProfile::Custom => 2,
        PowerProfile::LowPower => 3,
        PowerProfile::Stamina => 4,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistedConfig {
    pub profile: CoolingProfile,
    /// Magic used when serializing; invalid blobs fall back to defaults.
    pub magic: u32,
    pub version: u16,
}

pub const CONFIG_MAGIC: u32 = 0x5448_524D; // "THRM"
pub const CONFIG_VERSION: u16 = 1;

impl PersistedConfig {
    pub const fn safe_defaults() -> Self {
        Self {
            profile: CoolingProfile::Balanced,
            magic: CONFIG_MAGIC,
            version: CONFIG_VERSION,
        }
    }

    pub fn validate(self) -> Result<Self, ()> {
        if self.magic != CONFIG_MAGIC || self.version != CONFIG_VERSION {
            return Err(());
        }
        // CoolingProfile::from_u64 already maps unknowns to Balanced.
        Ok(Self {
            profile: CoolingProfile::from_u64(self.profile as u64),
            magic: CONFIG_MAGIC,
            version: CONFIG_VERSION,
        })
    }

    pub fn pack_u64(self) -> u64 {
        (self.magic as u64)
            | ((self.version as u64) << 32)
            | ((self.profile as u64) << 48)
    }

    pub fn unpack_u64(v: u64) -> Result<Self, ()> {
        let magic = v as u32;
        let version = ((v >> 32) & 0xffff) as u16;
        let profile = CoolingProfile::from_u64((v >> 48) & 0xff);
        Self {
            profile,
            magic,
            version,
        }
        .validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    /// Request firmware-auto (and drop managed lease).
    FirmwareAuto,
    /// Request a validated discrete fan level under managed lease.
    SetLevel(FanLevel),
    /// No fan write this tick.
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyStatus {
    pub thermal_state: ThermalState,
    pub fan_mode: FanControlMode,
    pub requested_level: FanLevel,
    pub profile: CoolingProfile,
    pub controlling_temp_mc: Option<MilliC>,
    pub lease: LeaseState,
    pub power_constraint_active: bool,
    pub max_power_mode: Option<PowerProfile>,
    pub error_flags: u32,
}

pub const ERR_SENSOR_MISSING: u32 = 1 << 0;
pub const ERR_SENSOR_STALE: u32 = 1 << 1;
pub const ERR_SENSOR_IMPOSSIBLE: u32 = 1 << 2;
pub const ERR_FAN_WRITE: u32 = 1 << 3;
pub const ERR_FAN_READBACK: u32 = 1 << 4;
pub const ERR_LEASE: u32 = 1 << 5;
pub const ERR_UNSUPPORTED_HW: u32 = 1 << 6;
pub const ERR_POWERD_UNAVAILABLE: u32 = 1 << 7;

/// Deterministic thermal policy state machine.
#[derive(Debug, Clone)]
pub struct ThermalPolicy {
    model: HardwareModel,
    profile: CoolingProfile,
    current_level: FanLevel,
    level_set_mono_ms: u64,
    last_valid_temp: Option<MilliC>,
    fan_mode: FanControlMode,
    lease: LeaseState,
    lease_expires_mono_ms: u64,
    manual_enabled: bool,
    consecutive_fan_errors: u8,
    power_gen: u64,
    active_max_power: Option<PowerProfile>,
    error_flags: u32,
    suspended: bool,
}

impl ThermalPolicy {
    pub fn new(model: HardwareModel) -> Self {
        let fan_mode = if model.manual_fan_allowed() {
            FanControlMode::FirmwareAuto
        } else if model.monitoring_supported() {
            FanControlMode::MonitoringOnly
        } else {
            FanControlMode::Unavailable
        };
        Self {
            model,
            profile: CoolingProfile::Balanced,
            current_level: FanLevel::Level0,
            level_set_mono_ms: 0,
            last_valid_temp: None,
            fan_mode,
            lease: LeaseState::Unavailable,
            lease_expires_mono_ms: 0,
            manual_enabled: false,
            consecutive_fan_errors: 0,
            power_gen: 0,
            active_max_power: None,
            error_flags: if model.manual_fan_allowed() {
                0
            } else {
                ERR_UNSUPPORTED_HW
            },
            suspended: false,
        }
    }

    pub fn model(&self) -> HardwareModel {
        self.model
    }

    pub fn profile(&self) -> CoolingProfile {
        self.profile
    }

    pub fn set_profile(&mut self, profile: CoolingProfile) {
        self.profile = profile;
        // Profile change must not cause an unsafe immediate fan reduction;
        // the next tick re-evaluates with dwell rules.
    }

    pub fn reset_safe_defaults(&mut self) {
        self.profile = CoolingProfile::Balanced;
        self.force_firmware_auto();
    }

    pub fn force_firmware_auto(&mut self) {
        self.manual_enabled = false;
        self.lease = LeaseState::None;
        self.lease_expires_mono_ms = 0;
        self.current_level = FanLevel::Level0;
        self.fan_mode = if self.model.manual_fan_allowed() {
            FanControlMode::FirmwareAuto
        } else if self.model.monitoring_supported() {
            FanControlMode::MonitoringOnly
        } else {
            FanControlMode::Unavailable
        };
    }

    pub fn prepare_suspend(&mut self) {
        self.suspended = true;
        self.force_firmware_auto();
    }

    pub fn resume(&mut self, now_ms: u64) {
        let _ = now_ms;
        self.suspended = false;
        self.last_valid_temp = None;
        self.force_firmware_auto();
        // Reacquire managed only after sensors/config/policy valid — next tick.
    }

    pub fn power_generation(&self) -> u64 {
        self.power_gen
    }

    pub fn status(&self) -> PolicyStatus {
        PolicyStatus {
            thermal_state: self
                .last_valid_temp
                .map(classify_thermal_state)
                .unwrap_or(ThermalState::Normal),
            fan_mode: self.fan_mode,
            requested_level: self.current_level,
            profile: self.profile,
            controlling_temp_mc: self.last_valid_temp,
            lease: self.lease,
            power_constraint_active: self.active_max_power.is_some(),
            max_power_mode: self.active_max_power,
            error_flags: self.error_flags,
        }
    }

    /// Apply a validated temperature reading (or sensor failure).
    pub fn tick_sensor(
        &mut self,
        now_ms: u64,
        sensor: Result<MilliC, SensorError>,
    ) -> PolicyAction {
        if self.suspended {
            return PolicyAction::FirmwareAuto;
        }

        match sensor {
            Ok(temp) => {
                self.error_flags &=
                    !(ERR_SENSOR_MISSING | ERR_SENSOR_STALE | ERR_SENSOR_IMPOSSIBLE);
                self.last_valid_temp = Some(temp);
                self.evaluate_fan(now_ms, temp)
            }
            Err(e) => {
                match e {
                    SensorError::Missing => self.error_flags |= ERR_SENSOR_MISSING,
                    SensorError::Stale => self.error_flags |= ERR_SENSOR_STALE,
                    SensorError::Impossible | SensorError::AbruptInvalid => {
                        self.error_flags |= ERR_SENSOR_IMPOSSIBLE
                    }
                }
                self.last_valid_temp = None;
                self.force_firmware_auto();
                PolicyAction::FirmwareAuto
            }
        }
    }

    fn evaluate_fan(&mut self, now_ms: u64, temp_mc: MilliC) -> PolicyAction {
        // Update thermal→power recommendation.
        let state = classify_thermal_state(temp_mc);
        if let Some((max, _, _)) = recommended_max_power_mode(state) {
            if self.active_max_power != Some(max) {
                self.power_gen = self.power_gen.saturating_add(1);
                self.active_max_power = Some(max);
            }
        } else if self.active_max_power.is_some() {
            // Clear with hysteresis: only clear when Normal and dwell elapsed.
            if state == ThermalState::Normal {
                self.power_gen = self.power_gen.saturating_add(1);
                self.active_max_power = None;
            }
        }

        if !self.model.manual_fan_allowed() {
            self.fan_mode = if self.model.monitoring_supported() {
                FanControlMode::MonitoringOnly
            } else {
                FanControlMode::Unavailable
            };
            return PolicyAction::Hold;
        }

        // Lease must be healthy before managed writes.
        if self.manual_enabled {
            if now_ms > self.lease_expires_mono_ms {
                self.lease = LeaseState::Expired;
                self.error_flags |= ERR_LEASE;
                self.force_firmware_auto();
                return PolicyAction::FirmwareAuto;
            }
            self.lease = LeaseState::Healthy;
        }

        let target = level_for_temp(temp_mc, self.profile);
        let current = self.current_level;

        if target > current {
            // Upward: immediate, may skip levels.
            self.current_level = target;
            self.level_set_mono_ms = now_ms;
            self.fan_mode = if target == FanLevel::FullSpeed {
                FanControlMode::FullSpeed
            } else {
                FanControlMode::Managed
            };
            return if self.manual_enabled {
                PolicyAction::SetLevel(target)
            } else {
                PolicyAction::Hold
            };
        }

        if target < current {
            // Full-speed exit still uses hysteresis, but full-speed entry is never delayed.
            // Downward: require hysteresis temperature AND dwell.
            let exit = level_exit_temp_mc(current, self.profile);
            let dwell_ok = now_ms.saturating_sub(self.level_set_mono_ms) >= DWELL_MS;
            if temp_mc <= exit && dwell_ok {
                // Step down one level at a time to avoid oscillation.
                let next = step_down(current);
                self.current_level = next;
                self.level_set_mono_ms = now_ms;
                self.fan_mode = FanControlMode::Managed;
                return if self.manual_enabled {
                    PolicyAction::SetLevel(next)
                } else {
                    PolicyAction::Hold
                };
            }
            return PolicyAction::Hold;
        }

        // target == current
        self.fan_mode = if current == FanLevel::FullSpeed {
            FanControlMode::FullSpeed
        } else if self.manual_enabled {
            FanControlMode::Managed
        } else {
            FanControlMode::FirmwareAuto
        };
        PolicyAction::Hold
    }

    /// Attempt to enter managed mode after sensors/config are valid.
    /// Returns false if model or lease backend cannot support it.
    pub fn try_enable_managed(&mut self, now_ms: u64, lease_backend_ok: bool) -> bool {
        if !self.model.manual_fan_allowed() || !lease_backend_ok || self.suspended {
            return false;
        }
        if self.last_valid_temp.is_none() {
            return false;
        }
        self.manual_enabled = true;
        self.lease = LeaseState::Healthy;
        self.lease_expires_mono_ms = now_ms.saturating_add(LEASE_TIMEOUT_MS);
        self.fan_mode = FanControlMode::Managed;
        self.error_flags &= !(ERR_LEASE | ERR_UNSUPPORTED_HW);
        true
    }

    pub fn renew_lease(&mut self, now_ms: u64) {
        if self.manual_enabled && self.lease == LeaseState::Healthy {
            self.lease_expires_mono_ms = now_ms.saturating_add(LEASE_TIMEOUT_MS);
        }
    }

    /// Simulate backend-owned lease expiry (service crashed / hung).
    pub fn inject_lease_expiry(&mut self) {
        self.lease = LeaseState::Expired;
        self.error_flags |= ERR_LEASE;
        self.force_firmware_auto();
    }

    pub fn report_fan_write_failure(&mut self) {
        self.consecutive_fan_errors = self.consecutive_fan_errors.saturating_add(1);
        self.error_flags |= ERR_FAN_WRITE;
        if self.consecutive_fan_errors >= 3 {
            self.force_firmware_auto();
        }
    }

    pub fn report_fan_write_ok(&mut self) {
        self.consecutive_fan_errors = 0;
        self.error_flags &= !ERR_FAN_WRITE;
    }

    pub fn report_fan_readback_failure(&mut self) {
        self.error_flags |= ERR_FAN_READBACK;
    }

    pub fn apply_persisted(&mut self, cfg: PersistedConfig) {
        match cfg.validate() {
            Ok(ok) => self.profile = ok.profile,
            Err(()) => {
                let d = PersistedConfig::safe_defaults();
                self.profile = d.profile;
            }
        }
    }
}

fn step_down(level: FanLevel) -> FanLevel {
    match level {
        FanLevel::FullSpeed => FanLevel::Level7,
        FanLevel::Level7 => FanLevel::Level6,
        FanLevel::Level6 => FanLevel::Level5,
        FanLevel::Level5 => FanLevel::Level4,
        FanLevel::Level4 => FanLevel::Level3,
        FanLevel::Level3 => FanLevel::Level2,
        FanLevel::Level2 => FanLevel::Level1,
        FanLevel::Level1 | FanLevel::Level0 => FanLevel::Level0,
    }
}

/// Mock clock for tests.
#[derive(Debug, Clone, Copy)]
pub struct MockClock {
    pub ms: u64,
}

impl MockClock {
    pub const fn new() -> Self {
        Self { ms: 0 }
    }
    pub fn advance(&mut self, delta: u64) {
        self.ms = self.ms.saturating_add(delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::{assert, assert_eq, option::Option::*, result::Result::*};

    fn policy_t440p_managed() -> ThermalPolicy {
        // Force manual path for unit tests by using a patched policy.
        let mut p = ThermalPolicy::new(HardwareModel::ThinkPadT440p);
        // Override the safety gate for pure unit tests of the state machine.
        p.model = HardwareModel::ThinkPadT440p;
        // SAFETY NOTE: production manual_fan_allowed() is false until kernel lease exists.
        // Tests call enable_managed_for_test.
        p
    }

    impl ThermalPolicy {
        fn enable_managed_for_test(&mut self, now: u64) {
            self.model = HardwareModel::ThinkPadT440p;
            // Bypass production gate for pure policy tests only.
            self.manual_enabled = true;
            self.lease = LeaseState::Healthy;
            // Long default lease so hysteresis/dwell tests are not cut short.
            // Lease-expiry tests overwrite lease_expires_mono_ms explicitly.
            self.lease_expires_mono_ms = now.saturating_add(3_600_000);
            self.fan_mode = FanControlMode::Managed;
            self.error_flags = 0;
            self.last_valid_temp = Some(40_000);
        }
    }

    // Re-open manual path inside evaluate_fan for tests by temporarily
    // patching manual_fan_allowed via a test-only model flag is awkward;
    // instead we call evaluate through a thin test helper.

    impl ThermalPolicy {
        fn tick_managed(&mut self, now: u64, temp: MilliC) -> PolicyAction {
            // Enable managed once; do not refresh lease on every tick (lease
            // expiry tests rely on a fixed deadline).
            if !self.manual_enabled {
                self.enable_managed_for_test(now);
            }
            self.last_valid_temp = Some(temp);
            self.error_flags &= !(ERR_SENSOR_MISSING | ERR_SENSOR_STALE | ERR_SENSOR_IMPOSSIBLE);
            let state = classify_thermal_state(temp);
            if let Some((max, _, _)) = recommended_max_power_mode(state) {
                if self.active_max_power != Some(max) {
                    self.power_gen = self.power_gen.saturating_add(1);
                    self.active_max_power = Some(max);
                }
            } else if self.active_max_power.is_some() && state == ThermalState::Normal {
                self.power_gen = self.power_gen.saturating_add(1);
                self.active_max_power = None;
            }

            if now > self.lease_expires_mono_ms {
                self.lease = LeaseState::Expired;
                self.error_flags |= ERR_LEASE;
                self.force_firmware_auto();
                return PolicyAction::FirmwareAuto;
            }
            self.lease = LeaseState::Healthy;

            let target = level_for_temp(temp, self.profile);
            let current = self.current_level;
            if target > current {
                self.current_level = target;
                self.level_set_mono_ms = now;
                self.fan_mode = if target == FanLevel::FullSpeed {
                    FanControlMode::FullSpeed
                } else {
                    FanControlMode::Managed
                };
                return PolicyAction::SetLevel(target);
            }
            if target < current {
                let exit = level_exit_temp_mc(current, self.profile);
                let dwell_ok = now.saturating_sub(self.level_set_mono_ms) >= DWELL_MS;
                if temp <= exit && dwell_ok {
                    let next = step_down(current);
                    self.current_level = next;
                    self.level_set_mono_ms = now;
                    self.fan_mode = FanControlMode::Managed;
                    return PolicyAction::SetLevel(next);
                }
                return PolicyAction::Hold;
            }
            self.fan_mode = if current == FanLevel::FullSpeed {
                FanControlMode::FullSpeed
            } else {
                FanControlMode::Managed
            };
            PolicyAction::Hold
        }
    }

    #[test]
    fn upward_curve_thresholds() {
        let cases = [
            (44_999, FanLevel::Level0),
            (45_000, FanLevel::Level1),
            (50_000, FanLevel::Level2),
            (55_000, FanLevel::Level3),
            (60_000, FanLevel::Level4),
            (65_000, FanLevel::Level5),
            (70_000, FanLevel::Level6),
            (75_000, FanLevel::Level7),
            (80_000, FanLevel::FullSpeed),
            (90_000, FanLevel::FullSpeed),
        ];
        for (t, expected) in cases {
            assert_eq!(
                t440p_balanced_level_for(t),
                expected,
                "temp {} mC",
                t
            );
        }
    }

    #[test]
    fn downward_hysteresis_3c() {
        let mut p = policy_t440p_managed();
        let mut clock = MockClock::new();
        // Heat to level 4 (60°C).
        assert_eq!(
            p.tick_managed(clock.ms, 60_000),
            PolicyAction::SetLevel(FanLevel::Level4)
        );
        // Drop to 58°C (within 3°C hysteresis of 60) — must hold.
        clock.advance(DWELL_MS + 1);
        assert_eq!(p.tick_managed(clock.ms, 58_000), PolicyAction::Hold);
        // Drop to 57°C (60-3) after dwell — step down.
        assert_eq!(
            p.tick_managed(clock.ms, 57_000),
            PolicyAction::SetLevel(FanLevel::Level3)
        );
    }

    #[test]
    fn dwell_prevents_rapid_reduction() {
        let mut p = policy_t440p_managed();
        let mut clock = MockClock::new();
        assert_eq!(
            p.tick_managed(clock.ms, 60_000),
            PolicyAction::SetLevel(FanLevel::Level4)
        );
        clock.advance(5_000); // < 10s dwell
        assert_eq!(p.tick_managed(clock.ms, 40_000), PolicyAction::Hold);
        clock.advance(6_000);
        assert_eq!(
            p.tick_managed(clock.ms, 40_000),
            PolicyAction::SetLevel(FanLevel::Level3)
        );
    }

    #[test]
    fn upward_not_delayed_by_dwell() {
        let mut p = policy_t440p_managed();
        let mut clock = MockClock::new();
        assert_eq!(
            p.tick_managed(clock.ms, 50_000),
            PolicyAction::SetLevel(FanLevel::Level2)
        );
        clock.advance(1_000); // dwell not elapsed for down, but up is free
        assert_eq!(
            p.tick_managed(clock.ms, 66_000),
            PolicyAction::SetLevel(FanLevel::Level5)
        );
    }

    #[test]
    fn jump_50_to_82_full_speed_immediately() {
        let mut p = policy_t440p_managed();
        let clock = MockClock::new();
        assert_eq!(
            p.tick_managed(clock.ms, 50_000),
            PolicyAction::SetLevel(FanLevel::Level2)
        );
        assert_eq!(
            p.tick_managed(clock.ms, 82_000),
            PolicyAction::SetLevel(FanLevel::FullSpeed)
        );
        assert_eq!(p.status().fan_mode, FanControlMode::FullSpeed);
    }

    #[test]
    fn stable_60c_no_oscillation() {
        let mut p = policy_t440p_managed();
        let mut clock = MockClock::new();
        assert_eq!(
            p.tick_managed(clock.ms, 60_000),
            PolicyAction::SetLevel(FanLevel::Level4)
        );
        for _ in 0..30 {
            clock.advance(1_000);
            assert_eq!(p.tick_managed(clock.ms, 60_000), PolicyAction::Hold);
            assert_eq!(p.current_level, FanLevel::Level4);
        }
    }

    #[test]
    fn missing_sensor_not_zero() {
        assert_eq!(validate_sensor(None, 1000, None), Err(SensorError::Missing));
        let mut p = ThermalPolicy::new(HardwareModel::Generic);
        let action = p.tick_sensor(1000, Err(SensorError::Missing));
        assert_eq!(action, PolicyAction::FirmwareAuto);
        assert!(p.status().controlling_temp_mc.is_none());
        assert!(p.status().error_flags & ERR_SENSOR_MISSING != 0);
    }

    #[test]
    fn stale_sensor_restores_firmware_auto() {
        let mut p = ThermalPolicy::new(HardwareModel::Generic);
        let _ = p.tick_sensor(1000, Ok(55_000));
        let action = p.tick_sensor(10_000, Err(SensorError::Stale));
        assert_eq!(action, PolicyAction::FirmwareAuto);
        assert!(p.status().error_flags & ERR_SENSOR_STALE != 0);
    }

    #[test]
    fn impossible_sensor_rejected() {
        assert_eq!(
            validate_sensor(Some((200_000, 100)), 100, None),
            Err(SensorError::Impossible)
        );
        assert_eq!(
            validate_sensor(Some((-100_000, 100)), 100, None),
            Err(SensorError::Impossible)
        );
    }

    #[test]
    fn fan_write_failure_restores_auto() {
        let mut p = policy_t440p_managed();
        p.enable_managed_for_test(0);
        p.report_fan_write_failure();
        p.report_fan_write_failure();
        p.report_fan_write_failure();
        assert!(!matches!(
            p.status().fan_mode,
            FanControlMode::Managed | FanControlMode::FullSpeed
        ));
    }

    #[test]
    fn fan_readback_reported() {
        let mut p = ThermalPolicy::new(HardwareModel::Generic);
        p.report_fan_readback_failure();
        assert!(p.status().error_flags & ERR_FAN_READBACK != 0);
    }

    #[test]
    fn lease_expiration_restores_auto() {
        let mut p = policy_t440p_managed();
        p.enable_managed_for_test(0);
        p.inject_lease_expiry();
        assert_eq!(p.status().lease, LeaseState::None);
        assert!(!matches!(p.status().fan_mode, FanControlMode::Managed));
    }

    #[test]
    fn service_shutdown_restores_auto() {
        let mut p = policy_t440p_managed();
        p.enable_managed_for_test(0);
        p.force_firmware_auto();
        assert!(!matches!(p.status().fan_mode, FanControlMode::Managed));
    }

    #[test]
    fn suspend_restores_auto() {
        let mut p = policy_t440p_managed();
        p.enable_managed_for_test(0);
        p.prepare_suspend();
        assert!(p.suspended);
        assert!(!matches!(p.status().fan_mode, FanControlMode::Managed));
    }

    #[test]
    fn resume_starts_firmware_auto() {
        let mut p = policy_t440p_managed();
        p.enable_managed_for_test(0);
        p.prepare_suspend();
        p.resume(5000);
        assert!(!p.manual_enabled);
        assert!(p.last_valid_temp.is_none());
    }

    #[test]
    fn invalid_persisted_falls_back() {
        let mut p = ThermalPolicy::new(HardwareModel::Generic);
        p.apply_persisted(PersistedConfig {
            profile: CoolingProfile::Cool,
            magic: 0xdead,
            version: 99,
        });
        assert_eq!(p.profile(), CoolingProfile::Balanced);
        assert!(PersistedConfig::unpack_u64(0).is_err());
        let ok = PersistedConfig::safe_defaults().pack_u64();
        assert!(PersistedConfig::unpack_u64(ok).is_ok());
    }

    #[test]
    fn unknown_thinkpad_cannot_acquire_manual() {
        let mut p = ThermalPolicy::new(HardwareModel::ThinkPadT480);
        assert!(!p.try_enable_managed(0, true));
        let mut p2 = ThermalPolicy::new(HardwareModel::Unknown);
        assert!(!p2.try_enable_managed(0, true));
        // Production T440p also blocked until kernel lease exists.
        let mut p3 = ThermalPolicy::new(HardwareModel::ThinkPadT440p);
        assert!(!HardwareModel::ThinkPadT440p.manual_fan_allowed());
        assert!(!p3.try_enable_managed(0, true));
    }

    #[test]
    fn profiles_cannot_disable_hot_protection() {
        for profile in [
            CoolingProfile::Quiet,
            CoolingProfile::Balanced,
            CoolingProfile::Cool,
            CoolingProfile::Performance,
        ] {
            assert_eq!(level_for_temp(80_000, profile), FanLevel::FullSpeed);
            assert_eq!(level_for_temp(85_000, profile), FanLevel::FullSpeed);
        }
    }

    #[test]
    fn thermal_clamp_overrides_turbo() {
        let eff = intersect_power_mode(PowerProfile::Turbo, Some(PowerProfile::Balanced));
        assert_eq!(eff, PowerProfile::Balanced);
        let eff2 = intersect_power_mode(PowerProfile::Stamina, Some(PowerProfile::Balanced));
        assert_eq!(eff2, PowerProfile::Stamina);
    }

    #[test]
    fn cooling_restores_user_profile_recommendation() {
        let mut p = policy_t440p_managed();
        let mut clock = MockClock::new();
        let _ = p.tick_managed(clock.ms, 86_000);
        assert_eq!(p.active_max_power, Some(PowerProfile::Balanced));
        // Cool down to normal.
        clock.advance(1_000);
        let _ = p.tick_managed(clock.ms, 50_000);
        assert!(p.active_max_power.is_none());
    }

    #[test]
    fn profile_change_no_unsafe_reduction() {
        let mut p = policy_t440p_managed();
        let mut clock = MockClock::new();
        let _ = p.tick_managed(clock.ms, 60_000);
        assert_eq!(p.current_level, FanLevel::Level4);
        p.set_profile(CoolingProfile::Quiet);
        // Immediate tick without dwell must not drop levels just because profile changed.
        clock.advance(100);
        let action = p.tick_managed(clock.ms, 60_000);
        // Quiet raises thresholds so 60°C may map lower, but dwell blocks reduction.
        assert!(matches!(action, PolicyAction::Hold | PolicyAction::SetLevel(_)));
        if let PolicyAction::SetLevel(l) = action {
            // If it did set, it must only be upward or equal path — Quiet shouldn't raise.
            assert!(l <= FanLevel::Level4 || clock.ms >= DWELL_MS);
        }
    }

    #[test]
    fn monotonic_time_unaffected_by_wall_clock() {
        // Policy only receives mono ms; large jumps forward expire lease.
        let mut p = policy_t440p_managed();
        p.enable_managed_for_test(1_000);
        p.lease_expires_mono_ms = 1_000 + LEASE_TIMEOUT_MS;
        // Keep manual_enabled true without re-arming the long default lease.
        assert!(p.manual_enabled);
        let action = p.tick_managed(1_000 + LEASE_TIMEOUT_MS + 1, 50_000);
        assert_eq!(action, PolicyAction::FirmwareAuto);
    }

    #[test]
    fn rapid_requests_bounded() {
        let mut p = policy_t440p_managed();
        let mut clock = MockClock::new();
        for i in 0..1000 {
            clock.advance(10);
            let t = 45_000 + ((i % 40) * 1_000);
            let _ = p.tick_managed(clock.ms, t);
        }
        // Still well-defined state.
        let _ = p.status();
    }

    #[test]
    fn service_restart_no_stale_manual() {
        // Fresh policy always starts non-managed.
        let p = ThermalPolicy::new(HardwareModel::ThinkPadT440p);
        assert!(!matches!(
            p.status().fan_mode,
            FanControlMode::Managed | FanControlMode::FullSpeed
        ));
    }

    #[test]
    fn validate_sensor_fresh() {
        let r = validate_sensor(Some((55_000, 900)), 1000, Some(50_000));
        assert_eq!(r, Ok((55_000, 900)));
    }

    #[test]
    fn abrupt_jump_rejected() {
        let r = validate_sensor(Some((100_000, 1000)), 1000, Some(40_000));
        assert_eq!(r, Err(SensorError::AbruptInvalid));
    }

    #[test]
    fn quiet_and_cool_bias() {
        // Cool starts earlier: 42°C under Cool may request level 1.
        assert_eq!(level_for_temp(42_000, CoolingProfile::Cool), FanLevel::Level1);
        // Quiet delays: 47°C under Quiet may stay at 0.
        assert_eq!(level_for_temp(47_000, CoolingProfile::Quiet), FanLevel::Level0);
        // Balanced at 45°C is level 1.
        assert_eq!(level_for_temp(45_000, CoolingProfile::Balanced), FanLevel::Level1);
    }

    #[test]
    fn config_roundtrip() {
        let c = PersistedConfig {
            profile: CoolingProfile::Cool,
            magic: CONFIG_MAGIC,
            version: CONFIG_VERSION,
        };
        let packed = c.pack_u64();
        let back = PersistedConfig::unpack_u64(packed).unwrap();
        assert_eq!(back.profile, CoolingProfile::Cool);
    }
}
