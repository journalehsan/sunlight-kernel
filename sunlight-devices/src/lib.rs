#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};
use sunlight_deviced::{device_name, generic_non_pci_name, InventoryRecord};
use sunlight_ipc::{HardwareBus, HardwareState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceClassId {
    Storage,
    Network,
    Display,
    Audio,
    Input,
    Usb,
    System,
    Bridge,
    Communication,
    Other(u8, u8),
}

impl DeviceClassId {
    pub const fn from_record(record: InventoryRecord) -> Self {
        let summary = record.summary;
        if matches!(summary.bus(), HardwareBus::Ps2) {
            return Self::Input;
        }
        match (summary.class(), summary.subclass()) {
            (0x01, _) => Self::Storage,
            (0x02, _) => Self::Network,
            (0x03, _) => Self::Display,
            (0x04, _) => Self::Audio,
            (0x05, _) => Self::System,
            (0x06, _) => Self::Bridge,
            (0x07, _) => Self::Communication,
            (0x09, _) => Self::Input,
            (0x0c, 0x03) => Self::Usb,
            (0x08 | 0x0b | 0x0c | 0x0d, _) => Self::System,
            (class, subclass) => Self::Other(class, subclass),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Storage => "Storage controllers",
            Self::Network => "Network adapters",
            Self::Display => "Display adapters",
            Self::Audio => "Audio devices",
            Self::Input => "Input devices",
            Self::Usb => "USB controllers",
            Self::System => "System devices",
            Self::Bridge => "Bridges",
            Self::Communication => "Communication devices",
            Self::Other(_, _) => "Other devices",
        }
    }

    pub const fn order(self) -> (u8, u8, u8) {
        match self {
            Self::Display => (0, 0, 0),
            Self::Network => (1, 0, 0),
            Self::Storage => (2, 0, 0),
            Self::Audio => (3, 0, 0),
            Self::Input => (4, 0, 0),
            Self::Usb => (5, 0, 0),
            Self::System => (6, 0, 0),
            Self::Bridge => (7, 0, 0),
            Self::Communication => (8, 0, 0),
            Self::Other(class, subclass) => (255, class, subclass),
        }
    }

    pub const fn stable_key(self) -> u64 {
        match self {
            Self::Storage => 0x01,
            Self::Network => 0x02,
            Self::Display => 0x03,
            Self::Audio => 0x04,
            Self::Input => 0x09,
            Self::Usb => 0x0c03,
            Self::System => 0x100,
            Self::Bridge => 0x06,
            Self::Communication => 0x07,
            Self::Other(class, subclass) => 0xff00_0000 | ((class as u64) << 8) | subclass as u64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TreeNodeId {
    Class(u64),
    Device(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceGroup {
    pub class: DeviceClassId,
    pub devices: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySnapshot {
    pub records: Vec<InventoryRecord>,
    pub groups: Vec<DeviceGroup>,
}

impl InventorySnapshot {
    pub fn new(mut records: Vec<InventoryRecord>) -> Self {
        records.sort_by(|left, right| {
            sunlight_deviced::inventory_order_key(left.key())
                .cmp(&sunlight_deviced::inventory_order_key(right.key()))
                .then_with(|| device_display_name(*left).cmp(&device_display_name(*right)))
                .then_with(|| left.key().cmp(&right.key()))
        });

        let mut groups: Vec<DeviceGroup> = Vec::new();
        for record in &records {
            let class = DeviceClassId::from_record(*record);
            if let Some(group) = groups.iter_mut().find(|group| group.class == class) {
                group.devices.push(record.key());
            } else {
                groups.push(DeviceGroup {
                    class,
                    devices: vec![record.key()],
                });
            }
        }
        groups.sort_by_key(|group| group.class.order());
        Self { records, groups }
    }

    pub fn record(&self, key: u64) -> Option<&InventoryRecord> {
        self.records.iter().find(|record| record.key() == key)
    }

    pub fn contains_class(&self, class_key: u64) -> bool {
        self.groups
            .iter()
            .any(|group| group.class.stable_key() == class_key)
    }

    pub fn contains_device(&self, key: u64) -> bool {
        self.record(key).is_some()
    }

    pub fn status_counts(&self) -> StatusCounts {
        let mut counts = StatusCounts::default();
        counts.total = self.records.len();
        for record in &self.records {
            match record.state() {
                HardwareState::Active => counts.active += 1,
                HardwareState::Loaded => counts.loaded += 1,
                HardwareState::ProbeFailed => counts.probe_failed += 1,
                HardwareState::NoDriver => counts.no_driver += 1,
                HardwareState::Disabled => counts.disabled += 1,
                HardwareState::Unknown => counts.unknown += 1,
            }
        }
        counts
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatusCounts {
    pub total: usize,
    pub active: usize,
    pub loaded: usize,
    pub probe_failed: usize,
    pub no_driver: usize,
    pub disabled: usize,
    pub unknown: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PresentationState {
    pub snapshot: Option<InventorySnapshot>,
    pub selected_device: Option<u64>,
    pub expanded_classes: Vec<u64>,
    pub refresh_error: Option<String>,
}

impl PresentationState {
    pub fn apply_snapshot(&mut self, snapshot: InventorySnapshot) {
        self.expanded_classes
            .retain(|class_key| snapshot.contains_class(*class_key));
        if self.snapshot.is_none() {
            self.expanded_classes = snapshot
                .groups
                .iter()
                .map(|group| group.class.stable_key())
                .collect();
        }
        if self
            .selected_device
            .is_some_and(|key| !snapshot.contains_device(key))
        {
            self.selected_device = None;
        }
        self.snapshot = Some(snapshot);
        self.refresh_error = None;
    }

    pub fn fail_refresh(&mut self, message: impl Into<String>) {
        self.refresh_error = Some(message.into());
    }

    pub fn select(&mut self, key: Option<u64>) {
        self.selected_device = key.filter(|key| {
            self.snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.contains_device(*key))
        });
    }

    pub fn selected_record(&self) -> Option<&InventoryRecord> {
        self.snapshot.as_ref()?.record(self.selected_device?)
    }
}

pub fn device_display_name(record: InventoryRecord) -> String {
    let summary = record.summary;
    let known = device_name(
        summary.bus(),
        summary.vendor_id().unwrap_or(0),
        summary.device_id().unwrap_or(0),
        summary.class(),
    );
    if !known.is_empty() {
        return String::from(known);
    }
    if !matches!(summary.bus(), HardwareBus::Pci) {
        return String::from(generic_non_pci_name(summary));
    }
    format!(
        "PCI device {:04x}:{:04x}",
        summary.vendor_id().unwrap_or(0),
        summary.device_id().unwrap_or(0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_deviced::InventorySummary;
    use sunlight_ipc::{HardwareFailureStage, HardwareState};

    fn record(key: u64, class: u8, subclass: u8, state: HardwareState) -> InventoryRecord {
        InventoryRecord {
            summary: InventorySummary {
                key,
                identity: ((class as u64) << 32) | ((subclass as u64) << 40),
                driver: 0,
                state,
                failure_stage: HardwareFailureStage::None,
                total: 1,
            },
            subsystem: 0,
            matched_driver: 0,
            bound_driver: 0,
            diagnostic_state: state,
            error_code: 0,
            irq: None,
            bars: [0; 6],
        }
    }

    #[test]
    fn groups_only_present_classes_and_every_device_once() {
        let snapshot = InventorySnapshot::new(vec![
            record(3, 0x03, 0, HardwareState::Active),
            record(2, 0x02, 0, HardwareState::NoDriver),
        ]);
        assert_eq!(snapshot.groups.len(), 2);
        let keys: Vec<u64> = snapshot
            .groups
            .iter()
            .flat_map(|group| group.devices.iter().copied())
            .collect();
        assert_eq!(keys.len(), snapshot.records.len());
        assert!(keys.contains(&2));
        assert!(keys.contains(&3));
    }

    #[test]
    fn unknown_devices_remain_visible_and_last() {
        let unknown_key = sunlight_ipc::HardwareInventoryRecord::pci_key(0, 0, 2, 0);
        let snapshot = InventorySnapshot::new(vec![
            record(unknown_key, 0xff, 1, HardwareState::Unknown),
            record(3, 0x03, 0, HardwareState::Active),
        ]);
        assert!(matches!(
            snapshot.groups.last().unwrap().class,
            DeviceClassId::Other(0xff, 1)
        ));
        assert_eq!(snapshot.groups.last().unwrap().devices, vec![unknown_key]);
    }

    #[test]
    fn group_and_device_ordering_is_deterministic() {
        let first = InventorySnapshot::new(vec![
            record(30, 0x02, 0, HardwareState::Active),
            record(10, 0x03, 0, HardwareState::Active),
            record(20, 0x02, 0, HardwareState::Active),
        ]);
        let second = InventorySnapshot::new(vec![
            record(20, 0x02, 0, HardwareState::Active),
            record(10, 0x03, 0, HardwareState::Active),
            record(30, 0x02, 0, HardwareState::Active),
        ]);
        assert_eq!(first.groups, second.groups);
        assert_eq!(first.records, second.records);
    }

    #[test]
    fn stable_ids_do_not_depend_on_vector_indexes() {
        assert_eq!(
            TreeNodeId::Class(DeviceClassId::Display.stable_key()),
            TreeNodeId::Class(3)
        );
        assert_eq!(TreeNodeId::Device(0x1234), TreeNodeId::Device(0x1234));
    }

    #[test]
    fn selection_resolves_record_and_survives_refresh_by_key() {
        let mut state = PresentationState::default();
        state.apply_snapshot(InventorySnapshot::new(vec![
            record(10, 0x03, 0, HardwareState::Active),
            record(20, 0x02, 0, HardwareState::Active),
        ]));
        state.select(Some(20));
        state.apply_snapshot(InventorySnapshot::new(vec![
            record(20, 0x02, 0, HardwareState::NoDriver),
            record(30, 0x01, 0, HardwareState::Active),
        ]));
        assert_eq!(state.selected_record().unwrap().key(), 20);
        assert_eq!(
            state.selected_record().unwrap().state(),
            HardwareState::NoDriver
        );
    }

    #[test]
    fn refresh_preserves_expansion_and_clears_missing_selection() {
        let mut state = PresentationState::default();
        state.apply_snapshot(InventorySnapshot::new(vec![
            record(10, 0x03, 0, HardwareState::Active),
            record(20, 0x02, 0, HardwareState::Active),
        ]));
        state.expanded_classes = vec![DeviceClassId::Network.stable_key()];
        state.select(Some(20));
        state.apply_snapshot(InventorySnapshot::new(vec![record(
            30,
            0x02,
            0,
            HardwareState::Active,
        )]));
        assert_eq!(
            state.expanded_classes,
            vec![DeviceClassId::Network.stable_key()]
        );
        assert_eq!(state.selected_device, None);
    }

    #[test]
    fn failed_refresh_preserves_last_valid_snapshot() {
        let mut state = PresentationState::default();
        state.apply_snapshot(InventorySnapshot::new(vec![record(
            10,
            0x03,
            0,
            HardwareState::Active,
        )]));
        state.fail_refresh("deviced unavailable");
        assert_eq!(state.snapshot.as_ref().unwrap().records.len(), 1);
        assert_eq!(state.refresh_error.as_deref(), Some("deviced unavailable"));
    }

    #[test]
    fn canonical_states_have_truthful_display_labels() {
        assert_eq!(
            sunlight_deviced::state_display_label(HardwareState::Active),
            "Active"
        );
        assert_eq!(
            sunlight_deviced::state_display_label(HardwareState::ProbeFailed),
            "Probe failed"
        );
        assert_eq!(
            sunlight_deviced::state_display_label(HardwareState::NoDriver),
            "Without driver"
        );
        assert_eq!(
            sunlight_deviced::state_display_label(HardwareState::Unknown),
            "Unknown"
        );
    }

    #[test]
    fn missing_optional_fields_are_safe() {
        let device = record(
            sunlight_ipc::HardwareInventoryRecord::pci_key(0, 0, 3, 0),
            0x03,
            0,
            HardwareState::NoDriver,
        );
        let snapshot = InventorySnapshot::new(vec![device]);
        let selected = snapshot.record(device.key()).unwrap();
        assert_eq!(selected.subsystem_vendor_id(), None);
        assert_eq!(selected.subsystem_device_id(), None);
        assert_eq!(selected.irq, None);
        assert!(selected.bars.iter().all(|bar| *bar == 0));
        assert!(!device_display_name(*selected).is_empty());
    }
}
