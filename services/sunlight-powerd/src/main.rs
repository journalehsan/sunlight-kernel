//! powerd v0 — SunlightOS central power profile policy service.
//!
//! v0 purpose:
//! - Establish the profile model (Turbo..Auto) and PowerPolicy knobs.
//! - Provide a tiny, reliable IPC surface and powerctl.
//! - Compute Auto effective profile from a simple PowerContext.
//! - Keep kernel small: all policy lives here in userspace.
//!
//! Non-goals for v0:
//! - Real ACPI battery driver integration (model is ready; values may be unknown).
//! - CPU freq scaling, thermal, display effects, scheduler bias consumers.
//! - Persistence of selected profile (future via KV/sm).
//!
//! Future hooks (stubs/TODOs in code):
//! - Battery / AC presence from a future power/acpi service or deviced Power driver.
//! - Notify scheduler (niced), cache manager, prefetch, Vortex/display, networkd, sm.
//!
//! Registered as "powerd".

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg,
    PowerPolicy, PowerProfile, PowerdMsg,
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

struct PowerState {
    selected: PowerProfile,
    context: PowerContext,
}

impl PowerState {
    const fn new() -> Self {
        Self {
            selected: PowerProfile::Balanced,
            context: PowerContext {
                on_ac: None,
                battery_percent: None,
                battery_present: false,
                load_percent: None,
                user_active: true,
            },
        }
    }

    fn effective(&self) -> PowerProfile {
        if self.selected == PowerProfile::Auto {
            choose_auto_profile(&self.context)
        } else {
            self.selected
        }
    }

    fn current_policy(&self) -> PowerPolicy {
        // For Custom in v0 we map to a conservative Balanced-derived policy.
        // Real Custom editing is a future feature.
        let eff = self.effective();
        let mut p = PowerPolicy::from_profile(eff);
        if self.selected == PowerProfile::Custom {
            p.selected_profile = PowerProfile::Custom;
            // Keep the concrete effective knobs computed above.
        }
        p
    }

    fn set_profile(&mut self, p: PowerProfile) {
        self.selected = p;
    }

    fn set_auto(&mut self) {
        self.selected = PowerProfile::Auto;
    }

    /// Best-effort context update from IPC. Unknowns remain None/false.
    fn update_context(&mut self, msg: &IpcMsg) {
        // Packing chosen for v0 (fits in few words, easy to extend):
        // word0: flags byte:
        //   bit0 = on_ac_known, bit1 = on_ac_value
        //   bit2 = battery_known, bit3..10 = battery_percent (if known)
        //   bit11 = battery_present
        //   bit12 = load_known, bit13..20 = load_percent
        //   bit21 = user_active
        // word1 reserved for future (thermal, etc.)
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

/// v0 Auto policy — conservative and safe.
/// Matches request, with small adjustments for unknown cases.
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
        // Unknown power source and no battery info: stay conservative.
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

fn reply_ok_status(state: &PowerState) -> IpcMsg {
    let sel = state.selected as u64;
    let eff = state.effective() as u64;
    let ctx = &state.context;
    // Pack small context snapshot for status consumers.
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
    IpcMsg::with_label(PowerdMsg::REPLY)
        .word(0, sel)
        .word(1, eff)
        .word(2, w2)
}

fn reply_policy(state: &PowerState) -> IpcMsg {
    let pol = state.current_policy();
    // word0: selected (low 8) | effective (next 8)
    let w0 = (pol.selected_profile as u64) | ((pol.effective_profile as u64) << 8);
    // word1: cache | prefetch<<8 | effects<<16 | sched<<24 | bg<<32
    let w1 = (pol.cache_mode as u64)
        | ((pol.prefetch_mode as u64) << 8)
        | ((pol.effects_mode as u64) << 16)
        | ((pol.scheduler_bias as u64) << 24)
        | (if pol.background_work_allowed { 1u64 } else { 0 } << 32);
    IpcMsg::with_label(PowerdMsg::REPLY).word(0, w0).word(1, w1)
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
    debug_log("[POWERD] powerd v0 starting");
    let ep = endpoint_create();
    nameserver_register("powerd", ep);
    debug_log("[POWERD] registered as 'powerd'");

    let mut state = PowerState::new();

    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            PowerdMsg::GET_STATUS | PowerdMsg::GET_PROFILE => reply_ok_status(&state),
            PowerdMsg::SET_PROFILE => {
                let p = PowerProfile::from_u64(msg.words[0]);
                state.set_profile(p);
                serial_println!("[POWERD] set profile -> {}", profile_name(p));
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
                // Recompute is implicit on next queries.
                reply_ok_status(&state)
            }
            PowerdMsg::SET_CUSTOM_POLICY | PowerdMsg::GET_CUSTOM_POLICY => {
                // v0: explicit unsupported until real custom policy store exists.
                reply_err(PowerdMsg::ERR_UNSUPPORTED)
            }
            _ => reply_err(PowerdMsg::ERR_BAD_REQUEST),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
