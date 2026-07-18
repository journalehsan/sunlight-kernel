#![no_std]

use core::fmt::Write;
use sunlight_ipc::{
    ipc_call, ipc_call_timeout, CapabilityToken, DevicedMsg, HardwareBus, HardwareFailureStage,
    HardwareState, IpcCallError, IpcMsg,
};

pub const DEFAULT_INVENTORY_TIMEOUT_MS: u64 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventorySummary {
    pub key: u64,
    pub identity: u64,
    pub driver: u64,
    pub state: HardwareState,
    pub failure_stage: HardwareFailureStage,
    pub total: usize,
}

impl InventorySummary {
    pub const fn bus(self) -> HardwareBus {
        HardwareBus::from_u64(self.key & 0xff)
    }

    pub const fn class(self) -> u8 {
        ((self.identity >> 32) & 0xff) as u8
    }

    pub const fn subclass(self) -> u8 {
        ((self.identity >> 40) & 0xff) as u8
    }

    pub const fn programming_interface(self) -> u8 {
        ((self.identity >> 48) & 0xff) as u8
    }

    pub const fn revision(self) -> u8 {
        ((self.identity >> 56) & 0xff) as u8
    }

    pub const fn vendor_id(self) -> Option<u16> {
        if matches!(self.bus(), HardwareBus::Pci) {
            Some((self.identity & 0xffff) as u16)
        } else {
            None
        }
    }

    pub const fn device_id(self) -> Option<u16> {
        if matches!(self.bus(), HardwareBus::Pci) {
            Some(((self.identity >> 16) & 0xffff) as u16)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventoryRecord {
    pub summary: InventorySummary,
    pub subsystem: u64,
    pub matched_driver: u64,
    pub bound_driver: u64,
    pub diagnostic_state: HardwareState,
    pub error_code: u64,
    pub irq: Option<u64>,
    pub bars: [u64; 6],
}

impl InventoryRecord {
    pub const fn key(self) -> u64 {
        self.summary.key
    }

    pub const fn state(self) -> HardwareState {
        self.summary.state
    }

    pub const fn failure_stage(self) -> HardwareFailureStage {
        self.summary.failure_stage
    }

    pub const fn subsystem_vendor_id(self) -> Option<u16> {
        if self.subsystem == 0 || self.subsystem == 0xffff_ffff {
            None
        } else {
            Some((self.subsystem & 0xffff) as u16)
        }
    }

    pub const fn subsystem_device_id(self) -> Option<u16> {
        if self.subsystem == 0 || self.subsystem == 0xffff_ffff {
            None
        } else {
            Some(((self.subsystem >> 16) & 0xffff) as u16)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryClientError {
    Transport(IpcCallError),
    NotFound,
    MalformedReply,
}

pub fn list(cap: CapabilityToken, index: usize) -> Option<InventorySummary> {
    decode_summary(ipc_call(
        cap,
        IpcMsg::with_label(DevicedMsg::LIST_INVENTORY).word(0, index as u64),
    ))
}

pub fn get(cap: CapabilityToken, key: u64) -> Option<InventorySummary> {
    decode_summary(ipc_call(
        cap,
        IpcMsg::with_label(DevicedMsg::GET_INVENTORY).word(0, key),
    ))
}

pub fn field(cap: CapabilityToken, key: u64, field: u64) -> Option<[u64; 3]> {
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(DevicedMsg::GET_INVENTORY_FIELD)
            .word(0, key)
            .word(1, field),
    );
    if reply.label == DevicedMsg::INVENTORY_REPLY && reply.words[0] == key {
        Some([reply.words[1], reply.words[2], reply.words[3]])
    } else {
        None
    }
}

pub fn list_timeout(
    cap: CapabilityToken,
    index: usize,
    timeout_ms: u64,
) -> Result<InventorySummary, InventoryClientError> {
    decode_summary_result(
        ipc_call_timeout(
            cap,
            IpcMsg::with_label(DevicedMsg::LIST_INVENTORY).word(0, index as u64),
            timeout_ms,
        )
        .map_err(InventoryClientError::Transport)?,
    )
}

pub fn get_timeout(
    cap: CapabilityToken,
    key: u64,
    timeout_ms: u64,
) -> Result<InventorySummary, InventoryClientError> {
    decode_summary_result(
        ipc_call_timeout(
            cap,
            IpcMsg::with_label(DevicedMsg::GET_INVENTORY).word(0, key),
            timeout_ms,
        )
        .map_err(InventoryClientError::Transport)?,
    )
}

pub fn field_timeout(
    cap: CapabilityToken,
    key: u64,
    field: u64,
    timeout_ms: u64,
) -> Result<[u64; 3], InventoryClientError> {
    let reply = ipc_call_timeout(
        cap,
        IpcMsg::with_label(DevicedMsg::GET_INVENTORY_FIELD)
            .word(0, key)
            .word(1, field),
        timeout_ms,
    )
    .map_err(InventoryClientError::Transport)?;
    if reply.label == DevicedMsg::ERROR && reply.words[0] == DevicedMsg::ERR_NOT_FOUND {
        return Err(InventoryClientError::NotFound);
    }
    if reply.label != DevicedMsg::INVENTORY_REPLY || reply.word_count < 2 || reply.words[0] != key {
        return Err(InventoryClientError::MalformedReply);
    }
    Ok([reply.words[1], reply.words[2], reply.words[3]])
}

pub fn load_record_timeout(
    cap: CapabilityToken,
    summary: InventorySummary,
    timeout_ms: u64,
) -> Result<InventoryRecord, InventoryClientError> {
    let subsystem = field_timeout(cap, summary.key, DevicedMsg::FIELD_SUBSYSTEM, timeout_ms)?[0];
    let drivers = field_timeout(cap, summary.key, DevicedMsg::FIELD_DRIVERS, timeout_ms)?;
    let diagnostic = field_timeout(cap, summary.key, DevicedMsg::FIELD_DIAGNOSTIC, timeout_ms)?;
    let diagnostic_state = HardwareState::from_u64(diagnostic[0] & 0xff);
    if diagnostic_state != summary.state {
        return Err(InventoryClientError::MalformedReply);
    }
    let irq_value = field_timeout(cap, summary.key, DevicedMsg::FIELD_IRQ, timeout_ms)?[0];
    let mut bars = [0u64; 6];
    for (index, value) in bars.iter_mut().enumerate() {
        *value = field_timeout(
            cap,
            summary.key,
            DevicedMsg::FIELD_BAR0 + index as u64,
            timeout_ms,
        )?[0];
    }
    Ok(InventoryRecord {
        summary,
        subsystem,
        matched_driver: drivers[0],
        bound_driver: drivers[1],
        diagnostic_state,
        error_code: diagnostic[1],
        irq: if irq_value == u64::MAX || irq_value == 0xff {
            None
        } else {
            Some(irq_value)
        },
        bars,
    })
}

pub fn decode_summary(reply: IpcMsg) -> Option<InventorySummary> {
    if reply.label != DevicedMsg::INVENTORY_REPLY || reply.word_count != 4 {
        return None;
    }
    Some(InventorySummary {
        key: reply.words[0],
        identity: reply.words[1],
        driver: reply.words[2],
        state: HardwareState::from_u64(reply.words[3] & 0xff),
        failure_stage: HardwareFailureStage::from_u64((reply.words[3] >> 8) & 0xff),
        total: ((reply.words[3] >> 16) & 0xffff) as usize,
    })
}

pub fn decode_summary_result(reply: IpcMsg) -> Result<InventorySummary, InventoryClientError> {
    if reply.label == DevicedMsg::ERROR && reply.words[0] == DevicedMsg::ERR_NOT_FOUND {
        return Err(InventoryClientError::NotFound);
    }
    decode_summary(reply).ok_or(InventoryClientError::MalformedReply)
}

pub const fn inventory_request_valid(label: u64, word_count: u32, field: u64) -> bool {
    match label {
        DevicedMsg::LIST_INVENTORY | DevicedMsg::GET_INVENTORY => word_count == 1,
        DevicedMsg::GET_INVENTORY_FIELD => {
            word_count == 2
                && matches!(
                    field,
                    DevicedMsg::FIELD_SUBSYSTEM
                        | DevicedMsg::FIELD_DRIVERS
                        | DevicedMsg::FIELD_DIAGNOSTIC
                        | DevicedMsg::FIELD_IRQ
                        | DevicedMsg::FIELD_BAR0..=DevicedMsg::FIELD_BAR5
                )
        }
        _ => false,
    }
}

pub const fn inventory_order_key(key: u64) -> (u8, u16, u8, u8, u8) {
    match HardwareBus::from_u64(key & 0xff) {
        HardwareBus::Pci => (
            0,
            ((key >> 8) & 0xffff) as u16,
            ((key >> 24) & 0xff) as u8,
            ((key >> 32) & 0xff) as u8,
            ((key >> 40) & 0xff) as u8,
        ),
        HardwareBus::Ps2 => (1, 0, 0, ((key >> 8) & 0xff) as u8, 0),
        HardwareBus::Platform => (2, 0, 0, 0, 0),
        HardwareBus::Unknown => (3, 0, 0, 0, 0),
    }
}

pub const fn bus_label(bus: HardwareBus) -> &'static str {
    match bus {
        HardwareBus::Pci => "PCI",
        HardwareBus::Ps2 => "PS2",
        HardwareBus::Platform => "PLAT",
        HardwareBus::Unknown => "?",
    }
}

pub const fn state_label(state: HardwareState) -> &'static str {
    match state {
        HardwareState::Active => "active",
        HardwareState::Loaded => "loaded",
        HardwareState::ProbeFailed => "probe-failed",
        HardwareState::NoDriver => "no-driver",
        HardwareState::Disabled => "disabled",
        HardwareState::Unknown => "unknown",
    }
}

pub const fn state_display_label(state: HardwareState) -> &'static str {
    match state {
        HardwareState::Active => "Active",
        HardwareState::Loaded => "Loaded",
        HardwareState::ProbeFailed => "Probe failed",
        HardwareState::NoDriver => "Without driver",
        HardwareState::Disabled => "Disabled",
        HardwareState::Unknown => "Unknown",
    }
}

pub const fn failure_stage_label(stage: HardwareFailureStage) -> &'static str {
    match stage {
        HardwareFailureStage::None => "none",
        HardwareFailureStage::Match => "driver match",
        HardwareFailureStage::ResourceAllocation => "resource allocation",
        HardwareFailureStage::ResourceMapping => "resource mapping",
        HardwareFailureStage::FeatureNegotiation => "feature negotiation",
        HardwareFailureStage::QueueSetup => "queue setup",
        HardwareFailureStage::DeviceInitialization => "device initialization",
        HardwareFailureStage::DeviceActivation => "device activation",
        HardwareFailureStage::ServiceBinding => "service binding",
        HardwareFailureStage::Unknown => "unknown",
    }
}

pub const fn class_label(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x01, _) => "Storage",
        (0x02, 0x00) => "Network",
        (0x03, _) => "Display",
        (0x04, _) => "Multimedia",
        (0x06, _) => "Bridge",
        (0x0c, 0x03) => "USB",
        (0x0c, _) => "Serial bus",
        (0x09, 0x00) => "Keyboard",
        (0x09, 0x02) => "Mouse",
        _ => "Unknown",
    }
}

pub const fn device_name(bus: HardwareBus, vendor: u16, device: u16, _class: u8) -> &'static str {
    match (bus, vendor, device) {
        (HardwareBus::Ps2, _, _) => "",
        (HardwareBus::Pci, 0x1af4, 0x1000 | 0x1041) => "VirtIO Network Device",
        (HardwareBus::Pci, 0x1af4, 0x1001 | 0x1042) => "VirtIO Block Device",
        (HardwareBus::Pci, 0x1af4, 0x1050) => "VirtIO GPU",
        (HardwareBus::Pci, 0x15ad, 0x07b0) => "VMware VMXNET3 Ethernet",
        (HardwareBus::Pci, 0x15ad, 0x0405) => "VMware SVGA II",
        _ => "",
    }
}

pub fn print_inventory(
    cap: CapabilityToken,
    verbose: bool,
    requested: Option<&str>,
    mut output: impl FnMut(&str),
) -> bool {
    if let Some(value) = requested {
        let Some(key) = parse_device_id(value) else {
            output("invalid device identifier\n");
            return false;
        };
        let Some(summary) = get(cap, key) else {
            output("device not found\n");
            return false;
        };
        if verbose {
            print_verbose(cap, summary, &mut output);
        } else {
            print_table_header(&mut output);
            print_table_row(summary, &mut output);
        }
        return true;
    }

    if verbose {
        let mut index = 0usize;
        loop {
            let Some(summary) = list(cap, index) else {
                break;
            };
            if index != 0 {
                output("\n");
            }
            print_verbose(cap, summary, &mut output);
            index += 1;
            if index >= summary.total {
                break;
            }
        }
        return true;
    }

    print_table_header(&mut output);
    let mut index = 0usize;
    let mut active = 0usize;
    let mut loaded = 0usize;
    let mut failed = 0usize;
    let mut without_driver = 0usize;
    let mut other = 0usize;
    let mut total = 0usize;
    loop {
        let Some(summary) = list(cap, index) else {
            break;
        };
        print_table_row(summary, &mut output);
        total += 1;
        match summary.state {
            HardwareState::Active => active += 1,
            HardwareState::Loaded => loaded += 1,
            HardwareState::ProbeFailed => failed += 1,
            HardwareState::NoDriver => without_driver += 1,
            HardwareState::Disabled | HardwareState::Unknown => other += 1,
        }
        index += 1;
        if index >= summary.total {
            break;
        }
    }
    let mut line = heapless::String::<160>::new();
    let _ = writeln!(
        line,
        "{} devices: {} active, {} loaded, {} probe failed, {} without driver, {} other",
        total, active, loaded, failed, without_driver, other
    );
    output(&line);
    true
}

fn print_table_header(output: &mut impl FnMut(&str)) {
    output("ID           BUS  CLASS      DEVICE                 DRIVER   STATUS\n");
}

fn print_table_row(summary: InventorySummary, output: &mut impl FnMut(&str)) {
    let mut line = heapless::String::<256>::new();
    let bus = summary.bus();
    let vendor = summary.vendor_id().unwrap_or(0);
    let device = summary.device_id().unwrap_or(0);
    let known_name = device_name(bus, vendor, device, summary.class());
    let _ = write!(
        line,
        "{:<12} {:<4} {:<10} ",
        DeviceId(summary.key),
        bus_label(bus),
        class_label(summary.class(), summary.subclass())
    );
    if known_name.is_empty() {
        if matches!(bus, HardwareBus::Pci) {
            let _ = write!(line, "{:<22}", GenericPciName(summary));
        } else {
            let _ = write!(line, "{:<22}", generic_non_pci_name(summary));
        }
    } else {
        let _ = write!(line, "{:<22}", known_name);
    }
    if summary.driver == 0 {
        let _ = write!(line, " {:<8}", "—");
    } else {
        let _ = write!(line, " {:<8}", ShortName(summary.driver));
    }
    let _ = writeln!(line, " {}", state_label(summary.state));
    output(&line);
}

fn print_verbose(cap: CapabilityToken, summary: InventorySummary, output: &mut impl FnMut(&str)) {
    let drivers = field(cap, summary.key, DevicedMsg::FIELD_DRIVERS).unwrap_or([0; 3]);
    let diagnostic = field(cap, summary.key, DevicedMsg::FIELD_DIAGNOSTIC).unwrap_or([0; 3]);
    let subsystem = field(cap, summary.key, DevicedMsg::FIELD_SUBSYSTEM).unwrap_or([0; 3]);
    let irq = field(cap, summary.key, DevicedMsg::FIELD_IRQ).unwrap_or([u64::MAX; 3]);
    let vendor = summary.vendor_id().unwrap_or(0);
    let device = summary.device_id().unwrap_or(0);
    let known_name = device_name(summary.bus(), vendor, device, summary.class());
    let mut line = heapless::String::<256>::new();

    if known_name.is_empty() {
        let _ = writeln!(line, "Device:           {}", GenericPciName(summary));
    } else {
        let _ = writeln!(line, "Device:           {}", known_name);
    }
    let _ = writeln!(line, "Device ID:        {}", DeviceId(summary.key));
    let _ = writeln!(line, "Bus:              {}", bus_label(summary.bus()));
    if matches!(summary.bus(), HardwareBus::Pci) {
        let _ = writeln!(line, "Vendor/device:    {:04x}:{:04x}", vendor, device);
        if subsystem[0] != 0 && subsystem[0] != 0xffff_ffff {
            let _ = writeln!(
                line,
                "Subsystem:        {:04x}:{:04x}",
                subsystem[0] & 0xffff,
                (subsystem[0] >> 16) & 0xffff
            );
        }
    }
    let _ = writeln!(
        line,
        "Class:            {:02x}:{:02x}:{:02x} {}",
        summary.class(),
        summary.subclass(),
        summary.programming_interface(),
        class_label(summary.class(), summary.subclass())
    );
    let _ = writeln!(line, "Revision:         {:02x}", summary.revision());
    write_optional_name(&mut line, "Matched driver:", drivers[0]);
    write_optional_name(&mut line, "Bound driver:", drivers[1]);
    let _ = writeln!(line, "State:            {}", state_label(summary.state));
    if summary.failure_stage != HardwareFailureStage::None {
        let _ = writeln!(
            line,
            "Failure stage:    {}",
            failure_stage_label(summary.failure_stage)
        );
    }
    if diagnostic[1] != 0 {
        let _ = writeln!(line, "Error code:       {}", diagnostic[1]);
        let _ = writeln!(
            line,
            "Error:            {}",
            diagnostic_message(summary, diagnostic[1])
        );
    }
    if irq[0] != u64::MAX && irq[0] != 0xff {
        let _ = writeln!(line, "IRQ:              {}", irq[0]);
    }
    for bar_index in 0..6u64 {
        if let Some(values) = field(cap, summary.key, DevicedMsg::FIELD_BAR0 + bar_index) {
            if values[0] != 0 {
                let _ = writeln!(
                    line,
                    "BAR{}:             {:#010x}",
                    bar_index, values[0] as u32
                );
            }
        }
    }
    let _ = writeln!(
        line,
        "Discovery source: {}",
        match summary.bus() {
            HardwareBus::Pci => "PCI boot enumeration",
            HardwareBus::Ps2 => "PS/2 controller",
            HardwareBus::Platform => "platform registration",
            HardwareBus::Unknown => "unknown",
        }
    );
    output(&line);
}

fn write_optional_name(line: &mut heapless::String<256>, label: &str, value: u64) {
    if value == 0 {
        let _ = writeln!(line, "{:<18} —", label);
    } else {
        let _ = writeln!(line, "{:<18} {}", label, ShortName(value));
    }
}

pub const fn diagnostic_message(summary: InventorySummary, code: u64) -> &'static str {
    match (summary.vendor_id(), summary.device_id(), code) {
        (Some(0x15ad), Some(0x07b0), 2) => "zero BAR",
        (Some(0x15ad), Some(0x07b0), 14) => "device reset failed",
        (Some(0x15ad), Some(0x07b0), 19) => "device activation failed",
        (_, _, 1) => "device initialization failed",
        (_, _, 21) => "VirtIO network initialization failed",
        _ => "driver reported a hardware initialization error",
    }
}

pub const fn generic_non_pci_name(summary: InventorySummary) -> &'static str {
    match (summary.bus(), summary.class(), summary.subclass()) {
        (HardwareBus::Ps2, 0x09, 0x00) => "PS/2 Keyboard",
        (HardwareBus::Ps2, 0x09, 0x02) => "PS/2 Mouse",
        _ => "Hardware device",
    }
}

struct GenericPciName(InventorySummary);

impl core::fmt::Display for GenericPciName {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match (self.0.vendor_id(), self.0.device_id()) {
            (Some(vendor), Some(device)) => {
                write!(formatter, "PCI device {:04x}:{:04x}", vendor, device)
            }
            _ => formatter.write_str("PCI device"),
        }
    }
}

pub struct DeviceId(pub u64);

impl core::fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match HardwareBus::from_u64(self.0 & 0xff) {
            HardwareBus::Pci => write!(
                formatter,
                "{:04x}:{:02x}:{:02x}.{}",
                (self.0 >> 8) & 0xffff,
                (self.0 >> 24) & 0xff,
                (self.0 >> 32) & 0xff,
                (self.0 >> 40) & 0xff
            ),
            HardwareBus::Ps2 => write!(formatter, "ps2:{}", (self.0 >> 8) & 0xff),
            HardwareBus::Platform => write!(formatter, "platform:{:x}", self.0 >> 8),
            HardwareBus::Unknown => write!(formatter, "unknown:{:x}", self.0 >> 8),
        }
    }
}

pub struct ShortName(pub u64);

impl core::fmt::Display for ShortName {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for index in 0..8 {
            let byte = ((self.0 >> (index * 8)) & 0xff) as u8;
            if byte == 0 {
                break;
            }
            formatter.write_str(core::str::from_utf8(&[byte]).unwrap_or("?"))?;
        }
        Ok(())
    }
}

pub fn parse_device_id(value: &str) -> Option<u64> {
    if let Some(rest) = value.strip_prefix("ps2:") {
        return parse_decimal(rest).map(sunlight_ipc::HardwareInventoryRecord::ps2_key);
    }
    let (domain, rest) = if value.len() >= 5 && value.as_bytes().get(4) == Some(&b':') {
        (parse_hex(&value[..4])? as u16, &value[5..])
    } else {
        (0, value)
    };
    let (bus, rest) = rest.split_once(':')?;
    let (device, function) = rest.split_once('.')?;
    Some(sunlight_ipc::HardwareInventoryRecord::pci_key(
        domain,
        parse_hex(bus)? as u8,
        parse_hex(device)? as u8,
        parse_hex(function)? as u8,
    ))
}

fn parse_hex(value: &str) -> Option<u64> {
    u64::from_str_radix(value, 16).ok()
}

fn parse_decimal(value: &str) -> Option<u8> {
    value.parse::<u8>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_is_truthful() {
        assert_eq!(state_label(HardwareState::NoDriver), "no-driver");
        assert_eq!(state_label(HardwareState::ProbeFailed), "probe-failed");
        assert_eq!(state_label(HardwareState::Active), "active");
        assert_eq!(state_display_label(HardwareState::Active), "Active");
        assert_eq!(
            state_display_label(HardwareState::NoDriver),
            "Without driver"
        );
    }

    #[test]
    fn stable_ids_are_distinct_for_matching_numeric_ids() {
        let first = sunlight_ipc::HardwareInventoryRecord::pci_key(0, 0, 3, 0);
        let second = sunlight_ipc::HardwareInventoryRecord::pci_key(0, 0, 4, 0);
        assert_ne!(first, second);
        assert_eq!(parse_device_id("0000:00:03.0"), Some(first));
    }

    #[test]
    fn stable_id_survives_state_updates() {
        let key = sunlight_ipc::HardwareInventoryRecord::pci_key(0, 0, 3, 0);
        let mut record = sunlight_ipc::HardwareInventoryRecord::empty();
        record.key = key;
        record.state = HardwareState::NoDriver as u64;
        record.state = HardwareState::ProbeFailed as u64;
        record.state = HardwareState::Active as u64;
        assert_eq!(record.key, key);
    }

    #[test]
    fn malformed_summary_is_rejected() {
        assert!(decode_summary(IpcMsg::with_label(DevicedMsg::INVENTORY_REPLY)).is_none());
    }

    #[test]
    fn malformed_and_oversized_requests_are_rejected() {
        assert!(!inventory_request_valid(DevicedMsg::LIST_INVENTORY, 2, 0));
        assert!(!inventory_request_valid(
            DevicedMsg::GET_INVENTORY_FIELD,
            2,
            DevicedMsg::FIELD_BAR5 + 1
        ));
        assert!(inventory_request_valid(
            DevicedMsg::GET_INVENTORY_FIELD,
            2,
            DevicedMsg::FIELD_BAR5
        ));
    }

    #[test]
    fn bus_keys_have_deterministic_order() {
        let pci = sunlight_ipc::HardwareInventoryRecord::pci_key(0, 0, 1, 0);
        let ps2 = sunlight_ipc::HardwareInventoryRecord::ps2_key(0);
        assert!(inventory_order_key(pci) < inventory_order_key(ps2));
    }

    #[test]
    fn unknown_optional_fields_decode_safely() {
        let reply = IpcMsg::with_label(DevicedMsg::INVENTORY_REPLY)
            .word(0, sunlight_ipc::HardwareInventoryRecord::ps2_key(0))
            .word(1, 0)
            .word(2, 0)
            .word(3, HardwareState::Unknown as u64 | (1 << 16));
        let summary = decode_summary(reply).unwrap();
        assert_eq!(summary.vendor_id(), None);
        assert_eq!(summary.device_id(), None);
        assert_eq!(state_label(summary.state), "unknown");
    }
}
