//! powerd v0 — SunlightOS central power profile policy service.
//!
//! Owns user-selected power modes (RequestedMode) and computes EffectiveMode
//! as the safest intersection of the user choice and temporary safety
//! constraints (thermal). Thermald may publish/clear thermal constraints;
//! powerd never calls back into thermald while applying one.
//!
//! Registered as "powerd".

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg,
    PowerPolicy, PowerProfile, PowerdMsg, ThermalConstraintReason, ThermalConstraintSeverity,
    ThermalConstraintSource,
};

/// Simple in-memory context. v0 treats missing data as None / safe defaults.
#[derive(Clone, Copy)]
struct PowerContext {
    on_ac: Option<bool>,
    battery_percent: Option<u8>,
    battery_present: bool,
    load_percent: Option<u8>,
    user_active: bool,
}

impl Default for PowerContext {
    fn default() -> Self {
        Self {
            on_ac: None,
            battery_percent: None,
            battery_present: false,
            load_percent: None,
            user_active: true,
        }
    }
}

#[derive(Clone, Copy)]
struct ThermalConstraint {
    severity: ThermalConstraintSeverity,
    maximum_allowed_mode: PowerProfile,
    reason: ThermalConstraintReason,
    source: ThermalConstraintSource,
    generation: u64,
}

struct PowerState {
    /// Persistent user choice (never overwritten by thermal).
    requested: PowerProfile,
    context: PowerContext,
    thermal: Option<ThermalConstraint>,
}

impl PowerState {
    const fn new() -> Self {
        Self {
            requested: PowerProfile::Balanced,
            context: PowerContext {
                on_ac: None,
                battery_percent: None,
                battery_present: false,
                load_percent: None,
                user_active: true,
            },
            thermal: None,
        }
    }

    /// User/auto base mode before thermal intersection.
    fn base_mode(&self) -> PowerProfile {
        if self.requested == PowerProfile::Auto {
            choose_auto_profile(&self.context)
        } else {
            self.requested
        }
    }

    fn effective(&self) -> PowerProfile {
        // Auto is resolved to a concrete mode before applying a thermal ceiling.
        // Custom is mapped through a validated safe concrete profile, never by
        // enum discriminant ordering.
        let base = resolve_concrete_for_ceiling(self.base_mode(), &self.context);
        let Some(t) = self.thermal else {
            return base;
        };
        let tmax = resolve_concrete_for_ceiling(t.maximum_allowed_mode, &self.context);
        intersect_modes(base, tmax)
    }

    fn current_policy(&self) -> PowerPolicy {
        let eff = self.effective();
        let mut p = PowerPolicy::from_profile(eff);
        p.selected_profile = self.requested;
        p.effective_profile = eff;
        p
    }

    fn set_profile(&mut self, p: PowerProfile) {
        self.requested = p;
    }

    fn set_auto(&mut self) {
        self.requested = PowerProfile::Auto;
    }

    fn set_thermal_constraint(
        &mut self,
        severity: ThermalConstraintSeverity,
        max_mode: PowerProfile,
        reason: ThermalConstraintReason,
        source: ThermalConstraintSource,
        generation: u64,
    ) -> Result<(), u64> {
        // Maximum allowed mode must be a concrete ordered mode — never Auto/Custom.
        if matches!(max_mode, PowerProfile::Auto | PowerProfile::Custom) {
            return Err(PowerdMsg::ERR_BAD_REQUEST);
        }
        if let Some(active) = self.thermal {
            if generation < active.generation {
                return Err(PowerdMsg::ERR_STALE_GENERATION);
            }
        }
        if severity == ThermalConstraintSeverity::None {
            // Treat as clear only if generation is current enough.
            if let Some(active) = self.thermal {
                if generation < active.generation {
                    return Err(PowerdMsg::ERR_STALE_GENERATION);
                }
            }
            self.thermal = None;
            return Ok(());
        }
        self.thermal = Some(ThermalConstraint {
            severity,
            maximum_allowed_mode: max_mode,
            reason,
            source,
            generation,
        });
        Ok(())
    }

    fn clear_thermal_constraint(&mut self, generation: u64) -> Result<(), u64> {
        if let Some(active) = self.thermal {
            if generation < active.generation {
                return Err(PowerdMsg::ERR_STALE_GENERATION);
            }
        }
        // Retain conservative effective mode when thermald dies: we do NOT
        // auto-clear on timeout here. Clear only via explicit Clear with gen.
        self.thermal = None;
        Ok(())
    }

    fn update_context(&mut self, msg: &IpcMsg) {
        let w = msg.words[0];
        let on_ac_known = (w & 0x1) != 0;
        let on_ac_val = (w & 0x2) != 0;
        let bat_known = (w & 0x4) != 0;
        let bat_pct = ((w >> 3) & 0xff) as u8;
        let bat_present = (w & (1 << 11)) != 0;
        let load_known = (w & (1 << 12)) != 0;
        let load_pct = ((w >> 13) & 0xff) as u8;
        let user_active = (w & (1 << 21)) != 0;

        if on_ac_known {
            self.context.on_ac = Some(on_ac_val);
        }
        if bat_known {
            self.context.battery_percent = Some(bat_pct.min(100));
        }
        self.context.battery_present = bat_present;
        if load_known {
            self.context.load_percent = Some(load_pct.min(100));
        }
        self.context.user_active = user_active;
    }
}

/// Rank for concrete power modes only (lower = more aggressive).
/// Auto/Custom must be resolved to a concrete mode before calling this.
fn mode_rank(p: PowerProfile) -> u8 {
    match p {
        PowerProfile::Turbo => 0,
        PowerProfile::Performance => 1,
        PowerProfile::Balanced => 2,
        PowerProfile::LowPower => 3,
        PowerProfile::Stamina => 4,
        // Auto/Custom are not ordered by enum discriminant; map via safe default
        // only if a caller forgets to resolve first.
        PowerProfile::Auto | PowerProfile::Custom => 2,
    }
}

/// Resolve Auto (context) / Custom (safe mapping) before thermal intersection.
fn resolve_concrete_for_ceiling(p: PowerProfile, ctx: &PowerContext) -> PowerProfile {
    match p {
        PowerProfile::Auto => choose_auto_profile(ctx),
        // Custom is not compared by discriminant; until custom parameters exist,
        // map through the validated safe Balanced profile.
        PowerProfile::Custom => PowerProfile::Balanced,
        other => other,
    }
}

fn intersect_modes(requested_base: PowerProfile, thermal_max: PowerProfile) -> PowerProfile {
    // Callers should pass concrete modes; still sanitize defensively.
    let base = match requested_base {
        PowerProfile::Auto | PowerProfile::Custom => PowerProfile::Balanced,
        other => other,
    };
    let tmax = match thermal_max {
        PowerProfile::Auto | PowerProfile::Custom => PowerProfile::Balanced,
        other => other,
    };
    // Lower rank = more aggressive. Clamp if requested exceeds thermal max.
    // Never compare raw enum discriminants (Custom=5 would look "safer" than Stamina=4).
    if mode_rank(base) < mode_rank(tmax) {
        tmax
    } else {
        base
    }
}

fn choose_auto_profile(ctx: &PowerContext) -> PowerProfile {
    if ctx.on_ac == Some(true) {
        if ctx.load_percent.unwrap_or(0) > 70 {
            PowerProfile::Performance
        } else {
            PowerProfile::Balanced
        }
    } else if let Some(battery) = ctx.battery_percent {
        if battery <= 15 {
            PowerProfile::Stamina
        } else if battery <= 30 {
            PowerProfile::LowPower
        } else {
            PowerProfile::Balanced
        }
    } else {
        PowerProfile::Balanced
    }
}

const PROFILES: [PowerProfile; 7] = [
    PowerProfile::Turbo,
    PowerProfile::Performance,
    PowerProfile::Balanced,
    PowerProfile::LowPower,
    PowerProfile::Stamina,
    PowerProfile::Custom,
    PowerProfile::Auto,
];

fn profile_name(p: PowerProfile) -> &'static str {
    p.as_str()
}

fn reply_err(code: u64) -> IpcMsg {
    IpcMsg::with_label(PowerdMsg::ERROR).word(0, code)
}

fn pack_context_word(ctx: &PowerContext) -> u64 {
    let mut w2: u64 = 0;
    if let Some(ac) = ctx.on_ac {
        w2 |= 1;
        if ac {
            w2 |= 2;
        }
    }
    if let Some(b) = ctx.battery_percent {
        w2 |= 1 << 2;
        w2 |= (b as u64) << 3;
    }
    if ctx.battery_present {
        w2 |= 1 << 11;
    }
    if let Some(l) = ctx.load_percent {
        w2 |= 1 << 12;
        w2 |= (l as u64) << 13;
    }
    if ctx.user_active {
        w2 |= 1 << 21;
    }
    w2
}

fn reply_ok_status(state: &PowerState) -> IpcMsg {
    let sel = state.requested as u64;
    let eff = state.effective() as u64;
    let w2 = pack_context_word(&state.context);
    // word3: thermal constraint snapshot
    let mut w3: u64 = 0;
    if let Some(t) = state.thermal {
        w3 |= 1; // present
        w3 |= (t.severity as u64) << 8;
        w3 |= (t.maximum_allowed_mode as u64) << 16;
        w3 |= (t.reason as u64) << 24;
        w3 |= (t.generation & 0xffff) << 32;
    }
    IpcMsg::with_label(PowerdMsg::REPLY)
        .word(0, sel)
        .word(1, eff)
        .word(2, w2)
        .word(3, w3)
}

fn reply_policy(state: &PowerState) -> IpcMsg {
    let pol = state.current_policy();
    let w0 = (pol.selected_profile as u64) | ((pol.effective_profile as u64) << 8);
    let w1 = (pol.cache_mode as u64)
        | ((pol.prefetch_mode as u64) << 8)
        | ((pol.effects_mode as u64) << 16)
        | ((pol.scheduler_bias as u64) << 24)
        | (if pol.background_work_allowed { 1u64 } else { 0 } << 32);
    IpcMsg::with_label(PowerdMsg::REPLY).word(0, w0).word(1, w1)
}

fn reply_power_policy_status(state: &PowerState) -> IpcMsg {
    // Same packing as GET_STATUS plus explicit constraint fields for thermald/UI.
    reply_ok_status(state)
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
    serial_println!("[POWERD] PANIC: {}", info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[POWERD] powerd v0 starting (thermal constraints enabled)");
    let ep = endpoint_create();
    nameserver_register("powerd", ep);
    debug_log("[POWERD] registered as 'powerd'");

    let mut state = PowerState::new();

    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            PowerdMsg::GET_STATUS | PowerdMsg::GET_PROFILE | PowerdMsg::GET_POWER_POLICY_STATUS => {
                reply_power_policy_status(&state)
            }
            PowerdMsg::SET_PROFILE => {
                let p = PowerProfile::from_u64(msg.words[0]);
                state.set_profile(p);
                serial_println!("[POWERD] set requested profile -> {}", profile_name(p));
                reply_ok_status(&state)
            }
            PowerdMsg::SET_AUTO => {
                state.set_auto();
                serial_println!("[POWERD] set Auto");
                reply_ok_status(&state)
            }
            PowerdMsg::LIST_PROFILES => {
                let idx = msg.words[0] as usize;
                if idx < PROFILES.len() {
                    let p = PROFILES[idx];
                    IpcMsg::with_label(PowerdMsg::REPLY)
                        .word(0, p as u64)
                        .word(1, PROFILES.len() as u64)
                } else {
                    reply_err(PowerdMsg::ERR_NOT_FOUND)
                }
            }
            PowerdMsg::GET_POLICY => reply_policy(&state),
            PowerdMsg::UPDATE_CONTEXT => {
                state.update_context(&msg);
                reply_ok_status(&state)
            }
            PowerdMsg::SET_THERMAL_CONSTRAINT => {
                let severity = ThermalConstraintSeverity::from_u64(msg.words[0]);
                let max_mode = PowerProfile::from_u64(msg.words[1]);
                let reason = ThermalConstraintReason::from_u64(msg.words[2]);
                let source = ThermalConstraintSource::from_u64(msg.words[3]);
                let generation = msg.words[4];
                match state.set_thermal_constraint(severity, max_mode, reason, source, generation)
                {
                    Ok(()) => {
                        serial_println!(
                            "[POWERD] thermal constraint gen={} max={} reason={}",
                            generation,
                            profile_name(max_mode),
                            reason.as_str()
                        );
                        reply_ok_status(&state)
                    }
                    Err(code) => reply_err(code),
                }
            }
            PowerdMsg::CLEAR_THERMAL_CONSTRAINT => {
                let generation = msg.words[0];
                match state.clear_thermal_constraint(generation) {
                    Ok(()) => {
                        serial_println!("[POWERD] thermal constraint cleared gen={}", generation);
                        reply_ok_status(&state)
                    }
                    Err(code) => reply_err(code),
                }
            }
            PowerdMsg::SET_CUSTOM_POLICY | PowerdMsg::GET_CUSTOM_POLICY => {
                reply_err(PowerdMsg::ERR_UNSUPPORTED)
            }
            _ => reply_err(PowerdMsg::ERR_BAD_REQUEST),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
