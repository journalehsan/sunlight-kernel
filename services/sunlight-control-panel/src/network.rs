//! Read-only Control Panel composition for the shared networkd snapshot.
//!
//! Formatting and page state live here; the toolkit remains unaware of
//! interfaces and the client remains unaware of GUI presentation.

use core::fmt::Write;

use sun_font::{self, FontRole, TextStyle, Typography};

use sunlight_ipc::{monotonic_millis, AdminState, InterfaceId, InterfaceKind, LinkState};
use sunlight_networkd::{InterfaceSnapshot, NetworkClient, NetworkSnapshot, SnapshotError};
use sunlight_ui::{
    widgets::{
        BadgeKind, Button, DisclosureEvent, DisclosureGroup, DisclosureState, PropertyGrid,
        PropertyRow, StatusBadge,
    },
    Canvas, Event, Point, Rect, Theme,
};

use crate::sysinfo::FixedStr;

const REFRESH_INTERVAL_MS: u64 = 5_000;
const RETRY_BACKOFF_MS: [u64; 5] = [1_000, 2_000, 5_000, 10_000, 30_000];
const KEY_ESC: u8 = 0x01;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NetworkAction {
    None,
    Back,
}

pub struct NetworkPageState {
    snapshot: Option<NetworkSnapshot>,
    expanded: Option<(u64, InterfaceId)>,
    focused_id: Option<InterfaceId>,
    window_focused: bool,
    stale: bool,
    error: FixedStr<80>,
    next_refresh_ms: u64,
    failures: usize,
}

impl NetworkPageState {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            expanded: None,
            focused_id: None,
            window_focused: false,
            stale: false,
            error: FixedStr::empty(),
            next_refresh_ms: 0,
            failures: 0,
        }
    }

    pub fn refresh(&mut self) -> bool {
        let now = monotonic_millis();
        match NetworkClient::new().snapshot() {
            Ok(snapshot) => {
                let generation_changed = self
                    .snapshot
                    .as_ref()
                    .map(|old| old.service_generation != snapshot.service_generation)
                    .unwrap_or(false);
                if generation_changed {
                    self.expanded = None;
                }
                self.snapshot = Some(snapshot);
                self.stale = false;
                self.error.clear();
                self.failures = 0;
                self.next_refresh_ms = now.saturating_add(REFRESH_INTERVAL_MS);
                self.reconcile_selection();
            }
            Err(error) => {
                self.stale = self.snapshot.is_some();
                self.error.set(error_label(error));
                let step = RETRY_BACKOFF_MS[self.failures.min(RETRY_BACKOFF_MS.len() - 1)];
                self.failures = self.failures.saturating_add(1);
                self.next_refresh_ms = now.saturating_add(step);
            }
        }
        true
    }

    pub fn refresh_due(&self) -> bool {
        monotonic_millis() >= self.next_refresh_ms
    }

    fn interfaces(&self) -> impl Iterator<Item = &InterfaceSnapshot> {
        self.snapshot
            .iter()
            .flat_map(|snapshot| snapshot.interfaces.iter())
            .filter(|interface| {
                matches!(
                    interface.kind,
                    InterfaceKind::Ethernet
                        | InterfaceKind::VirtioNet
                        | InterfaceKind::Vmxnet3
                        | InterfaceKind::Loopback
                )
            })
    }

    fn reconcile_selection(&mut self) {
        let primary = self
            .interfaces()
            .find(|interface| !interface.is_loopback() && interface.is_default)
            .or_else(|| self.interfaces().find(|interface| !interface.is_loopback()))
            .or_else(|| self.interfaces().next())
            .map(|interface| interface.id);
        if self
            .focused_id
            .map(|id| !self.interfaces().any(|interface| interface.id == id))
            .unwrap_or(true)
        {
            self.focused_id = primary;
        }
        if self
            .expanded
            .map(|(generation, id)| {
                self.snapshot
                    .as_ref()
                    .map(|snapshot| {
                        generation != snapshot.service_generation
                            || !snapshot
                                .interfaces
                                .iter()
                                .any(|interface| interface.id == id)
                    })
                    .unwrap_or(true)
            })
            .unwrap_or(true)
        {
            self.expanded = self.snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .interfaces
                    .iter()
                    .find(|interface| !interface.is_loopback() && interface.is_default)
                    .or_else(|| {
                        snapshot
                            .interfaces
                            .iter()
                            .find(|interface| !interface.is_loopback())
                    })
                    .map(|interface| (snapshot.service_generation, interface.id))
            });
        }
    }

    fn is_expanded(&self, interface: &InterfaceSnapshot) -> bool {
        self.expanded
            == self
                .snapshot
                .as_ref()
                .map(|snapshot| (snapshot.service_generation, interface.id))
    }

    fn toggle(&mut self, interface: &InterfaceSnapshot) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let key = (snapshot.service_generation, interface.id);
        self.expanded = if self.expanded == Some(key) {
            None
        } else {
            Some(key)
        };
        self.focused_id = Some(interface.id);
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme, win_w: u32, win_h: u32) {
        canvas.fill_rect(Rect::new(0, 0, win_w, win_h), theme.bg);
        let header = Rect::new(0, 0, win_w, 44);
        canvas.fill_rect(header, theme.panel);
        canvas.draw_rect(Rect::new(0, 43, win_w, 1), theme.border);
        draw_text(canvas, "Network", 18, 10, 24, FontRole::UiTitle, theme.text);

        let refresh = refresh_rect(win_w, win_h);
        let back = back_rect(win_h);
        Button::secondary(back, "Back")
            .with_font(&Typography::UI_MEDIUM)
            .draw(canvas, theme);
        Button::secondary(refresh, "Refresh")
            .with_font(&Typography::UI_MEDIUM)
            .draw(canvas, theme);

        let summary = Rect::new(14, 54, win_w.saturating_sub(28), 42);
        canvas.fill_rounded_rect(summary, 7, theme.panel_alt);
        canvas.stroke_rounded_rect(summary, 7, 1, theme.border);
        let count = self.interfaces().count();
        let primary = self
            .interfaces()
            .find(|interface| !interface.is_loopback() && interface.is_default)
            .or_else(|| self.interfaces().find(|interface| !interface.is_loopback()));
        let mut primary_text = FixedStr::<48>::empty();
        if let Some(interface) = primary {
            primary_text.push_str(interface.name());
            if let Some((address, prefix)) = interface.ipv4_address {
                primary_text.push_str("  ");
                push_ipv4(&mut primary_text, address);
                let _ = write!(&mut primary_text, "/{}", prefix);
            }
        } else {
            primary_text.set("No active Ethernet interface");
        }
        let service = if self.error.is_empty() {
            "Network service"
        } else {
            self.error.as_str()
        };
        let badge = if self.error.is_empty() {
            BadgeKind::Ok
        } else {
            BadgeKind::Warn
        };
        StatusBadge::new(summary.x + 10, summary.y + 9, badge)
            .with_label(service)
            .with_font(&Typography::UI_SMALL)
            .draw(canvas, theme);
        let mut count_text = FixedStr::<24>::empty();
        let _ = write!(&mut count_text, "{} interface(s)", count);
        draw_text(
            canvas,
            primary_text.as_str(),
            summary.x + 10,
            summary.y + 23,
            16,
            FontRole::UiRegular,
            theme.text,
        );
        sun_font::draw_text_right(
            canvas,
            summary,
            count_text.as_str(),
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            10,
        );
        if self.stale {
            draw_text(
                canvas,
                "Stale",
                summary.right() - 68,
                summary.y + 6,
                14,
                FontRole::UiSmall,
                theme.warn,
            );
        }

        let mut y = 108i32;
        for interface in self.interfaces() {
            let expanded = self.is_expanded(interface);
            let height = group_height(interface, expanded);
            let rect = Rect::new(14, y, win_w.saturating_sub(28), height);
            self.draw_group(canvas, theme, interface, rect, expanded);
            y += height as i32 + 8;
            if y > win_h as i32 - 48 {
                break;
            }
        }
        if count == 0 {
            draw_text(
                canvas,
                if self.error.is_empty() {
                    "No supported interfaces reported"
                } else {
                    "Network service unavailable"
                },
                24,
                116,
                18,
                FontRole::UiRegular,
                theme.text_dim,
            );
        }
    }

    fn draw_group(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        interface: &InterfaceSnapshot,
        rect: Rect,
        expanded: bool,
    ) {
        let (status, badge) = link_status(interface);
        let title = type_label(interface);
        let group = DisclosureGroup::new(rect, title)
            .with_subtitle(interface.name())
            .with_status(status, badge)
            .with_state(DisclosureState {
                expanded,
                focused: self.focused_id == Some(interface.id),
            })
            .with_font(&Typography::UI_REGULAR);
        group.draw(canvas, theme);
        if !expanded {
            return;
        }

        let content = group.content_rect();
        let mut address = FixedStr::<32>::empty();
        let mut mask = FixedStr::<24>::empty();
        let mut gateway = FixedStr::<24>::empty();
        let mut dns = FixedStr::<24>::empty();
        let ipv4_status = if let Some((value, prefix)) = interface.ipv4_address {
            push_ipv4(&mut address, value);
            let _ = write!(&mut address, "/{}", prefix);
            push_ipv4(&mut mask, prefix_to_mask(prefix));
            "IPv4 Configured"
        } else {
            address.set("Not configured");
            mask.set("Not available");
            "IPv4 Not Configured"
        };
        option_ipv4(&mut gateway, interface.gateway, "Not configured");
        option_ipv4(&mut dns, interface.dns_server, "Not configured");
        if interface.is_loopback() {
            let rows = [
                PropertyRow {
                    label: "Operational state",
                    value: status,
                },
                PropertyRow {
                    label: "IPv4 address",
                    value: address.as_str(),
                },
                PropertyRow {
                    label: "Local traffic",
                    value: "Traffic remains local to this system",
                },
            ];
            PropertyGrid::new(content, &rows)
                .with_fonts(&Typography::UI_SMALL, &Typography::UI_REGULAR)
                .draw(canvas, theme);
        } else {
            let admin = if matches!(interface.administrative_state, AdminState::Enabled) {
                "Enabled"
            } else {
                "Disabled"
            };
            let rows = [
                PropertyRow {
                    label: "Link",
                    value: status,
                },
                PropertyRow {
                    label: "IPv4",
                    value: ipv4_status,
                },
                PropertyRow {
                    label: "Address / prefix",
                    value: address.as_str(),
                },
                PropertyRow {
                    label: "Subnet mask",
                    value: mask.as_str(),
                },
                PropertyRow {
                    label: "Default gateway",
                    value: gateway.as_str(),
                },
                PropertyRow {
                    label: "DNS server",
                    value: dns.as_str(),
                },
                PropertyRow {
                    label: "Administrative state",
                    value: admin,
                },
            ];
            PropertyGrid::new(content, &rows)
                .with_fonts(&Typography::UI_SMALL, &Typography::UI_REGULAR)
                .draw(canvas, theme);
        }
    }

    pub fn update(&mut self, event: Event, win_w: u32, win_h: u32) -> NetworkAction {
        if let Event::FocusChanged { focused } = event {
            self.window_focused = focused;
            return NetworkAction::None;
        }
        if let Event::KeyPress {
            keycode,
            pressed: true,
            ..
        } = event
        {
            match keycode {
                KEY_ESC => return NetworkAction::Back,
                KEY_UP | KEY_DOWN => {
                    let ids: [Option<InterfaceId>; 8] = {
                        let mut values = [None; 8];
                        for (index, interface) in self.interfaces().take(values.len()).enumerate() {
                            values[index] = Some(interface.id);
                        }
                        values
                    };
                    if let Some(current) = self.focused_id {
                        if let Some(index) = ids.iter().position(|id| *id == Some(current)) {
                            let next = if keycode == KEY_UP {
                                index.saturating_sub(1)
                            } else {
                                (index + 1).min(ids.len().saturating_sub(1))
                            };
                            if let Some(id) = ids[next] {
                                self.focused_id = Some(id);
                            }
                        }
                    }
                    return NetworkAction::None;
                }
                _ => {}
            }
        }
        if let Event::Click { x, y } = event {
            let point = Point::new(x, y);
            if back_rect(win_h).contains(point) {
                return NetworkAction::Back;
            }
            if refresh_rect(win_w, win_h).contains(point) {
                self.refresh();
                return NetworkAction::None;
            }
        }

        let mut y = 108i32;
        let target = self.interfaces().find_map(|interface| {
            let expanded = self.is_expanded(interface);
            let rect = Rect::new(
                14,
                y,
                win_w.saturating_sub(28),
                group_height(interface, expanded),
            );
            y += rect.h as i32 + 8;
            let group = DisclosureGroup::new(rect, type_label(interface))
                .with_state(DisclosureState {
                    expanded,
                    focused: self.focused_id == Some(interface.id),
                })
                .with_font(&Typography::UI_REGULAR);
            (group.handle_event(event, self.window_focused) == DisclosureEvent::Toggled)
                .then_some(*interface)
        });
        if let Some(interface) = target {
            self.toggle(&interface);
        }
        NetworkAction::None
    }
}

fn group_height(interface: &InterfaceSnapshot, expanded: bool) -> u32 {
    if !expanded {
        return DisclosureGroup::HEADER_HEIGHT;
    }
    if interface.is_loopback() {
        128
    } else {
        218
    }
}

fn back_rect(win_h: u32) -> Rect {
    Rect::new(16, win_h as i32 - 38, 76, 26)
}
fn refresh_rect(win_w: u32, win_h: u32) -> Rect {
    Rect::new(win_w as i32 - 104, win_h as i32 - 38, 88, 26)
}

fn type_label(interface: &InterfaceSnapshot) -> &'static str {
    if interface.is_loopback() {
        "Loopback"
    } else {
        "Ethernet"
    }
}

fn link_status(interface: &InterfaceSnapshot) -> (&'static str, BadgeKind) {
    match interface.operational_state {
        LinkState::Up | LinkState::Carrier => ("Link Up", BadgeKind::Ok),
        LinkState::NoCarrier => ("No Carrier", BadgeKind::Warn),
        LinkState::Down => ("Link Down", BadgeKind::Danger),
        LinkState::Unknown => ("Unknown", BadgeKind::Dim),
    }
}

fn error_label(error: SnapshotError) -> &'static str {
    match error {
        SnapshotError::ServiceUnavailable => "Service unavailable",
        SnapshotError::Timeout => "Network service timed out",
        SnapshotError::Transport => "Network service unavailable",
        SnapshotError::Allocation => "Not enough memory for network state",
        SnapshotError::Malformed => "Network service returned invalid data",
    }
}

fn push_ipv4<const N: usize>(out: &mut FixedStr<N>, address: [u8; 4]) {
    let _ = write!(
        out,
        "{}.{}.{}.{}",
        address[0], address[1], address[2], address[3]
    );
}

fn option_ipv4<const N: usize>(out: &mut FixedStr<N>, address: Option<[u8; 4]>, absent: &str) {
    if let Some(address) = address {
        push_ipv4(out, address);
    } else {
        out.set(absent);
    }
}

fn prefix_to_mask(prefix: u8) -> [u8; 4] {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    mask.to_be_bytes()
}

fn draw_text(
    canvas: &mut Canvas,
    text: &str,
    x: i32,
    y: i32,
    height: u32,
    role: FontRole,
    color: sunlight_ui::Color,
) {
    sun_font::draw_text_vcenter(canvas, text, x, y, height, &TextStyle::new(role, color));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_masks_keep_network_byte_order() {
        assert_eq!(prefix_to_mask(0), [0, 0, 0, 0]);
        assert_eq!(prefix_to_mask(8), [255, 0, 0, 0]);
        assert_eq!(prefix_to_mask(24), [255, 255, 255, 0]);
        assert_eq!(prefix_to_mask(32), [255, 255, 255, 255]);
    }
}
