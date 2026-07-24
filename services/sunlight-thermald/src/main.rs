//! sunlight-thermald — thermal policy manager for SunlightOS.
//!
//! Owns sensors, fan policy, lease requests, and thermal constraints to powerd.
//! Does not own user power-mode selection (that is powerd).
//!
//! Safety:
//! - Starts in firmware-auto / monitoring-only.
//! - Manual fan control is disabled until a kernel-owned EC lease exists.
//! - Never replaces hardware critical thermal protection.

#![no_std]
#![no_main]

mod backend;

use backend::{NullBackend, ThermalBackend};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv_timeout, ipc_reply, monotonic_millis,
    nameserver_lookup, nameserver_register, CoolingProfile, FanControlMode, FanLevel, IpcMsg,
    LeaseState, PowerProfile, PowerdMsg, ThermalConstraintReason, ThermalConstraintSeverity,
    ThermalConstraintSource, ThermalState, ThermaldMsg,
};
use sunlight_thermald::{
    classify_thermal_state, recommended_max_power_mode, validate_sensor, HardwareModel,
    PersistedConfig, PolicyAction, ThermalPolicy, ERR_POWERD_UNAVAILABLE, ERR_SENSOR_MISSING,
    ERR_UNSUPPORTED_HW, LEASE_RENEW_MS, SAMPLE_INTERVAL_MS,
};

struct ServiceState {
    policy: ThermalPolicy,
    backend: NullBackend,
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
}

impl ServiceState {
    fn new() -> Self {
        let backend = NullBackend;
        let model = backend.model();
        let mut policy = ThermalPolicy::new(model);
        let config = PersistedConfig::safe_defaults();
        policy.apply_persisted(config);
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
        }
    }

    fn sample(&mut self, now: u64) {
        // Read sensors.
        let raw = self.backend.read_cpu_temp_mc(now);
        let validated = match raw {
            Ok((t, ts)) => validate_sensor(Some((t, ts)), now, self.policy.status().controlling_temp_mc),
            Err(e) => Err(e),
        };
        let sensor = validated.map(|(t, _)| t);
        let action = self.policy.tick_sensor(now, sensor);
        self.apply_action(action);

        // Fan observation (best-effort).
        if let Ok(snap) = self.backend.read_fan() {
            self.fan_rpm = snap.rpm;
            self.observed_level = snap.level;
        }

        // Lease renew if managed (currently never in production).
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

        self.sync_power_constraint(now);
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
            // Do not clear local state; report unavailable.
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
                ThermalState::Normal => (
                    ThermalConstraintSeverity::None,
                    ThermalConstraintReason::None,
                ),
            };
            // Prefer recommended severity from helper when available.
            let (sev, reason) = if let Some((_, s, r)) =
                st.controlling_temp_mc
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
        let reply = ipc_call(powerd, IpcMsg::with_label(PowerdMsg::GET_POWER_POLICY_STATUS));
        if reply.label != PowerdMsg::REPLY {
            // Fall back to GET_STATUS for older powerd during roll-out.
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

        // word0: thermal_state | fan_mode<<8 | profile<<16 | lease<<24
        let w0 = (st.thermal_state as u64)
            | ((st.fan_mode as u64) << 8)
            | ((st.profile as u64) << 16)
            | ((st.lease as u64) << 24);
        // word1: temp milli-C as i32 bit pattern, or i32::MIN if missing
        let w1 = match st.controlling_temp_mc {
            Some(t) => t as u32 as u64,
            None => i32::MIN as u32 as u64,
        };
        // word2: requested_level | rpm<<8 (rpm 0 = unknown, packed as u16)
        let rpm = self.fan_rpm.unwrap_or(0) as u64;
        let w2 = (st.requested_level as u64) | (rpm << 8);
        // word3: power requested | effective<<8 | on_ac flags<<16 | constraint<<24
        let mut w3 = (self.power_requested as u64) | ((self.power_effective as u64) << 8);
        if let Some(ac) = self.on_ac {
            w3 |= 1 << 16;
            if ac {
                w3 |= 1 << 17;
            }
        }
        if st.power_constraint_active {
            w3 |= 1 << 24;
        }
        if let Some(m) = st.max_power_mode {
            w3 |= (m as u64) << 32;
        }
        // word4: error flags | model<<32
        let w4 = (err as u64) | (model_tag(self.policy.model()) << 32);

        IpcMsg::with_label(ThermaldMsg::REPLY)
            .word(0, w0)
            .word(1, w1)
            .word(2, w2)
            .word(3, w3)
            .word(4, w4)
    }

    fn reply_sensor(&self, index: u64) -> IpcMsg {
        // v0: single logical CPU package sensor at index 0 when present.
        if index != 0 {
            return IpcMsg::with_label(ThermaldMsg::ERROR).word(0, ThermaldMsg::ERR_NOT_FOUND);
        }
        let st = self.policy.status();
        let valid = st.controlling_temp_mc.is_some() as u64;
        let temp = st.controlling_temp_mc.unwrap_or(0) as u32 as u64;
        IpcMsg::with_label(ThermaldMsg::REPLY)
            .word(0, 0) // id
            .word(1, valid)
            .word(2, temp)
            .word(3, 1) // total sensors advertised
    }

    fn reply_cooling(&self, index: u64) -> IpcMsg {
        if index != 0 {
            return IpcMsg::with_label(ThermaldMsg::ERROR).word(0, ThermaldMsg::ERR_NOT_FOUND);
        }
        let st = self.policy.status();
        let rpm = self.fan_rpm.unwrap_or(0) as u64;
        IpcMsg::with_label(ThermaldMsg::REPLY)
            .word(0, 0) // device id
            .word(1, st.fan_mode as u64)
            .word(2, st.requested_level as u64)
            .word(3, rpm)
            .word(4, 1) // total
    }
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
    // Boot sequence: firmware-auto → discover → validate config → sensors → (lease if safe).
    state.policy.force_firmware_auto();
    let _ = state.backend.restore_firmware_auto();
    state.config = PersistedConfig::safe_defaults();
    state.policy.apply_persisted(state.config);

    let mut now = monotonic_millis();
    state.sample(now);

    loop {
        now = monotonic_millis();
        if now.saturating_sub(state.last_sample_ms) >= SAMPLE_INTERVAL_MS {
            state.sample(now);
        }

        // Event-driven IPC with short timeout so sampling stays ~1 Hz without busy-loop.
        match ipc_recv_timeout(ep, 200) {
            Some(msg) => {
                let reply = handle_msg(&mut state, &msg);
                ipc_reply(reply);
            }
            None => {
                // idle
            }
        }
    }
}

fn handle_msg(state: &mut ServiceState, msg: &IpcMsg) -> IpcMsg {
    match msg.label {
        ThermaldMsg::GET_STATUS => {
            // Refresh packing with correct model tag.
            let mut r = state.reply_status();
            // Overwrite word4 model bits cleanly.
            let err = r.words[4] & 0xffff_ffff;
            r.words[4] = err | (model_tag(state.policy.model()) << 32);
            r
        }
        ThermaldMsg::LIST_SENSORS => state.reply_sensor(msg.words[0]),
        ThermaldMsg::LIST_COOLING => state.reply_cooling(msg.words[0]),
        ThermaldMsg::GET_PROFILE | ThermaldMsg::GET_POLICY => {
            let p = state.policy.profile() as u64;
            IpcMsg::with_label(ThermaldMsg::REPLY)
                .word(0, p)
                .word(1, 0) // custom not used
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
        ThermaldMsg::SET_VALIDATED_CUSTOM_POLICY => {
            reply_err(ThermaldMsg::ERR_UNSUPPORTED)
        }
        _ => reply_err(ThermaldMsg::ERR_BAD_REQUEST),
    }
}
