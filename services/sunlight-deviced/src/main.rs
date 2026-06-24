//! deviced — userspace device/driver registry for SunlightOS.
//!
//! v0 is intentionally small: it tracks ring-3 driver registrations in fixed
//! arrays and exposes one-record-at-a-time IPC queries. Hotplug, dependency
//! graphs, restart policy, and richer metadata belong in later shm-backed
//! protocol revisions.

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, monotonic_millis,
    nameserver_register, DeviceId, DeviceKind, DevicedMsg, DriverId, DriverKind, DriverState,
    IpcMsg,
};

const MAX_DRIVERS: usize = 32;
const MAX_DEVICES: usize = 32;

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

#[derive(Clone, Copy)]
struct DriverRecord {
    id: DriverId,
    name: u64,
    pid: u64,
    kind: DriverKind,
    state: DriverState,
    capabilities: u64,
    _started_at_ms: u64,
    last_seen_ms: u64,
    restart_count: u32,
    occupied: bool,
}

impl DriverRecord {
    const fn empty() -> Self {
        Self {
            id: 0,
            name: 0,
            pid: 0,
            kind: DriverKind::Unknown,
            state: DriverState::Stopped,
            capabilities: 0,
            _started_at_ms: 0,
            last_seen_ms: 0,
            restart_count: 0,
            occupied: false,
        }
    }
}

#[derive(Clone, Copy)]
struct DeviceRecord {
    id: DeviceId,
    name: u64,
    kind: DeviceKind,
    driver_id: DriverId,
    state: DriverState,
    occupied: bool,
}

impl DeviceRecord {
    const fn empty() -> Self {
        Self {
            id: 0,
            name: 0,
            kind: DeviceKind::Unknown,
            driver_id: 0,
            state: DriverState::Stopped,
            occupied: false,
        }
    }
}

struct Registry {
    drivers: [DriverRecord; MAX_DRIVERS],
    devices: [DeviceRecord; MAX_DEVICES],
    next_driver_id: DriverId,
    next_device_id: DeviceId,
}

impl Registry {
    const fn new() -> Self {
        Self {
            drivers: [DriverRecord::empty(); MAX_DRIVERS],
            devices: [DeviceRecord::empty(); MAX_DEVICES],
            next_driver_id: 1,
            next_device_id: 1,
        }
    }

    fn register_driver(&mut self, msg: &IpcMsg) -> IpcMsg {
        let name = msg.words[0];
        let pid = msg.words[1];
        let kind = DriverKind::from_u64(msg.words[2] & 0xffff);
        let state = DriverState::from_u64((msg.words[2] >> 16) & 0xffff);
        let capabilities = msg.words[3];

        if name == 0 {
            return reply_err(DevicedMsg::ERR_BAD_REQUEST);
        }

        let now = monotonic_millis();
        if let Some(idx) = self.find_driver_by_name(name) {
            self.drivers[idx].pid = pid;
            self.drivers[idx].kind = kind;
            self.drivers[idx].state = state;
            self.drivers[idx].capabilities = capabilities;
            self.drivers[idx].last_seen_ms = now;
            self.upsert_logical_device(idx);
            return reply_driver_summary(&self.drivers[idx], self.driver_count());
        }

        if let Some(idx) = self.free_driver_slot() {
            let id = self.next_driver_id;
            self.next_driver_id = self.next_driver_id.wrapping_add(1).max(1);
            self.drivers[idx] = DriverRecord {
                id,
                name,
                pid,
                kind,
                state,
                capabilities,
                _started_at_ms: now,
                last_seen_ms: now,
                restart_count: 0,
                occupied: true,
            };
            self.upsert_logical_device(idx);
            return reply_driver_summary(&self.drivers[idx], self.driver_count());
        }

        reply_err(DevicedMsg::ERR_FULL)
    }

    fn update_state(&mut self, msg: &IpcMsg) -> IpcMsg {
        let id = msg.words[0];
        let state = DriverState::from_u64(msg.words[1]);
        match self.find_driver_by_id(id) {
            Some(idx) => {
                let now = monotonic_millis();
                let id = self.drivers[idx].id;
                self.drivers[idx].state = state;
                self.drivers[idx].last_seen_ms = now;
                self.sync_device_state(id, state);
                reply_driver_summary(&self.drivers[idx], self.driver_count())
            }
            None => reply_err(DevicedMsg::ERR_NOT_FOUND),
        }
    }

    fn touch_driver(&mut self, msg: &IpcMsg) -> IpcMsg {
        let id = msg.words[0];
        match self.find_driver_by_id(id) {
            Some(idx) => {
                self.drivers[idx].last_seen_ms = monotonic_millis();
                reply_driver_summary(&self.drivers[idx], self.driver_count())
            }
            None => reply_err(DevicedMsg::ERR_NOT_FOUND),
        }
    }

    fn list_driver(&self, msg: &IpcMsg) -> IpcMsg {
        let requested = msg.words[0] as usize;
        let mut seen = 0usize;
        for driver in self.drivers.iter() {
            if driver.occupied {
                if seen == requested {
                    return reply_driver_summary(driver, self.driver_count());
                }
                seen += 1;
            }
        }
        reply_err(DevicedMsg::ERR_NOT_FOUND).word(1, self.driver_count() as u64)
    }

    fn get_driver(&self, msg: &IpcMsg) -> IpcMsg {
        let key = msg.words[0];
        let idx = if key < 256 {
            self.find_driver_by_id(key)
        } else {
            self.find_driver_by_name(key)
        };
        match idx {
            Some(i) => reply_driver_summary(&self.drivers[i], self.driver_count()),
            None => reply_err(DevicedMsg::ERR_NOT_FOUND),
        }
    }

    fn list_device(&self, msg: &IpcMsg) -> IpcMsg {
        let requested = msg.words[0] as usize;
        let mut seen = 0usize;
        for device in self.devices.iter() {
            if device.occupied {
                if seen == requested {
                    return reply_device_summary(device, self.device_count());
                }
                seen += 1;
            }
        }
        reply_err(DevicedMsg::ERR_NOT_FOUND).word(1, self.device_count() as u64)
    }

    fn get_device(&self, msg: &IpcMsg) -> IpcMsg {
        let key = msg.words[0];
        for device in self.devices.iter() {
            if device.occupied && (device.id == key || device.name == key) {
                return reply_device_summary(device, self.device_count());
            }
        }
        reply_err(DevicedMsg::ERR_NOT_FOUND)
    }

    fn mark_failed(&mut self, msg: &IpcMsg) -> IpcMsg {
        let id = msg.words[0];
        match self.find_driver_by_id(id) {
            Some(idx) => {
                self.drivers[idx].state = DriverState::Failed;
                self.drivers[idx].last_seen_ms = monotonic_millis();
                self.sync_device_state(id, DriverState::Failed);
                reply_driver_summary(&self.drivers[idx], self.driver_count())
            }
            None => reply_err(DevicedMsg::ERR_NOT_FOUND),
        }
    }

    fn unregister(&mut self, msg: &IpcMsg) -> IpcMsg {
        let id = msg.words[0];
        match self.find_driver_by_id(id) {
            Some(idx) => {
                let summary = self.drivers[idx];
                self.drivers[idx] = DriverRecord::empty();
                self.remove_devices_for_driver(id);
                reply_driver_summary(&summary, self.driver_count())
            }
            None => reply_err(DevicedMsg::ERR_NOT_FOUND),
        }
    }

    fn free_driver_slot(&self) -> Option<usize> {
        self.drivers.iter().position(|driver| !driver.occupied)
    }

    fn find_driver_by_id(&self, id: DriverId) -> Option<usize> {
        self.drivers
            .iter()
            .position(|driver| driver.occupied && driver.id == id)
    }

    fn find_driver_by_name(&self, name: u64) -> Option<usize> {
        self.drivers
            .iter()
            .position(|driver| driver.occupied && driver.name == name)
    }

    fn driver_count(&self) -> usize {
        self.drivers.iter().filter(|driver| driver.occupied).count()
    }

    fn device_count(&self) -> usize {
        self.devices.iter().filter(|device| device.occupied).count()
    }

    fn upsert_logical_device(&mut self, driver_idx: usize) {
        let driver = self.drivers[driver_idx];
        let kind = logical_device_kind(driver.kind, driver.capabilities);
        if let Some(idx) = self.devices.iter().position(|device| {
            device.occupied && device.driver_id == driver.id && device.name == driver.name
        }) {
            self.devices[idx].kind = kind;
            self.devices[idx].state = driver.state;
            return;
        }

        if let Some(idx) = self.devices.iter().position(|device| !device.occupied) {
            let id = self.next_device_id;
            self.next_device_id = self.next_device_id.wrapping_add(1).max(1);
            self.devices[idx] = DeviceRecord {
                id,
                name: driver.name,
                kind,
                driver_id: driver.id,
                state: driver.state,
                occupied: true,
            };
        }
    }

    fn sync_device_state(&mut self, driver_id: DriverId, state: DriverState) {
        for device in self.devices.iter_mut() {
            if device.occupied && device.driver_id == driver_id {
                device.state = state;
            }
        }
    }

    fn remove_devices_for_driver(&mut self, driver_id: DriverId) {
        for device in self.devices.iter_mut() {
            if device.occupied && device.driver_id == driver_id {
                *device = DeviceRecord::empty();
            }
        }
    }
}

fn logical_device_kind(kind: DriverKind, caps: u64) -> DeviceKind {
    if matches!(kind, DriverKind::Keyboard | DriverKind::Mouse) {
        DeviceKind::Input
    } else if caps & sunlight_ipc::DriverCaps::NETWORK != 0 || kind == DriverKind::Network {
        DeviceKind::Network
    } else if caps & sunlight_ipc::DriverCaps::BLOCK != 0 || kind == DriverKind::Block {
        DeviceKind::Block
    } else if caps & sunlight_ipc::DriverCaps::DISPLAY != 0 || kind == DriverKind::Display {
        DeviceKind::Display
    } else if caps & sunlight_ipc::DriverCaps::BUS != 0 || kind == DriverKind::Virtio {
        DeviceKind::Bus
    } else {
        DeviceKind::Unknown
    }
}

fn reply_err(code: u64) -> IpcMsg {
    IpcMsg::with_label(DevicedMsg::ERROR).word(0, code)
}

fn reply_driver_summary(driver: &DriverRecord, total: usize) -> IpcMsg {
    let packed = (driver.kind as u64)
        | ((driver.state as u64) << 8)
        | ((driver.restart_count as u64) << 16)
        | (((total as u64) & 0xff) << 32)
        | ((driver.capabilities & 0x00ff_ffff) << 40);
    IpcMsg::with_label(DevicedMsg::REPLY)
        .word(0, driver.id)
        .word(1, driver.name)
        .word(2, driver.pid)
        .word(3, packed)
}

fn reply_device_summary(device: &DeviceRecord, total: usize) -> IpcMsg {
    let packed =
        (device.kind as u64) | ((device.state as u64) << 8) | (((total as u64) & 0xff) << 32);
    IpcMsg::with_label(DevicedMsg::REPLY)
        .word(0, device.id)
        .word(1, device.name)
        .word(2, device.driver_id)
        .word(3, packed)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[DEVICED] PANIC: {}", info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[DEVICED] starting");
    let ep = endpoint_create();
    nameserver_register("deviced", ep);
    debug_log("[DEVICED] registered as 'deviced'");

    let mut registry = Registry::new();
    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            DevicedMsg::REGISTER_DRIVER => registry.register_driver(&msg),
            DevicedMsg::UPDATE_DRIVER_STATE => registry.update_state(&msg),
            DevicedMsg::TOUCH_DRIVER => registry.touch_driver(&msg),
            DevicedMsg::LIST_DRIVERS => registry.list_driver(&msg),
            DevicedMsg::GET_DRIVER => registry.get_driver(&msg),
            DevicedMsg::LIST_DEVICES => registry.list_device(&msg),
            DevicedMsg::GET_DEVICE => registry.get_device(&msg),
            DevicedMsg::MARK_DRIVER_FAILED => registry.mark_failed(&msg),
            DevicedMsg::UNREGISTER_DRIVER => registry.unregister(&msg),
            _ => reply_err(DevicedMsg::ERR_BAD_REQUEST),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
