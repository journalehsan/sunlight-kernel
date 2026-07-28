//! sunlight-thermald — thermal policy manager for SunlightOS.
//!
//! Owns sensors, fan policy, lease requests, and thermal constraints to powerd.
//! Does not own user power-mode selection (that is powerd).
//!
//! Safety:
//! - Starts in firmware-auto / monitoring-only.
//! - Manual fan control is disabled until a kernel-owned EC lease exists.
//! - Live power constraints from real DTS are disabled until physical validation.
//! - Never replaces hardware critical thermal protection.

#![no_std]
#![no_main]

mod backend;

use backend::{ProductionBackend, ThermalBackend};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv_timeout, ipc_reply, monotonic_millis,
    nameserver_lookup, nameserver_register, CoolingProfile, FanControlMode, FanLevel, IpcMsg,
    LeaseState, PowerProfile, PowerdMsg, SystemIdentityRecord, ThermalConstraintReason,
    ThermalConstraintSeverity, ThermalConstraintSource, ThermalState, ThermaldMsg,
};
use sunlight_thermald::{
    classify_thermal_state, recommended_max_power_mode, validate_sensor, HardwareModel,
    PersistedConfig, PolicyAction, ThermalPolicy, ERR_POWERD_UNAVAILABLE, ERR_SENSOR_MISSING,
    ERR_UNSUPPORTED_HW, LEASE_RENEW_MS, SAMPLE_INTERVAL_MS,
};

struct ServiceState {
    policy: ThermalPolicy,
    backend: ProductionBackend,
    last_sample_ms: u64,
    last_lease_renew_ms: u64,
    last_power_gen_sent: u64,
    fan_rpm: Option<u32>,
    observed_level: Option<FanLevel>,
    power_requested: PowerProfile,
    power_effective: PowerProfile,
    on_ac: Option<bool>,
    powerd_ok: bool,
    config: PersistedConfig,
    package_sensor: bool,
}

impl ServiceState {
    fn new() -> Self {
        let backend = ProductionBackend::discover();
        let model = backend.model();
        let mut policy = ThermalPolicy::new(model);
        let config = PersistedConfig::safe_defaults();
        policy.apply_persisted(config);
        // When monitoring is available but model is Generic/Unknown, force
        // FirmwareAuto display rather than Unavailable when sensors exist.
        if backend.monitoring_available() {
            policy.force_firmware_auto();
        }
        Self {
            policy,
            backend,
            last_sample_ms: 0,
            last_lease_renew_ms: 0,
            last_power_gen_sent: 0,
            fan_rpm: None,
            observed_level: None,
            power_requested: PowerProfile::Balanced,
            power_effective: PowerProfile::Balanced,
            on_ac: None,
            powerd_ok: false,
            config,
            package_sensor: false,
        }
    }

    fn sample(&mut self, now: u64) {
        let raw = self.backend.read_cpu_temp_mc(now);
        let validated = match raw {
            Ok((t, ts)) => {
                validate_sensor(Some((t, ts)), now, self.policy.status().controlling_temp_mc)
            }
            Err(e) => Err(e),
        };
        let sensor = validated.map(|(t, _)| t);
        let action = self.policy.tick_sensor(now, sensor);
        self.apply_action(action);

        if let Ok(snap) = self.backend.read_fan() {
            self.fan_rpm = snap.rpm;
            self.observed_level = snap.level;
            // Prefer FirmwareAuto when fan is firmware-owned.
            if snap.firmware_auto && !self.backend.control_available() {
                // Policy already holds MonitoringOnly/Unavailable; leave it.
            }
        }

        if self.policy.status().lease == LeaseState::Healthy
            && now.saturating_sub(self.last_lease_renew_ms) >= LEASE_RENEW_MS
        {
            match self.backend.renew_lease(now) {
                Ok(()) => {
                    self.policy.renew_lease(now);
                    self.last_lease_renew_ms = now;
                }
                Err(_) => {
                    self.policy.inject_lease_expiry();
                    let _ = self.backend.restore_firmware_auto();
                }
            }
        }

        // Detect package sensor for labeling.
        self.package_sensor = false;
        for i in 0..self.backend.sensor_count() {
            if let Some(s) = self.backend.sensor_at(i) {
                // class 1 = CpuPackage
                if s.class == 1 && s.status == 1 {
                    self.package_sensor = true;
                    break;
                }
            }
        }

        if self.backend.power_constraints_allowed() {
            self.sync_power_constraint(now);
        }
        self.refresh_powerd_status();
        self.last_sample_ms = now;
    }

    fn apply_action(&mut self, action: PolicyAction) {
        match action {
            PolicyAction::FirmwareAuto => {
                let _ = self.backend.restore_firmware_auto();
                let _ = self.backend.release_lease();
            }
            PolicyAction::SetLevel(level) => match self.backend.set_fan_level(level) {
                Ok(()) => self.policy.report_fan_write_ok(),
                Err(_) => {
                    self.policy.report_fan_write_failure();
                    let _ = self.backend.restore_firmware_auto();
                }
            },
            PolicyAction::Hold => {}
        }
    }

    fn sync_power_constraint(&mut self, _now: u64) {
        let st = self.policy.status();
        let gen = self.policy.power_generation();
        if gen == self.last_power_gen_sent {
            return;
        }
        let Some(powerd) = nameserver_lookup("powerd") else {
            self.powerd_ok = false;
            return;
        };
        self.powerd_ok = true;

        if let Some(max) = st.max_power_mode {
            let (sev, reason) = match st.thermal_state {
                ThermalState::Warm => (
                    ThermalConstraintSeverity::Warm,
                    ThermalConstraintReason::ThermalWarm,
                ),
                ThermalState::Hot => (
                    ThermalConstraintSeverity::Hot,
                    ThermalConstraintReason::ThermalHot,
                ),
                ThermalState::Critical => (
                    ThermalConstraintSeverity::Critical,
                    ThermalConstraintReason::ThermalCritical,
                ),
                ThermalState::Normal | ThermalState::Unavailable => (
                    ThermalConstraintSeverity::None,
                    ThermalConstraintReason::None,
                ),
            };
            let (sev, reason) = if let Some((_, s, r)) = st
                .controlling_temp_mc
                .map(classify_thermal_state)
                .and_then(recommended_max_power_mode)
            {
                (s, r)
            } else {
                (sev, reason)
            };
            let reply = ipc_call(
                powerd,
                IpcMsg::with_label(PowerdMsg::SET_THERMAL_CONSTRAINT)
                    .word(0, sev as u64)
                    .word(1, max as u64)
                    .word(2, reason as u64)
                    .word(3, ThermalConstraintSource::Thermald as u64)
                    .word(4, gen),
            );
            if reply.label == PowerdMsg::REPLY {
                self.last_power_gen_sent = gen;
            }
        } else {
            let reply = ipc_call(
                powerd,
                IpcMsg::with_label(PowerdMsg::CLEAR_THERMAL_CONSTRAINT).word(0, gen),
            );
            if reply.label == PowerdMsg::REPLY {
                self.last_power_gen_sent = gen;
            }
        }
    }

    fn refresh_powerd_status(&mut self) {
        let Some(powerd) = nameserver_lookup("powerd") else {
            self.powerd_ok = false;
            return;
        };
        let reply = ipc_call(
            powerd,
            IpcMsg::with_label(PowerdMsg::GET_POWER_POLICY_STATUS),
        );
        if reply.label != PowerdMsg::REPLY {
            let reply = ipc_call(powerd, IpcMsg::with_label(PowerdMsg::GET_STATUS));
            if reply.label == PowerdMsg::REPLY {
                self.power_requested = PowerProfile::from_u64(reply.words[0]);
                self.power_effective = PowerProfile::from_u64(reply.words[1]);
                let w2 = reply.words[2];
                self.on_ac = if (w2 & 1) != 0 {
                    Some((w2 & 2) != 0)
                } else {
                    None
                };
                self.powerd_ok = true;
            } else {
                self.powerd_ok = false;
            }
            return;
        }
        self.power_requested = PowerProfile::from_u64(reply.words[0]);
        self.power_effective = PowerProfile::from_u64(reply.words[1]);
        let w2 = reply.words[2];
        self.on_ac = if (w2 & 1) != 0 {
            Some((w2 & 2) != 0)
        } else {
            None
        };
        self.powerd_ok = true;
    }

    fn reply_status(&self) -> IpcMsg {
        let st = self.policy.status();
        let mut err = st.error_flags;
        if !self.powerd_ok {
            err |= ERR_POWERD_UNAVAILABLE;
        }
        if matches!(
            st.fan_mode,
            FanControlMode::Unavailable | FanControlMode::MonitoringOnly
        ) {
            err |= ERR_UNSUPPORTED_HW;
        }
        if st.controlling_temp_mc.is_none() {
            err |= ERR_SENSOR_MISSING;
        }

        // Prefer FirmwareAuto reporting when monitoring with sensors (firmware owns fan).
        let fan_mode = if self.backend.monitoring_available() && !self.backend.control_available() {
            FanControlMode::FirmwareAuto
        } else {
            st.fan_mode
        };

        let w0 = (st.thermal_state as u64)
            | ((fan_mode as u64) << 8)
            | ((st.profile as u64) << 16)
            | ((st.lease as u64) << 24);
        let w1 = match st.controlling_temp_mc {
            Some(t) => t as u32 as u64,
            None => i32::MIN as u32 as u64,
        };
        let rpm = self.fan_rpm.unwrap_or(0) as u64;
        let w2 = (st.requested_level as u64) | (rpm << 8);
        let mut w3 = (self.power_requested as u64) | ((self.power_effective as u64) << 8);
        if let Some(ac) = self.on_ac {
            w3 |= 1 << 16;
            if ac {
                w3 |= 1 << 17;
            }
        }
        if st.power_constraint_active && self.backend.power_constraints_allowed() {
            w3 |= 1 << 24;
        }
        if let Some(m) = st.max_power_mode {
            w3 |= (m as u64) << 32;
        }
        // word4: error flags | model<<32 | package_flag<<40 | sensor_count<<48
        let mut w4 = (err as u64) | (model_tag(self.policy.model()) << 32);
        if self.package_sensor {
            w4 |= 1 << 40;
        }
        w4 |= (self.backend.sensor_count() as u64) << 48;

        IpcMsg::with_label(ThermaldMsg::REPLY)
            .word(0, w0)
            .word(1, w1)
            .word(2, w2)
            .word(3, w3)
            .word(4, w4)
    }

    fn reply_sensor(&self, index: u64) -> IpcMsg {
        let idx = index as usize;
        let total = self.backend.sensor_count();
        if total == 0 {
            // Advertise a single unavailable slot so CLI stays stable.
            if idx != 0 {
                return IpcMsg::with_label(ThermaldMsg::ERROR).word(0, ThermaldMsg::ERR_NOT_FOUND);
            }
            return IpcMsg::with_label(ThermaldMsg::REPLY)
                .word(0, 0)
                .word(1, 0) // invalid
                .word(2, 0) // must not mean 0°C
                .word(3, 1)
                .word(4, 2); // status Unavailable
        }
        let Some(s) = self.backend.sensor_at(idx) else {
            return IpcMsg::with_label(ThermaldMsg::ERROR).word(0, ThermaldMsg::ERR_NOT_FOUND);
        };
        let valid = (s.status == 1) as u64;
        let temp = if s.status == 1 || s.status == 4 {
            s.value as u32 as u64
        } else {
            0
        };
        IpcMsg::with_label(ThermaldMsg::REPLY)
            .word(0, s.id as u64)
            .word(1, valid)
            .word(2, temp)
            .word(3, total as u64)
            .word(
                4,
                (s.status as u64)
                    | ((s.class as u64) << 8)
                    | ((s.source as u64) << 16)
                    | ((s.label as u64) << 24),
            )
            .word(5, s.mono_ms)
    }

    fn reply_cooling(&self, index: u64) -> IpcMsg {
        if index != 0 {
            return IpcMsg::with_label(ThermaldMsg::ERROR).word(0, ThermaldMsg::ERR_NOT_FOUND);
        }
        let st = self.policy.status();
        let fan_mode = if self.backend.monitoring_available() && !self.backend.control_available() {
            FanControlMode::FirmwareAuto
        } else {
            st.fan_mode
        };
        let rpm = self.fan_rpm.unwrap_or(0) as u64;
        IpcMsg::with_label(ThermaldMsg::REPLY)
            .word(0, 0)
            .word(1, fan_mode as u64)
            .word(2, st.requested_level as u64)
            .word(3, rpm)
            .word(4, 1)
    }

    fn reply_identity(&self) -> IpcMsg {
        let Some(id) = self
            .backend
            .identity()
            .or_else(sunlight_ipc::system_identity)
        else {
            return IpcMsg::with_label(ThermaldMsg::ERROR).word(0, ThermaldMsg::ERR_UNAVAILABLE);
        };
        // Pack short tags into words; full strings via dedicated multi-message later.
        // words: major/minor/confidence/ready, then hash-like first 8 bytes of mfr/product.
        let w0 = (id.smbios_major as u64)
            | ((id.smbios_minor as u64) << 8)
            | ((id.identity_confidence as u64) << 16)
            | ((id.ready as u64) << 24);
        let w1 = pack8(&id.manufacturer);
        let w2 = pack8(&id.product_name);
        let w3 = pack8(&id.bios_vendor);
        let w4 = pack8(&id.bios_version);
        IpcMsg::with_label(ThermaldMsg::REPLY)
            .word(0, w0)
            .word(1, w1)
            .word(2, w2)
            .word(3, w3)
            .word(4, w4)
            // Flag that full identity is available via kernel syscall for privileged tools.
            .word(5, 1)
    }
}

fn pack8(field: &[u8; 64]) -> u64 {
    let mut w = 0u64;
    for i in 0..8 {
        w |= (field[i] as u64) << (i * 8);
    }
    w
}

fn model_tag(m: HardwareModel) -> u64 {
    match m {
        HardwareModel::Unknown => 0,
        HardwareModel::Generic => 1,
        HardwareModel::ThinkPadT440p => 2,
        HardwareModel::ThinkPadT480 => 3,
    }
}

fn reply_err(code: u64) -> IpcMsg {
    IpcMsg::with_label(ThermaldMsg::ERROR).word(0, code)
}

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[THERMALD] PANIC: {}", info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[THERMALD] sunlight-thermald starting (monitoring-first)");
    let ep = endpoint_create();
    nameserver_register("thermald", ep);
    debug_log("[THERMALD] registered as 'thermald'");

    let mut state = ServiceState::new();
    state.policy.force_firmware_auto();
    let _ = state.backend.restore_firmware_auto();
    state.config = PersistedConfig::safe_defaults();
    state.policy.apply_persisted(state.config);

    if let Some(id) = state.backend.identity() {
        if id.ready != 0 {
            serial_println!(
                "[THERMALD] identity ready smbios={}.{} conf={}",
                id.smbios_major,
                id.smbios_minor,
                id.identity_confidence
            );
            // Log public product only (never serial/UUID).
            let mfr = SystemIdentityRecord::field_str(&id.manufacturer);
            let prod = SystemIdentityRecord::field_str(&id.product_name);
            if !mfr.is_empty() && !prod.is_empty() {
                serial_println!("[THERMALD] device: {} {}", mfr, prod);
            }
        }
    }
    if state.backend.monitoring_available() {
        debug_log("[THERMALD] kernel thermal sensors available (Intel DTS)");
    } else {
        debug_log("[THERMALD] no thermal sensors (FirmwareAuto / Unavailable)");
    }

    let mut now = monotonic_millis();
    state.sample(now);

    // Idle-cost measurement (Phase 1 validation):
    // Previously ipc_recv_timeout(ep, 200) woke ~5 Hz while kernel DTS and
    // SAMPLE_INTERVAL_MS are ~1 Hz. Over a 10 s window that yields ~50 wakes
    // and ~10 samples (ratio 5:1) even when no IPC messages arrive — pure
    // same-generation polling. Measured fix: wait up to SAMPLE_INTERVAL_MS so
    // the service wake rate matches the sensor generation rate (~1 Hz) while
    // still handling IPC promptly when a client is present (recv returns early).
    const IPC_IDLE_WAIT_MS: u64 = SAMPLE_INTERVAL_MS;
    let mut wake_count: u64 = 0;
    let mut sample_count: u64 = 0;
    let mut idle_wake_count: u64 = 0;
    let measure_until = now.saturating_add(10_000);
    let mut measured = false;

    loop {
        now = monotonic_millis();
        wake_count = wake_count.saturating_add(1);
        if now.saturating_sub(state.last_sample_ms) >= SAMPLE_INTERVAL_MS {
            state.sample(now);
            sample_count = sample_count.saturating_add(1);
        }

        match ipc_recv_timeout(ep, IPC_IDLE_WAIT_MS) {
            Some(msg) => {
                let reply = handle_msg(&mut state, &msg);
                ipc_reply(reply);
            }
            None => {
                idle_wake_count = idle_wake_count.saturating_add(1);
            }
        }

        if !measured && now >= measure_until {
            measured = true;
            serial_println!(
                "[THERMALD] idle-cost 10s: wakes={} samples={} idle_timeouts={} wait_ms={}",
                wake_count,
                sample_count,
                idle_wake_count,
                IPC_IDLE_WAIT_MS
            );
        }
    }
}

fn handle_msg(state: &mut ServiceState, msg: &IpcMsg) -> IpcMsg {
    match msg.label {
        ThermaldMsg::GET_STATUS => state.reply_status(),
        ThermaldMsg::LIST_SENSORS => state.reply_sensor(msg.words[0]),
        ThermaldMsg::LIST_COOLING => state.reply_cooling(msg.words[0]),
        ThermaldMsg::GET_IDENTITY => state.reply_identity(),
        ThermaldMsg::GET_PROFILE | ThermaldMsg::GET_POLICY => {
            let p = state.policy.profile() as u64;
            IpcMsg::with_label(ThermaldMsg::REPLY).word(0, p).word(1, 0)
        }
        ThermaldMsg::SET_PROFILE => {
            let profile = CoolingProfile::from_u64(msg.words[0]);
            state.policy.set_profile(profile);
            state.config.profile = profile;
            serial_println!("[THERMALD] cooling profile -> {}", profile.as_str());
            state.reply_status()
        }
        ThermaldMsg::RESET_SAFE_DEFAULTS => {
            state.config = PersistedConfig::safe_defaults();
            state.policy.reset_safe_defaults();
            let _ = state.backend.restore_firmware_auto();
            serial_println!("[THERMALD] safe defaults restored");
            state.reply_status()
        }
        ThermaldMsg::FORCE_FIRMWARE_AUTO => {
            state.policy.force_firmware_auto();
            let _ = state.backend.restore_firmware_auto();
            let _ = state.backend.release_lease();
            state.reply_status()
        }
        ThermaldMsg::PREPARE_SUSPEND => {
            state.policy.prepare_suspend();
            let _ = state.backend.restore_firmware_auto();
            let _ = state.backend.release_lease();
            serial_println!("[THERMALD] prepare suspend -> firmware-auto");
            state.reply_status()
        }
        ThermaldMsg::RESUME => {
            let now = monotonic_millis();
            state.policy.resume(now);
            let _ = state.backend.restore_firmware_auto();
            serial_println!("[THERMALD] resume -> firmware-auto");
            state.reply_status()
        }
        ThermaldMsg::SET_VALIDATED_CUSTOM_POLICY => reply_err(ThermaldMsg::ERR_UNSUPPORTED),
        _ => reply_err(ThermaldMsg::ERR_BAD_REQUEST),
    }
}
