use spin::Mutex;
use sunlight_ipc::{
    HardwareBus, HardwareFailureStage, HardwareInventoryRecord, HardwareState,
    HARDWARE_INVENTORY_MAX_RECORDS,
};

struct HardwareInventory {
    records: [HardwareInventoryRecord; HARDWARE_INVENTORY_MAX_RECORDS],
    count: usize,
}

impl HardwareInventory {
    const fn new() -> Self {
        Self {
            records: [HardwareInventoryRecord::empty(); HARDWARE_INVENTORY_MAX_RECORDS],
            count: 0,
        }
    }

    fn insert(&mut self, record: HardwareInventoryRecord) {
        if self.count < self.records.len() {
            self.records[self.count] = record;
            self.count += 1;
        }
    }

    fn find_mut(&mut self, key: u64) -> Option<&mut HardwareInventoryRecord> {
        self.records[..self.count]
            .iter_mut()
            .find(|record| record.key == key)
    }
}

static INVENTORY: Mutex<HardwareInventory> = Mutex::new(HardwareInventory::new());

pub unsafe fn enumerate_boot_hardware() {
    use sunlight_virtio::pci::{pci_read32, pci_read8};

    let mut inventory = INVENTORY.lock();
    inventory.count = 0;

    for bus in 0u8..8 {
        for device in 0u8..32 {
            let header = pci_read8(bus, device, 0, 0x0e);
            let function_count = if header & 0x80 != 0 { 8 } else { 1 };
            for function in 0u8..function_count {
                let ids = pci_read32(bus, device, function, 0x00);
                if ids == 0xffff_ffff {
                    continue;
                }
                let class = pci_read32(bus, device, function, 0x08);
                let header_type = pci_read8(bus, device, function, 0x0e) & 0x7f;
                let subsystem = if header_type == 0 {
                    pci_read32(bus, device, function, 0x2c)
                } else {
                    0
                };
                let mut bars = [0u64; 6];
                let bar_count = if header_type == 0 {
                    6
                } else if header_type == 1 {
                    2
                } else {
                    0
                };
                for (index, bar) in bars.iter_mut().take(bar_count).enumerate() {
                    *bar = pci_read32(bus, device, function, 0x10 + index as u8 * 4) as u64;
                }
                inventory.insert(HardwareInventoryRecord {
                    key: HardwareInventoryRecord::pci_key(0, bus, device, function),
                    identity: (ids as u64)
                        | (((class >> 24) as u64) << 32)
                        | ((((class >> 16) & 0xff) as u64) << 40)
                        | ((((class >> 8) & 0xff) as u64) << 48)
                        | (((class & 0xff) as u64) << 56),
                    subsystem: subsystem as u64,
                    matched_driver: 0,
                    bound_driver: 0,
                    state: HardwareState::NoDriver as u64,
                    error_code: 0,
                    irq: pci_read8(bus, device, function, 0x3c) as u64,
                    bars,
                });
            }
        }
    }

    inventory.insert(HardwareInventoryRecord {
        key: HardwareInventoryRecord::ps2_key(0),
        identity: (0x09u64 << 32),
        subsystem: 0,
        matched_driver: pack_short_name("keyboard"),
        bound_driver: 0,
        state: HardwareState::Loaded as u64,
        error_code: 0,
        irq: 1,
        bars: [0; 6],
    });
    inventory.insert(HardwareInventoryRecord {
        key: HardwareInventoryRecord::ps2_key(1),
        identity: (0x09u64 << 32) | (0x02u64 << 40),
        subsystem: 0,
        matched_driver: pack_short_name("mouse"),
        bound_driver: 0,
        state: HardwareState::Loaded as u64,
        error_code: 0,
        irq: 12,
        bars: [0; 6],
    });
}

pub fn update_pci(
    bus: u8,
    device: u8,
    function: u8,
    matched_driver: u64,
    bound_driver: u64,
    state: HardwareState,
    failure_stage: HardwareFailureStage,
    error_code: u64,
) {
    update(
        HardwareInventoryRecord::pci_key(0, bus, device, function),
        matched_driver,
        bound_driver,
        state,
        failure_stage,
        error_code,
    );
}

pub fn update_ps2(
    port: u8,
    matched_driver: u64,
    bound_driver: u64,
    state: HardwareState,
    failure_stage: HardwareFailureStage,
    error_code: u64,
) {
    update(
        HardwareInventoryRecord::ps2_key(port),
        matched_driver,
        bound_driver,
        state,
        failure_stage,
        error_code,
    );
}

fn update(
    key: u64,
    matched_driver: u64,
    bound_driver: u64,
    state: HardwareState,
    failure_stage: HardwareFailureStage,
    error_code: u64,
) {
    if let Some(record) = INVENTORY.lock().find_mut(key) {
        if matched_driver != 0 {
            record.matched_driver = matched_driver;
        }
        record.bound_driver = bound_driver;
        record.state = (state as u64) | ((failure_stage as u64) << 8);
        record.error_code = error_code;
    }
}

pub fn snapshot(index: usize) -> Option<(HardwareInventoryRecord, usize)> {
    let inventory = INVENTORY.lock();
    inventory
        .records
        .get(index)
        .copied()
        .filter(|_| index < inventory.count)
        .map(|record| (record, inventory.count))
}

pub const fn pack_short_name(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut word = 0u64;
    let mut index = 0usize;
    while index < bytes.len() && index < 8 {
        word |= (bytes[index] as u64) << (index * 8);
        index += 1;
    }
    word
}

pub const fn bus_name(bus: HardwareBus) -> &'static str {
    match bus {
        HardwareBus::Pci => "PCI",
        HardwareBus::Ps2 => "PS2",
        HardwareBus::Platform => "Platform",
        HardwareBus::Unknown => "Unknown",
    }
}
