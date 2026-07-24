//! Control Panel page: Power & Thermal.
//!
//! Shows powerd requested/effective modes and thermald sensors/fan state.
//! Does not present a second set of power-profile buttons owned by thermald.
//! Cooling preferences (Balanced/Quiet/Cool/Performance) are thermal fan bias
//! only; the five power modes remain owned by powerd via powerctl/UI buttons.

use core::fmt::Write;

use sun_font::{self, FontRole, TextStyle};

use sunlight_ipc::{
    ipc_call, monotonic_millis, nameserver_lookup, system_identity, CoolingProfile, FanControlMode,
    IpcMsg, PowerProfile, PowerdMsg, SystemIdentityRecord, ThermalState, ThermaldMsg,
};
use sunlight_ui::{
    widgets::{Button, ButtonState},
    Canvas, Color, Event, MaterialPalette, Point, Rect, Theme,
};

use crate::sysinfo::FixedStr;

const REFRESH_INTERVAL_MS: u64 = 1_000;
const KEY_ESC: u8 = 0x01;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PowerThermalAction {
    None,
    Back,
}

#[derive(Clone, Copy)]
struct PowerSnap {
    requested: PowerProfile,
    effective: PowerProfile,
    constraint: bool,
    constraint_reason: u8,
    on_ac: Option<bool>,
}

#[derive(Clone, Copy)]
struct ThermalSnap {
    state: ThermalState,
    fan_mode: FanControlMode,
    temp_mc: Option<i32>,
    level: u8,
    rpm: u32,
    errors: u32,
    available: bool,
    sensor_count: u8,
    valid_sensors: u8,
    package_sensor: bool,
}

#[derive(Clone, Copy)]
struct IdentitySnap {
    manufacturer: FixedStr<40>,
    product: FixedStr<40>,
    ready: bool,
}

pub struct PowerThermalPageState {
    power: Option<PowerSnap>,
    thermal: Option<ThermalSnap>,
    identity: IdentitySnap,
    sensors_expanded: bool,
    power_error: FixedStr<64>,
    thermal_error: FixedStr<64>,
    status: FixedStr<80>,
    next_refresh_ms: u64,
    selected_power: usize,
    selected_cooling: usize,
}

const POWER_MODES: [PowerProfile; 5] = [
    PowerProfile::Turbo,
    PowerProfile::Performance,
    PowerProfile::Balanced,
    PowerProfile::LowPower,
    PowerProfile::Stamina,
];

const COOLING_PROFILES: [CoolingProfile; 4] = [
    CoolingProfile::Balanced,
    CoolingProfile::Quiet,
    CoolingProfile::Cool,
    CoolingProfile::Performance,
];

impl PowerThermalPageState {
    pub fn new() -> Self {
        Self {
            power: None,
            thermal: None,
            identity: IdentitySnap {
                manufacturer: FixedStr::empty(),
                product: FixedStr::empty(),
                ready: false,
            },
            sensors_expanded: false,
            power_error: FixedStr::empty(),
            thermal_error: FixedStr::empty(),
            status: FixedStr::empty(),
            next_refresh_ms: 0,
            selected_power: 2, // Balanced
            selected_cooling: 0,
        }
    }

    pub fn refresh_due(&self) -> bool {
        monotonic_millis() >= self.next_refresh_ms
    }

    pub fn refresh(&mut self) -> bool {
        let now = monotonic_millis();
        self.refresh_identity();
        self.refresh_power();
        self.refresh_thermal();
        self.next_refresh_ms = now.saturating_add(REFRESH_INTERVAL_MS);
        true
    }

    fn refresh_identity(&mut self) {
        // Public identity only — never request serial/UUID.
        if let Some(id) = system_identity() {
            let mut mfr = FixedStr::<40>::empty();
            let mut prod = FixedStr::<40>::empty();
            let _ = mfr.push_str(SystemIdentityRecord::field_str(&id.manufacturer));
            let _ = prod.push_str(SystemIdentityRecord::field_str(&id.product_name));
            self.identity = IdentitySnap {
                manufacturer: mfr,
                product: prod,
                ready: id.ready != 0,
            };
        }
    }

    fn refresh_power(&mut self) {
        let Some(cap) = nameserver_lookup("powerd") else {
            self.power_error.set("powerd unavailable");
            return;
        };
        let reply = ipc_call(cap, IpcMsg::with_label(PowerdMsg::GET_STATUS));
        if reply.label != PowerdMsg::REPLY {
            self.power_error.set("powerd status failed");
            return;
        }
        let w2 = reply.words[2];
        let w3 = reply.words[3];
        let on_ac = if (w2 & 1) != 0 {
            Some((w2 & 2) != 0)
        } else {
            None
        };
        let requested = PowerProfile::from_u64(reply.words[0]);
        self.selected_power = POWER_MODES
            .iter()
            .position(|p| *p == requested)
            .unwrap_or(2);
        self.power = Some(PowerSnap {
            requested,
            effective: PowerProfile::from_u64(reply.words[1]),
            constraint: (w3 & 1) != 0,
            constraint_reason: ((w3 >> 24) & 0xff) as u8,
            on_ac,
        });
        self.power_error.clear();
    }

    fn refresh_thermal(&mut self) {
        let Some(cap) = nameserver_lookup("thermald") else {
            self.thermal_error.set("thermald unavailable");
            self.thermal = Some(ThermalSnap {
                state: ThermalState::Normal,
                fan_mode: FanControlMode::Unavailable,
                temp_mc: None,
                level: 0,
                rpm: 0,
                errors: 0,
                available: false,
                sensor_count: 0,
                valid_sensors: 0,
                package_sensor: false,
            });
            return;
        };
        let reply = ipc_call(cap, IpcMsg::with_label(ThermaldMsg::GET_STATUS));
        if reply.label != ThermaldMsg::REPLY {
            self.thermal_error.set("thermald status failed");
            return;
        }
        let w0 = reply.words[0];
        let w1 = reply.words[1];
        let w2 = reply.words[2];
        let w4 = reply.words[4];
        let temp_bits = w1 as u32 as i32;
        let temp_mc = if temp_bits == i32::MIN {
            None
        } else {
            Some(temp_bits)
        };
        let profile = CoolingProfile::from_u64((w0 >> 16) & 0xff);
        self.selected_cooling = COOLING_PROFILES
            .iter()
            .position(|p| *p == profile)
            .unwrap_or(0);
        let _ = profile;
        let sensor_count = ((w4 >> 48) & 0xff) as u8;
        let package_sensor = ((w4 >> 40) & 1) != 0;
        let mut valid_sensors = 0u8;
        for i in 0..sensor_count.min(8) {
            let r = ipc_call(
                cap,
                IpcMsg::with_label(ThermaldMsg::LIST_SENSORS).word(0, i as u64),
            );
            if r.label == ThermaldMsg::REPLY && r.words[1] != 0 {
                valid_sensors = valid_sensors.saturating_add(1);
            }
        }
        self.thermal = Some(ThermalSnap {
            state: ThermalState::from_u64(w0 & 0xff),
            fan_mode: FanControlMode::from_u64((w0 >> 8) & 0xff),
            temp_mc,
            level: (w2 & 0xff) as u8,
            rpm: ((w2 >> 8) & 0xffff) as u32,
            errors: (w4 & 0xffff_ffff) as u32,
            available: true,
            sensor_count,
            valid_sensors,
            package_sensor,
        });
        self.thermal_error.clear();
    }

    fn set_power_mode(&mut self, mode: PowerProfile) {
        let Some(cap) = nameserver_lookup("powerd") else {
            self.status.set("powerd unavailable");
            return;
        };
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(PowerdMsg::SET_PROFILE).word(0, mode as u64),
        );
        if reply.label == PowerdMsg::REPLY {
            self.status.set("Power mode updated");
            self.refresh_power();
        } else {
            self.status.set("Failed to set power mode");
        }
    }

    fn set_cooling_profile(&mut self, profile: CoolingProfile) {
        let Some(cap) = nameserver_lookup("thermald") else {
            self.status.set("thermald unavailable");
            return;
        };
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ThermaldMsg::SET_PROFILE).word(0, profile as u64),
        );
        if reply.label == ThermaldMsg::REPLY {
            self.status.set("Cooling preference updated");
            self.refresh_thermal();
        } else {
            self.status.set("Failed to set cooling profile");
        }
    }

    fn reset_thermal_defaults(&mut self) {
        let Some(cap) = nameserver_lookup("thermald") else {
            self.status.set("thermald unavailable");
            return;
        };
        let reply = ipc_call(cap, IpcMsg::with_label(ThermaldMsg::RESET_SAFE_DEFAULTS));
        if reply.label == ThermaldMsg::REPLY {
            self.status.set("Safe thermal defaults restored");
            self.refresh_thermal();
        } else {
            self.status.set("Reset failed");
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme, win_w: u32, win_h: u32) {
        canvas.clear_transparent(Rect::new(0, 0, win_w, win_h));

        let header = Rect::new(0, 0, win_w, 44);
        canvas.fill_material(
            header,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(0)
                .without_border(),
        );
        canvas.draw_rect(Rect::new(0, 43, win_w, 1), theme.chrome.subtle_border);

        draw_text(
            canvas,
            Rect::new(16, 12, win_w - 32, 20),
            "Power & Thermal",
            theme,
            FontRole::UiTitle,
            theme.text,
        );

        // Back button
        let back = Button::secondary(Rect::new(12, win_h as i32 - 44, 80, 28), "Back");
        back.draw(canvas, theme);

        // --- Power section ---
        let power_card = Rect::new(12, 56, win_w - 24, 150);
        canvas.fill_material(
            power_card,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(10)
                .without_border(),
        );
        draw_text(
            canvas,
            Rect::new(24, 64, 200, 18),
            "Power modes (powerd)",
            theme,
            FontRole::UiMedium,
            theme.text,
        );

        if let Some(p) = self.power {
            let mut line = FixedStr::<96>::empty();
            let _ = write!(
                line,
                "Requested: {}   Effective: {}",
                p.requested.as_str(),
                p.effective.as_str()
            );
            draw_text(
                canvas,
                Rect::new(24, 88, win_w - 48, 16),
                line.as_str(),
                theme,
                FontRole::UiSmall,
                theme.text,
            );
            let src = match p.on_ac {
                Some(true) => "Source: AC",
                Some(false) => "Source: Battery",
                None => "Source: unknown",
            };
            draw_text(
                canvas,
                Rect::new(24, 106, 200, 16),
                src,
                theme,
                FontRole::UiSmall,
                theme.text_dim,
            );
            if p.constraint {
                let reason = match p.constraint_reason {
                    1 => "ThermalWarm",
                    2 => "ThermalHot",
                    3 => "ThermalCritical",
                    _ => "Thermal",
                };
                let mut c = FixedStr::<80>::empty();
                let _ = write!(c, "Constraint: {} (thermal safety)", reason);
                draw_text(
                    canvas,
                    Rect::new(24, 124, win_w - 48, 16),
                    c.as_str(),
                    theme,
                    FontRole::UiSmall,
                    theme.accent,
                );
            } else {
                draw_text(
                    canvas,
                    Rect::new(24, 124, win_w - 48, 16),
                    "Constraint: none",
                    theme,
                    FontRole::UiSmall,
                    theme.text_dim,
                );
            }
        } else if !self.power_error.is_empty() {
            draw_text(
                canvas,
                Rect::new(24, 88, win_w - 48, 16),
                self.power_error.as_str(),
                theme,
                FontRole::UiSmall,
                theme.accent,
            );
        }

        // Power mode buttons
        let mut x = 24i32;
        let y = 148i32;
        for (i, mode) in POWER_MODES.iter().enumerate() {
            let label = mode.as_str();
            let w = 84u32;
            let btn = if i == self.selected_power {
                let mut b = Button::new(Rect::new(x, y, w, 26), label);
                b.state = ButtonState::Pressed;
                b
            } else {
                Button::secondary(Rect::new(x, y, w, 26), label)
            };
            btn.draw(canvas, theme);
            x += w as i32 + 6;
        }

        // --- Thermal section ---
        let thermal_card = Rect::new(12, 218, win_w - 24, 280);
        canvas.fill_material(
            thermal_card,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(10)
                .without_border(),
        );
        draw_text(
            canvas,
            Rect::new(24, 226, 280, 18),
            "Thermal & Cooling (thermald)",
            theme,
            FontRole::UiMedium,
            theme.text,
        );

        // Device identity (public fields only).
        {
            let mut dev = FixedStr::<96>::empty();
            if self.identity.ready
                && (!self.identity.manufacturer.is_empty() || !self.identity.product.is_empty())
            {
                let _ = write!(
                    dev,
                    "Device: {} {}",
                    self.identity.manufacturer.as_str(),
                    self.identity.product.as_str()
                );
            } else {
                let _ = dev.push_str("Device: Unknown");
            }
            draw_text(
                canvas,
                Rect::new(24, 246, win_w - 48, 16),
                dev.as_str(),
                theme,
                FontRole::UiSmall,
                theme.text_dim,
            );
        }

        if let Some(t) = self.thermal {
            let mut temp_s = FixedStr::<48>::empty();
            match t.temp_mc {
                Some(mc) => {
                    if t.package_sensor {
                        let _ = write!(temp_s, "CPU package: {}°C", mc / 1000);
                    } else {
                        let _ = write!(temp_s, "CPU max (cores): {}°C", mc / 1000);
                    }
                }
                None => {
                    let _ = temp_s.push_str("CPU: Unavailable");
                }
            }
            let mut state_s = FixedStr::<48>::empty();
            let _ = write!(state_s, "State: {}", t.state.as_str());
            draw_text(
                canvas,
                Rect::new(24, 266, 200, 16),
                state_s.as_str(),
                theme,
                FontRole::UiSmall,
                theme.text,
            );
            draw_text(
                canvas,
                Rect::new(220, 266, 220, 16),
                temp_s.as_str(),
                theme,
                FontRole::UiSmall,
                theme.text,
            );

            let mut sens_s = FixedStr::<64>::empty();
            let _ = write!(
                sens_s,
                "Sensors: {} valid of {}",
                t.valid_sensors, t.sensor_count
            );
            draw_text(
                canvas,
                Rect::new(24, 284, 220, 16),
                sens_s.as_str(),
                theme,
                FontRole::UiSmall,
                theme.text_dim,
            );
            let expand_label = if self.sensors_expanded {
                "[-] Sensors"
            } else {
                "[+] Sensors"
            };
            draw_text(
                canvas,
                Rect::new(260, 284, 120, 16),
                expand_label,
                theme,
                FontRole::UiSmall,
                theme.accent,
            );

            let mut fan_s = FixedStr::<96>::empty();
            if t.rpm > 0 {
                let _ = write!(
                    fan_s,
                    "Fan: Firmware Auto  {} RPM",
                    t.rpm
                );
            } else {
                let _ = write!(
                    fan_s,
                    "Fan: {}  RPM: Unavailable",
                    if matches!(t.fan_mode, FanControlMode::Unavailable) {
                        "Unavailable"
                    } else {
                        "Firmware Auto"
                    }
                );
            }
            draw_text(
                canvas,
                Rect::new(24, 302, win_w - 48, 16),
                fan_s.as_str(),
                theme,
                FontRole::UiSmall,
                theme.text,
            );

            // Warnings
            let mut warn_y = 320i32;
            draw_text(
                canvas,
                Rect::new(24, warn_y, win_w - 48, 16),
                "Managed fan control: Disabled — EC lease not implemented",
                theme,
                FontRole::UiSmall,
                Color::rgb(220, 140, 40),
            );
            warn_y += 18;
            if t.temp_mc.is_none() || matches!(t.state, ThermalState::Unavailable) {
                draw_text(
                    canvas,
                    Rect::new(24, warn_y, win_w - 48, 16),
                    "Thermal state: Unavailable (not Normal without a valid sensor)",
                    theme,
                    FontRole::UiSmall,
                    theme.text_dim,
                );
                warn_y += 18;
                draw_text(
                    canvas,
                    Rect::new(24, warn_y, win_w - 48, 16),
                    "No temperature sensor (not reported as 0°C)",
                    theme,
                    FontRole::UiSmall,
                    theme.text_dim,
                );
                warn_y += 18;
            }
            let _ = warn_y;

            draw_text(
                canvas,
                Rect::new(24, 368, 280, 16),
                "Cooling preference (not a power mode):",
                theme,
                FontRole::UiSmall,
                theme.text_dim,
            );
            let mut cx = 24i32;
            let cy = 388i32;
            for (i, prof) in COOLING_PROFILES.iter().enumerate() {
                let label = if *prof == CoolingProfile::Balanced {
                    "Balanced*"
                } else {
                    prof.as_str()
                };
                let w = 100u32;
                let btn = if i == self.selected_cooling {
                    let mut b = Button::new(Rect::new(cx, cy, w, 26), label);
                    b.state = ButtonState::Pressed;
                    b
                } else {
                    Button::secondary(Rect::new(cx, cy, w, 26), label)
                };
                btn.draw(canvas, theme);
                cx += w as i32 + 6;
            }
            draw_text(
                canvas,
                Rect::new(24, 420, 200, 14),
                "* Recommended",
                theme,
                FontRole::UiSmall,
                theme.text_dim,
            );
        } else if !self.thermal_error.is_empty() {
            draw_text(
                canvas,
                Rect::new(24, 266, win_w - 48, 16),
                self.thermal_error.as_str(),
                theme,
                FontRole::UiSmall,
                theme.accent,
            );
        }

        // Restore defaults
        let reset = Button::secondary(
            Rect::new(win_w as i32 - 180, win_h as i32 - 44, 168, 28),
            "Restore Safe Defaults",
        );
        reset.draw(canvas, theme);

        if !self.status.is_empty() {
            draw_text(
                canvas,
                Rect::new(100, win_h as i32 - 40, win_w - 300, 20),
                self.status.as_str(),
                theme,
                FontRole::UiSmall,
                theme.text_dim,
            );
        }
    }

    pub fn update(&mut self, event: Event, win_w: u32, win_h: u32) -> (bool, PowerThermalAction) {
        match event {
            Event::Click { x, y } => {
                let pt = Point::new(x, y);
                let back = Rect::new(12, win_h as i32 - 44, 80, 28);
                if back.contains(pt) {
                    return (true, PowerThermalAction::Back);
                }
                let reset = Rect::new(win_w as i32 - 180, win_h as i32 - 44, 168, 28);
                if reset.contains(pt) {
                    self.reset_thermal_defaults();
                    return (true, PowerThermalAction::None);
                }
                // Toggle sensors expanded region.
                let expand = Rect::new(260, 284, 120, 16);
                if expand.contains(pt) {
                    self.sensors_expanded = !self.sensors_expanded;
                    return (true, PowerThermalAction::None);
                }
                // Power mode buttons
                let mut px = 24i32;
                let py = 148i32;
                for mode in POWER_MODES.iter() {
                    let r = Rect::new(px, py, 84, 26);
                    if r.contains(pt) {
                        self.set_power_mode(*mode);
                        return (true, PowerThermalAction::None);
                    }
                    px += 90;
                }
                // Cooling profile buttons
                let mut cx = 24i32;
                let cy = 370i32;
                for prof in COOLING_PROFILES.iter() {
                    let r = Rect::new(cx, cy, 100, 26);
                    if r.contains(pt) {
                        self.set_cooling_profile(*prof);
                        return (true, PowerThermalAction::None);
                    }
                    cx += 106;
                }
                (false, PowerThermalAction::None)
            }
            Event::KeyPress {
                keycode: KEY_ESC,
                pressed: true,
                ..
            } => (true, PowerThermalAction::Back),
            Event::Tick => {
                if self.refresh_due() {
                    self.refresh();
                    (true, PowerThermalAction::None)
                } else {
                    (false, PowerThermalAction::None)
                }
            }
            _ => (false, PowerThermalAction::None),
        }
    }
}

fn draw_text(
    canvas: &mut Canvas,
    rect: Rect,
    text: &str,
    theme: &Theme,
    role: FontRole,
    color: Color,
) {
    let _ = theme;
    sun_font::draw_text_vcenter(
        canvas,
        text,
        rect.x,
        rect.y,
        rect.h,
        &TextStyle::new(role, color),
    );
}
