//! Small typed read-only client for networkd.
//!
//! This module deliberately owns protocol decoding, validation and stable
//! interface identity.  Presentation clients such as `networkctl`, Control
//! Panel, and a future panel applet format this model independently.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use sunlight_ipc::{
    ipc_call_timeout, monotonic_millis, nameserver_lookup_timeout, unpack_iface_summary,
    AdminState, InterfaceId, InterfaceKind, IpcMsg, LinkState, NetworkdMsg,
};

/// The daemon currently publishes at most eight interface records.
pub const MAX_INTERFACES: usize = 8;
const LOOKUP_TIMEOUT_MS: u64 = 50;
const REQUEST_TIMEOUT_MS: u64 = 40;

/// A complete, validated view of networkd state. `service_generation` is the
/// capability generation: it changes when the registered service is replaced.
#[derive(Clone, Debug)]
pub struct NetworkSnapshot {
    pub service_generation: u64,
    pub captured_monotonic_time: u64,
    pub interfaces: Vec<InterfaceSnapshot>,
}

/// A read-only interface record.  Fields absent from the v0 protocol are not
/// represented; callers must not infer MAC, MTU, speed, or traffic counters.
#[derive(Clone, Copy, Debug)]
pub struct InterfaceSnapshot {
    pub id: InterfaceId,
    pub name: [u8; 8],
    pub name_len: u8,
    pub kind: InterfaceKind,
    pub administrative_state: AdminState,
    pub operational_state: LinkState,
    pub configuration_mode: sunlight_ipc::IpConfigMode,
    pub ipv4_address: Option<([u8; 4], u8)>,
    pub gateway: Option<[u8; 4]>,
    pub dns_server: Option<[u8; 4]>,
    pub is_default: bool,
    pub priority: i32,
}

impl InterfaceSnapshot {
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("?")
    }

    pub const fn is_loopback(&self) -> bool {
        matches!(self.kind, InterfaceKind::Loopback)
    }

    pub const fn is_ethernet_like(&self) -> bool {
        matches!(
            self.kind,
            InterfaceKind::Ethernet | InterfaceKind::VirtioNet | InterfaceKind::Vmxnet3
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionKind {
    Ethernet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerivedConnectionState {
    Connected,
    Configuring,
    CableDisconnected,
    Unavailable,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkPanelSummary {
    pub service_generation: u64,
    pub captured_monotonic_time: u64,
    pub connection_kind: Option<ConnectionKind>,
    pub state: DerivedConnectionState,
    pub interface_name: [u8; 8],
    pub interface_name_len: u8,
    pub administrative_state: AdminState,
    pub operational_state: LinkState,
    pub configuration_mode: sunlight_ipc::IpConfigMode,
    pub ipv4_address: Option<([u8; 4], u8)>,
    pub gateway: Option<[u8; 4]>,
    pub dns_server: Option<[u8; 4]>,
    pub is_default: bool,
    pub priority: i32,
}

impl NetworkPanelSummary {
    pub fn interface_name(&self) -> &str {
        core::str::from_utf8(&self.interface_name[..self.interface_name_len as usize])
            .unwrap_or("?")
    }

    pub const fn has_interface(&self) -> bool {
        self.connection_kind.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    ServiceUnavailable,
    Timeout,
    Transport,
    Allocation,
    Malformed,
}

/// Stateless bounded client. Each request obtains a fresh service capability,
/// so a daemon restart cannot publish an old service generation.
pub struct NetworkClient;

impl NetworkClient {
    pub const fn new() -> Self {
        Self
    }

    /// Fetch and validate all records before returning any of them. On every
    /// error the caller keeps its prior snapshot intact.
    pub fn snapshot(&self) -> Result<NetworkSnapshot, SnapshotError> {
        let cap = nameserver_lookup_timeout("networkd", LOOKUP_TIMEOUT_MS)
            .ok_or(SnapshotError::ServiceUnavailable)?;
        let first = call_list(cap, 0)?;
        if first.label == NetworkdMsg::ERROR {
            // `list_one` reports a zero count this way because the compact v0
            // protocol has no independent list header.
            if first.words[0] == NetworkdMsg::ERR_NOT_FOUND && first.words[1] == 0 {
                return Ok(NetworkSnapshot {
                    service_generation: cap.0,
                    captured_monotonic_time: monotonic_millis(),
                    interfaces: Vec::new(),
                });
            }
            return Err(SnapshotError::Malformed);
        }
        let first_summary = unpack_iface_summary(&first).ok_or(SnapshotError::Malformed)?;
        let total = first_summary.total as usize;
        if total == 0 || total > MAX_INTERFACES {
            return Err(SnapshotError::Malformed);
        }

        let mut interfaces = Vec::new();
        interfaces
            .try_reserve_exact(total)
            .map_err(|_| SnapshotError::Allocation)?;
        push_summary(&mut interfaces, first_summary, total)?;
        for index in 1..total {
            let reply = call_list(cap, index as u64)?;
            let summary = unpack_iface_summary(&reply).ok_or(SnapshotError::Malformed)?;
            if summary.total as usize != total {
                return Err(SnapshotError::Malformed);
            }
            push_summary(&mut interfaces, summary, total)?;
        }
        Ok(NetworkSnapshot {
            service_generation: cap.0,
            captured_monotonic_time: monotonic_millis(),
            interfaces,
        })
    }

    pub fn panel_summary(&self) -> Result<NetworkPanelSummary, SnapshotError> {
        let snapshot = self.snapshot()?;
        Ok(NetworkPanelSummary::from_snapshot(&snapshot))
    }
}

impl NetworkPanelSummary {
    pub fn from_snapshot(snapshot: &NetworkSnapshot) -> Self {
        let primary = snapshot
            .interfaces
            .iter()
            .find(|interface| interface.is_ethernet_like() && interface.is_default)
            .or_else(|| {
                snapshot
                    .interfaces
                    .iter()
                    .find(|interface| interface.is_ethernet_like())
            })
            .copied();
        if let Some(interface) = primary {
            let carrier_present = matches!(
                interface.operational_state,
                LinkState::Up | LinkState::Carrier
            );
            let address_configured = interface.ipv4_address.is_some();
            let state = if carrier_present && address_configured {
                DerivedConnectionState::Connected
            } else if !carrier_present {
                DerivedConnectionState::CableDisconnected
            } else if matches!(
                interface.configuration_mode,
                sunlight_ipc::IpConfigMode::Dhcp | sunlight_ipc::IpConfigMode::Static
            ) || matches!(interface.administrative_state, AdminState::Enabled)
            {
                DerivedConnectionState::Configuring
            } else {
                DerivedConnectionState::Error
            };
            Self {
                service_generation: snapshot.service_generation,
                captured_monotonic_time: snapshot.captured_monotonic_time,
                connection_kind: Some(ConnectionKind::Ethernet),
                state,
                interface_name: interface.name,
                interface_name_len: interface.name_len,
                administrative_state: interface.administrative_state,
                operational_state: interface.operational_state,
                configuration_mode: interface.configuration_mode,
                ipv4_address: interface.ipv4_address,
                gateway: interface.gateway,
                dns_server: interface.dns_server,
                is_default: interface.is_default,
                priority: interface.priority,
            }
        } else {
            Self {
                service_generation: snapshot.service_generation,
                captured_monotonic_time: snapshot.captured_monotonic_time,
                connection_kind: None,
                state: DerivedConnectionState::Unavailable,
                interface_name: [0; 8],
                interface_name_len: 0,
                administrative_state: AdminState::Disabled,
                operational_state: LinkState::Unknown,
                configuration_mode: sunlight_ipc::IpConfigMode::None,
                ipv4_address: None,
                gateway: None,
                dns_server: None,
                is_default: false,
                priority: 0,
            }
        }
    }
}

fn call_list(
    cap: sunlight_ipc::CapabilityToken,
    index: u64,
) -> Result<sunlight_ipc::IpcMsg, SnapshotError> {
    ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetworkdMsg::LIST_INTERFACES).word(0, index),
        REQUEST_TIMEOUT_MS,
    )
    .map_err(|err| match err {
        sunlight_ipc::IpcCallError::Timeout => SnapshotError::Timeout,
        _ => SnapshotError::Transport,
    })
}

fn push_summary(
    out: &mut Vec<InterfaceSnapshot>,
    summary: sunlight_ipc::IfaceSummary,
    expected_total: usize,
) -> Result<(), SnapshotError> {
    if summary.id == 0 || summary.total as usize != expected_total || summary.prefix > 32 {
        return Err(SnapshotError::Malformed);
    }
    let mut name = [0u8; 8];
    for (index, byte) in name.iter_mut().enumerate() {
        *byte = ((summary.name >> (index * 8)) & 0xff) as u8;
    }
    let name_len = name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(name.len());
    if name_len == 0 || !name[..name_len].iter().all(u8::is_ascii_graphic) {
        return Err(SnapshotError::Malformed);
    }
    if out.iter().any(|interface| interface.id == summary.id) {
        return Err(SnapshotError::Malformed);
    }
    let ipv4_address = if summary.addr == [0; 4] {
        None
    } else {
        Some((summary.addr, summary.prefix))
    };
    out.push(InterfaceSnapshot {
        id: summary.id,
        name,
        name_len: name_len as u8,
        kind: summary.kind,
        administrative_state: summary.admin,
        operational_state: summary.link,
        configuration_mode: summary.mode,
        ipv4_address,
        gateway: (summary.gw != [0; 4]).then_some(summary.gw),
        dns_server: (summary.dns != [0; 4]).then_some(summary.dns),
        is_default: summary.is_default,
        priority: summary.priority,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_ipc::{pack_short_name, AdminState, IfaceSummary, IpConfigMode};

    fn summary(id: u64, name: &str, prefix: u8) -> IfaceSummary {
        IfaceSummary {
            id,
            name: pack_short_name(name),
            kind: InterfaceKind::Ethernet,
            admin: AdminState::Enabled,
            link: LinkState::Carrier,
            mode: IpConfigMode::Dhcp,
            addr: [192, 0, 2, 1],
            prefix,
            gw: [192, 0, 2, 254],
            dns: [1, 1, 1, 1],
            priority: 0,
            is_default: true,
            total: 1,
        }
    }

    #[test]
    fn decodes_ethernet_and_loopback_without_name_identity() {
        let mut values = Vec::new();
        values.try_reserve_exact(2).unwrap();
        push_summary(&mut values, summary(7, "eth0", 24), 1).unwrap();
        assert_eq!(values[0].id, 7);
        assert_eq!(values[0].name(), "eth0");
    }

    #[test]
    fn rejects_bad_prefix_duplicate_id_and_bad_name() {
        let mut values = Vec::new();
        let bad_prefix = summary(1, "eth0", 33);
        assert_eq!(
            push_summary(&mut values, bad_prefix, 1),
            Err(SnapshotError::Malformed)
        );
        push_summary(&mut values, summary(1, "eth0", 24), 1).unwrap();
        assert_eq!(
            push_summary(&mut values, summary(1, "eth1", 24), 1),
            Err(SnapshotError::Malformed)
        );
    }

    #[test]
    fn panel_summary_prefers_default_ethernet_and_derives_connected() {
        let snapshot = NetworkSnapshot {
            service_generation: 7,
            captured_monotonic_time: 11,
            interfaces: vec![
                InterfaceSnapshot {
                    id: 1,
                    name: *b"eth0\0\0\0\0",
                    name_len: 4,
                    kind: InterfaceKind::Ethernet,
                    administrative_state: AdminState::Enabled,
                    operational_state: LinkState::Carrier,
                    configuration_mode: IpConfigMode::Dhcp,
                    ipv4_address: Some(([192, 168, 1, 24], 24)),
                    gateway: Some([192, 168, 1, 1]),
                    dns_server: None,
                    is_default: true,
                    priority: 0,
                },
                InterfaceSnapshot {
                    id: 2,
                    name: *b"eth1\0\0\0\0",
                    name_len: 4,
                    kind: InterfaceKind::Ethernet,
                    administrative_state: AdminState::Enabled,
                    operational_state: LinkState::NoCarrier,
                    configuration_mode: IpConfigMode::Dhcp,
                    ipv4_address: None,
                    gateway: None,
                    dns_server: None,
                    is_default: false,
                    priority: 10,
                },
            ],
        };
        let summary = NetworkPanelSummary::from_snapshot(&snapshot);
        assert_eq!(summary.connection_kind, Some(ConnectionKind::Ethernet));
        assert_eq!(summary.interface_name(), "eth0");
        assert_eq!(summary.state, DerivedConnectionState::Connected);
    }

    #[test]
    fn panel_summary_reports_unavailable_without_ethernet() {
        let snapshot = NetworkSnapshot {
            service_generation: 1,
            captured_monotonic_time: 2,
            interfaces: vec![InterfaceSnapshot {
                id: 1,
                name: *b"lo\0\0\0\0\0\0",
                name_len: 2,
                kind: InterfaceKind::Loopback,
                administrative_state: AdminState::Enabled,
                operational_state: LinkState::Up,
                configuration_mode: IpConfigMode::Static,
                ipv4_address: Some(([127, 0, 0, 1], 8)),
                gateway: None,
                dns_server: None,
                is_default: false,
                priority: 0,
            }],
        };
        let summary = NetworkPanelSummary::from_snapshot(&snapshot);
        assert_eq!(summary.state, DerivedConnectionState::Unavailable);
        assert!(!summary.has_interface());
    }
}
