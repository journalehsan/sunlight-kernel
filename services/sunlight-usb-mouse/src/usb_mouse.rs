//! Minimal xHCI USB HID boot-mouse driver.
//!
//! Hardware access is deliberately limited to the three SunlightX syscalls at
//! the bottom of this file. All controller and USB protocol state lives here in
//! ring 3. The implementation owns one slot, endpoint zero, and one interrupt-IN
//! endpoint. It polls the xHCI event ring, matching Linux usbmouse's asynchronous
//! URB/resubmit pattern without requiring a userspace interrupt delivery ABI.

use core::mem::MaybeUninit;
use core::ptr::{read_volatile, write_volatile};

const PAGE_SIZE: usize = 4096;
const DMA_PAGES: usize = 4;
const TRB_COUNT: usize = 16;
const EVENT_TRB_COUNT: usize = 16;

const DCBAA_OFF: usize = 0x0000;
const COMMAND_RING_OFF: usize = 0x0800;
const EVENT_RING_OFF: usize = 0x0900;
const ERST_OFF: usize = 0x0a00;
const DEVICE_CONTEXT_OFF: usize = 0x1000;
const INPUT_CONTEXT_OFF: usize = 0x2000;
const EP0_RING_OFF: usize = 0x3000;
const INTERRUPT_RING_OFF: usize = 0x3100;
const DATA_OFF: usize = 0x3200;
const DATA_CAPACITY: usize = 256;
const HID_REPORT_LENGTH: usize = 4;

const USBCMD: usize = 0x00;
const USBSTS: usize = 0x04;
const PAGESIZE: usize = 0x08;
const CRCR: usize = 0x18;
const DCBAAP: usize = 0x30;
const CONFIG: usize = 0x38;
const PORTSC_BASE: usize = 0x400;
const PORT_STRIDE: usize = 0x10;

const CMD_RUN_STOP: u32 = 1 << 0;
const CMD_HCRST: u32 = 1 << 1;
const STS_HCH: u32 = 1 << 0;
const STS_CNR: u32 = 1 << 11;

const PORT_CCS: u32 = 1 << 0;
const PORT_PED: u32 = 1 << 1;
const PORT_PR: u32 = 1 << 4;
const PORT_POWER: u32 = 1 << 9;
const PORT_SPEED_SHIFT: u32 = 10;
const PORT_WRITE_PRESERVE: u32 = PORT_POWER | (3 << 14) | (7 << 25);
const PORT_CHANGE_BITS: u32 =
    (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20) | (1 << 21) | (1 << 22) | (1 << 23);

const TRB_TYPE_NORMAL: u32 = 1;
const TRB_TYPE_SETUP: u32 = 2;
const TRB_TYPE_DATA: u32 = 3;
const TRB_TYPE_STATUS: u32 = 4;
const TRB_TYPE_LINK: u32 = 6;
const TRB_TYPE_ENABLE_SLOT: u32 = 9;
const TRB_TYPE_ADDRESS_DEVICE: u32 = 11;
const TRB_TYPE_CONFIGURE_ENDPOINT: u32 = 12;
const TRB_TYPE_RESET_ENDPOINT: u32 = 14;
const TRB_TYPE_SET_DEQUEUE: u32 = 16;
const TRB_TYPE_TRANSFER_EVENT: u32 = 32;
const TRB_TYPE_COMMAND_COMPLETION: u32 = 33;

const COMPLETION_SUCCESS: u8 = 1;
const COMPLETION_SHORT_PACKET: u8 = 13;

const REQUEST_GET_DESCRIPTOR: u8 = 6;
const REQUEST_SET_CONFIGURATION: u8 = 9;
const REQUEST_SET_PROTOCOL: u8 = 11;
const DESCRIPTOR_DEVICE: u8 = 1;
const DESCRIPTOR_CONFIGURATION: u8 = 2;

const ENDPOINT_TYPE_INTERRUPT_IN: u32 = 7;

const SPIN_LIMIT: usize = 20_000_000;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Trb {
    parameter: u64,
    status: u32,
    control: u32,
}

impl Trb {
    fn trb_type(self) -> u32 {
        (self.control >> 10) & 0x3f
    }

    fn completion_code(self) -> u8 {
        (self.status >> 24) as u8
    }
}

#[repr(C, align(16))]
struct ErstEntry {
    segment_base: u64,
    segment_size: u32,
    reserved: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NoController,
    MapFailed,
    DmaFailed,
    ControllerTimeout,
    UnsupportedPageSize,
    ScratchpadsRequired,
    NoConnectedPort,
    CommandFailed,
    TransferFailed,
    BadDescriptor,
    NoBootMouse,
}

/// Relative mouse event shared with the PS/2 input path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub dx: i16,
    pub dy: i16,
    pub buttons: u8,
}

#[derive(Clone, Copy)]
struct EndpointInfo {
    address: u8,
    max_packet: u16,
    interval: u8,
}

struct ProducerRing {
    offset: usize,
    enqueue: usize,
    cycle: bool,
}

impl ProducerRing {
    const fn new(offset: usize) -> Self {
        Self {
            offset,
            enqueue: 0,
            cycle: true,
        }
    }
}

struct XhciMouse {
    mmio: *mut u8,
    operational: *mut u8,
    runtime: *mut u8,
    doorbells: *mut u8,
    dma: *mut u8,
    dma_phys: u64,
    context_size: usize,
    port: u8,
    port_speed: u8,
    slot_id: u8,
    endpoint_id: u8,
    command_ring: ProducerRing,
    ep0_ring: ProducerRing,
    interrupt_ring: ProducerRing,
    event_dequeue: usize,
    event_cycle: bool,
    interrupt_armed: bool,
}

unsafe impl Send for XhciMouse {}

static mut DRIVER: MaybeUninit<XhciMouse> = MaybeUninit::uninit();
static mut DRIVER_READY: bool = false;

/// Initialize the controller, enumerate one HID boot mouse, and arm its first
/// 4-byte interrupt transfer.
pub fn init() -> Result<(), Error> {
    let controller = XhciMouse::initialize()?;
    unsafe {
        core::ptr::addr_of_mut!(DRIVER).write(MaybeUninit::new(controller));
        DRIVER_READY = true;
    }
    Ok(())
}

/// Main non-blocking entry point. It consumes at most one completed xHCI event,
/// parses `[buttons, dx, dy, wheel]`, and immediately resubmits the DMA buffer.
pub fn poll() -> Option<MouseEvent> {
    unsafe {
        if !DRIVER_READY {
            return None;
        }
        (&mut *core::ptr::addr_of_mut!(DRIVER))
            .assume_init_mut()
            .poll_once()
    }
}

impl XhciMouse {
    fn initialize() -> Result<Self, Error> {
        let (bar_phys, bar_size) = syscall::xhci_info().ok_or(Error::NoController)?;
        let mmio = syscall::map_mmio(bar_phys, bar_size).ok_or(Error::MapFailed)?;
        let (dma, dma_phys) = syscall::dma_alloc(DMA_PAGES).ok_or(Error::DmaFailed)?;
        unsafe { core::ptr::write_bytes(dma, 0, DMA_PAGES * PAGE_SIZE) };

        let cap_length = unsafe { read_volatile(mmio) as usize };
        let hcsparams1 = unsafe { read_volatile(mmio.add(0x04).cast::<u32>()) };
        let hcsparams2 = unsafe { read_volatile(mmio.add(0x08).cast::<u32>()) };
        let hccparams1 = unsafe { read_volatile(mmio.add(0x10).cast::<u32>()) };
        let dboff = unsafe { read_volatile(mmio.add(0x14).cast::<u32>()) } as usize & !3;
        let rtsoff = unsafe { read_volatile(mmio.add(0x18).cast::<u32>()) } as usize & !0x1f;
        let operational = unsafe { mmio.add(cap_length) };
        let runtime = unsafe { mmio.add(rtsoff) };
        let doorbells = unsafe { mmio.add(dboff) };
        let context_size = if hccparams1 & (1 << 2) != 0 { 64 } else { 32 };

        let scratch_lo = (hcsparams2 >> 27) & 0x1f;
        let scratch_hi = (hcsparams2 >> 16) & 0x1f;
        if (scratch_hi << 5) | scratch_lo != 0 {
            return Err(Error::ScratchpadsRequired);
        }

        let mut driver = Self {
            mmio,
            operational,
            runtime,
            doorbells,
            dma,
            dma_phys,
            context_size,
            port: 0,
            port_speed: 0,
            slot_id: 0,
            endpoint_id: 0,
            command_ring: ProducerRing::new(COMMAND_RING_OFF),
            ep0_ring: ProducerRing::new(EP0_RING_OFF),
            interrupt_ring: ProducerRing::new(INTERRUPT_RING_OFF),
            event_dequeue: 0,
            event_cycle: true,
            interrupt_armed: false,
        };

        driver.take_ownership(hccparams1)?;
        driver.reset_controller()?;
        if driver.read_op32(PAGESIZE) & 1 == 0 {
            return Err(Error::UnsupportedPageSize);
        }
        driver.setup_rings(hcsparams1)?;
        driver.start_controller()?;
        driver.reset_connected_port((hcsparams1 >> 24) as u8)?;
        driver.enumerate_mouse()?;
        Ok(driver)
    }

    fn take_ownership(&mut self, hccparams1: u32) -> Result<(), Error> {
        let mut offset = ((hccparams1 >> 16) & 0xffff) as usize * 4;
        let mut hops = 0;
        while offset != 0 && hops < 64 {
            let header = unsafe { read_volatile(self.mmio.add(offset).cast::<u32>()) };
            if header & 0xff == 1 {
                let legacy = unsafe { self.mmio.add(offset).cast::<u32>() };
                let mut value = unsafe { read_volatile(legacy) };
                value |= 1 << 24;
                // USBLEGSUP: request ownership for the operating-system driver.
                unsafe { write_volatile(legacy, value) };
                let mut spins = SPIN_LIMIT;
                while unsafe { read_volatile(legacy) } & (1 << 16) != 0 {
                    if spins == 0 {
                        return Err(Error::ControllerTimeout);
                    }
                    spins -= 1;
                    syscall::yield_now();
                }
                break;
            }
            offset = ((header >> 8) & 0xff) as usize * 4;
            hops += 1;
        }
        Ok(())
    }

    fn reset_controller(&mut self) -> Result<(), Error> {
        let command = self.read_op32(USBCMD) & !CMD_RUN_STOP;
        // USBCMD.R/S: halt the controller before changing ring registers.
        self.write_op32(USBCMD, command);
        self.wait_op_bits(USBSTS, STS_HCH, true)?;
        // USBCMD.HCRST: restore the host controller to its reset state.
        self.write_op32(USBCMD, command | CMD_HCRST);
        self.wait_op_bits(USBCMD, CMD_HCRST, false)?;
        self.wait_op_bits(USBSTS, STS_CNR, false)
    }

    fn setup_rings(&mut self, hcsparams1: u32) -> Result<(), Error> {
        let max_slots = (hcsparams1 & 0xff).min(1);
        // DCBAA[0] is the scratchpad-array pointer and remains zero because
        // initialization rejected controllers that require scratchpads.
        unsafe { write_volatile(self.dma.add(DCBAA_OFF).cast::<u64>(), 0) };
        self.initialize_link(COMMAND_RING_OFF);
        self.initialize_link(EP0_RING_OFF);
        self.initialize_link(INTERRUPT_RING_OFF);

        let erst = unsafe { &mut *self.dma.add(ERST_OFF).cast::<ErstEntry>() };
        erst.segment_base = self.phys(EVENT_RING_OFF);
        erst.segment_size = EVENT_TRB_COUNT as u32;
        erst.reserved = 0;

        // DCBAAP: publish the device-context base-address array to xHCI.
        self.write_op64(DCBAAP, self.phys(DCBAA_OFF));
        // CRCR: install the command ring and initial producer cycle state.
        self.write_op64(CRCR, self.phys(COMMAND_RING_OFF) | 1);
        // CONFIG.MaxSlotsEn: this minimal driver permits exactly one slot.
        self.write_op32(CONFIG, max_slots);

        let interrupter = unsafe { self.runtime.add(0x20) };
        // ERSTSZ: interrupter 0 uses one event-ring segment.
        unsafe { write_volatile(interrupter.add(0x08).cast::<u32>(), 1) };
        // ERSTBA: point interrupter 0 at the single ERST entry.
        unsafe { write_volatile(interrupter.add(0x10).cast::<u64>(), self.phys(ERST_OFF)) };
        // ERDP: set the initial event dequeue pointer and clear event-handler busy.
        unsafe {
            write_volatile(
                interrupter.add(0x18).cast::<u64>(),
                self.phys(EVENT_RING_OFF) | (1 << 3),
            )
        };
        Ok(())
    }

    fn start_controller(&mut self) -> Result<(), Error> {
        // USBSTS: acknowledge stale write-one-to-clear status before run.
        self.write_op32(USBSTS, 0x171c);
        // USBCMD.R/S: start command, event, and transfer-ring processing.
        self.write_op32(USBCMD, self.read_op32(USBCMD) | CMD_RUN_STOP);
        self.wait_op_bits(USBSTS, STS_HCH, false)
    }

    fn reset_connected_port(&mut self, max_ports: u8) -> Result<(), Error> {
        for port in 1..=max_ports {
            let offset = PORTSC_BASE + (port as usize - 1) * PORT_STRIDE;
            let status = self.read_op32(offset);
            if status & PORT_CCS == 0 {
                continue;
            }
            // PORTSC.PR: reset the attached device; do not echo RW1C change bits.
            self.write_op32(offset, (status & PORT_WRITE_PRESERVE) | PORT_PR);
            let mut spins = SPIN_LIMIT;
            loop {
                let current = self.read_op32(offset);
                if current & PORT_PR == 0 && current & PORT_PED != 0 {
                    self.port = port;
                    self.port_speed = ((current >> PORT_SPEED_SHIFT) & 0x0f) as u8;
                    // PORTSC change bits: acknowledge reset/connect changes after sampling.
                    self.write_op32(offset, (current & PORT_WRITE_PRESERVE) | PORT_CHANGE_BITS);
                    return Ok(());
                }
                if spins == 0 {
                    break;
                }
                spins -= 1;
                syscall::yield_now();
            }
        }
        Err(Error::NoConnectedPort)
    }

    fn enumerate_mouse(&mut self) -> Result<(), Error> {
        let completion = self.command(Trb {
            parameter: 0,
            status: 0,
            control: TRB_TYPE_ENABLE_SLOT << 10,
        })?;
        self.slot_id = (completion.control >> 24) as u8;
        if self.slot_id == 0 {
            return Err(Error::CommandFailed);
        }
        unsafe {
            write_volatile(
                self.dma
                    .add(DCBAA_OFF + self.slot_id as usize * 8)
                    .cast::<u64>(),
                self.phys(DEVICE_CONTEXT_OFF),
            );
        }

        self.prepare_address_context();
        self.command(Trb {
            parameter: self.phys(INPUT_CONTEXT_OFF),
            status: 0,
            control: (TRB_TYPE_ADDRESS_DEVICE << 10) | ((self.slot_id as u32) << 24),
        })?;

        let device_length = self.control_in(
            0x80,
            REQUEST_GET_DESCRIPTOR,
            (DESCRIPTOR_DEVICE as u16) << 8,
            0,
            18,
        )?;
        if device_length < 8 || self.data()[1] != DESCRIPTOR_DEVICE {
            return Err(Error::BadDescriptor);
        }

        let header_length = self.control_in(
            0x80,
            REQUEST_GET_DESCRIPTOR,
            (DESCRIPTOR_CONFIGURATION as u16) << 8,
            0,
            9,
        )?;
        if header_length < 9 || self.data()[1] != DESCRIPTOR_CONFIGURATION {
            return Err(Error::BadDescriptor);
        }
        let total_length = u16::from_le_bytes([self.data()[2], self.data()[3]]) as usize;
        if total_length < 9 || total_length > DATA_CAPACITY {
            return Err(Error::BadDescriptor);
        }
        let config_length = self.control_in(
            0x80,
            REQUEST_GET_DESCRIPTOR,
            (DESCRIPTOR_CONFIGURATION as u16) << 8,
            0,
            total_length as u16,
        )?;
        let (configuration, interface, endpoint) = self
            .find_boot_mouse(config_length)
            .ok_or(Error::NoBootMouse)?;

        self.control_no_data(0x00, REQUEST_SET_CONFIGURATION, configuration as u16, 0)?;
        self.prepare_interrupt_context(endpoint);
        self.command(Trb {
            parameter: self.phys(INPUT_CONTEXT_OFF),
            status: 0,
            control: (TRB_TYPE_CONFIGURE_ENDPOINT << 10) | ((self.slot_id as u32) << 24),
        })?;
        self.control_no_data(0x21, REQUEST_SET_PROTOCOL, 0, interface as u16)?;
        self.submit_interrupt();
        Ok(())
    }

    fn prepare_address_context(&mut self) {
        self.zero_input_context();
        self.input_write(0, 1, (1 << 0) | (1 << 1));
        let route_speed_entries = ((self.port_speed as u32) << 20) | (1 << 27);
        self.input_write(1, 0, route_speed_entries);
        self.input_write(1, 1, (self.port as u32) << 16);

        let max_packet = match self.port_speed {
            4 => 512, // SuperSpeed
            3 => 64,  // high speed
            _ => 8,   // low/full speed default before the first 8-byte descriptor
        };
        self.input_write(2, 1, (3 << 1) | (4 << 3) | ((max_packet as u32) << 16));
        self.input_write(2, 2, self.phys(EP0_RING_OFF) as u32 | 1);
        self.input_write(2, 3, (self.phys(EP0_RING_OFF) >> 32) as u32);
        self.input_write(2, 4, 8);
    }

    fn prepare_interrupt_context(&mut self, endpoint: EndpointInfo) {
        self.zero_input_context();
        self.endpoint_id = ((endpoint.address & 0x0f) * 2) + 1;
        self.input_write(0, 1, (1 << 0) | (1 << self.endpoint_id));
        self.input_write(1, 0, (self.endpoint_id as u32) << 27);

        let interval = self.xhci_interval(endpoint.interval);
        let ctx = 1 + self.endpoint_id as usize;
        self.input_write(ctx, 0, (interval as u32) << 16);
        self.input_write(
            ctx,
            1,
            (3 << 1) | (ENDPOINT_TYPE_INTERRUPT_IN << 3) | ((endpoint.max_packet as u32) << 16),
        );
        self.input_write(ctx, 2, self.phys(INTERRUPT_RING_OFF) as u32 | 1);
        self.input_write(ctx, 3, (self.phys(INTERRUPT_RING_OFF) >> 32) as u32);
        self.input_write(ctx, 4, 4 | ((endpoint.max_packet as u32) << 16));
    }

    fn xhci_interval(&self, usb_interval: u8) -> u8 {
        if self.port_speed >= 3 {
            usb_interval.saturating_sub(1).min(15)
        } else {
            let mut value = usb_interval.max(1) as u16 * 8;
            let mut exponent = 0;
            while value > 1 && exponent < 15 {
                value >>= 1;
                exponent += 1;
            }
            exponent
        }
    }

    fn find_boot_mouse(&self, length: usize) -> Option<(u8, u8, EndpointInfo)> {
        let data = self.data();
        let configuration = data[5];
        let mut current_mouse_interface = None;
        let mut offset = 0;
        while offset + 2 <= length {
            let descriptor_length = data[offset] as usize;
            if descriptor_length < 2 || offset + descriptor_length > length {
                return None;
            }
            match data[offset + 1] {
                4 if descriptor_length >= 9 => {
                    current_mouse_interface = if data[offset + 5] == 0x03
                        && data[offset + 6] == 0x01
                        && data[offset + 7] == 0x02
                    {
                        Some(data[offset + 2])
                    } else {
                        None
                    };
                }
                5 if descriptor_length >= 7 && current_mouse_interface.is_some() => {
                    let address = data[offset + 2];
                    let attributes = data[offset + 3];
                    if address & 0x80 != 0 && attributes & 0x03 == 0x03 {
                        return Some((
                            configuration,
                            current_mouse_interface.unwrap_or(0),
                            EndpointInfo {
                                address,
                                max_packet: u16::from_le_bytes([
                                    data[offset + 4],
                                    data[offset + 5],
                                ]) & 0x07ff,
                                interval: data[offset + 6],
                            },
                        ));
                    }
                }
                _ => {}
            }
            offset += descriptor_length;
        }
        None
    }

    fn control_in(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<usize, Error> {
        let setup = (request_type as u64)
            | ((request as u64) << 8)
            | ((value as u64) << 16)
            | ((index as u64) << 32)
            | ((length as u64) << 48);
        self.push_ep0(Trb {
            parameter: setup,
            status: 8,
            control: (TRB_TYPE_SETUP << 10) | (3 << 16) | (1 << 6),
        });
        self.push_ep0(Trb {
            parameter: self.phys(DATA_OFF),
            status: length as u32,
            control: (TRB_TYPE_DATA << 10) | (1 << 16) | (1 << 5),
        });
        self.push_ep0(Trb {
            parameter: 0,
            status: 0,
            control: (TRB_TYPE_STATUS << 10) | (1 << 5),
        });
        self.ring_doorbell(self.slot_id, 1);
        let event = self.wait_transfer(1)?;
        let residual = (event.status & 0x00ff_ffff) as usize;
        Ok((length as usize).saturating_sub(residual))
    }

    fn control_no_data(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
    ) -> Result<(), Error> {
        let setup = (request_type as u64)
            | ((request as u64) << 8)
            | ((value as u64) << 16)
            | ((index as u64) << 32);
        self.push_ep0(Trb {
            parameter: setup,
            status: 8,
            control: (TRB_TYPE_SETUP << 10) | (0 << 16) | (1 << 6),
        });
        self.push_ep0(Trb {
            parameter: 0,
            status: 0,
            control: (TRB_TYPE_STATUS << 10) | (1 << 16) | (1 << 5),
        });
        self.ring_doorbell(self.slot_id, 1);
        self.wait_transfer(1).map(|_| ())
    }

    fn poll_once(&mut self) -> Option<MouseEvent> {
        let event = self.pop_event()?;
        if event.trb_type() != TRB_TYPE_TRANSFER_EVENT {
            return None;
        }
        let endpoint = ((event.control >> 16) & 0x1f) as u8;
        if endpoint != self.endpoint_id {
            return None;
        }
        self.interrupt_armed = false;
        match event.completion_code() {
            COMPLETION_SUCCESS | COMPLETION_SHORT_PACKET => {
                let residual = (event.status & 0x00ff_ffff) as usize;
                let actual_length = HID_REPORT_LENGTH.saturating_sub(residual);
                if actual_length < 3 {
                    self.submit_interrupt();
                    return None;
                }
                let report = self.data();
                // Wheel is decoded for protocol completeness. MouseEvent is
                // intentionally ABI-compatible with the existing PS/2 event,
                // which has no wheel member yet.
                let _wheel = if actual_length >= 4 {
                    report[3] as i8
                } else {
                    0
                };
                let result = MouseEvent {
                    buttons: report[0] & 0x07,
                    dx: report[1] as i8 as i16,
                    // HID boot reports positive Y down, which already matches
                    // display_server's top-down coordinate system.
                    dy: report[2] as i8 as i16,
                };
                self.submit_interrupt();
                Some(result)
            }
            _ => {
                let _ = self.recover_interrupt_endpoint();
                None
            }
        }
    }

    fn submit_interrupt(&mut self) {
        if self.interrupt_armed {
            return;
        }
        unsafe { core::ptr::write_bytes(self.dma.add(DATA_OFF), 0, HID_REPORT_LENGTH) };
        let trb = Trb {
            parameter: self.phys(DATA_OFF),
            status: HID_REPORT_LENGTH as u32,
            // ISP requests a Transfer Event when a physical boot mouse returns
            // its common 3-byte short report; IOC covers full 4-byte reports.
            control: (TRB_TYPE_NORMAL << 10) | (1 << 2) | (1 << 5),
        };
        Self::push_ring_raw(self.dma, self.dma_phys, &mut self.interrupt_ring, trb);
        self.interrupt_armed = true;
        self.ring_doorbell(self.slot_id, self.endpoint_id);
    }

    fn recover_interrupt_endpoint(&mut self) -> Result<(), Error> {
        self.command(Trb {
            parameter: 0,
            status: 0,
            control: (TRB_TYPE_RESET_ENDPOINT << 10)
                | ((self.endpoint_id as u32) << 16)
                | ((self.slot_id as u32) << 24),
        })?;
        self.interrupt_ring.enqueue = 0;
        self.interrupt_ring.cycle = true;
        unsafe {
            core::ptr::write_bytes(
                self.dma.add(INTERRUPT_RING_OFF),
                0,
                TRB_COUNT * core::mem::size_of::<Trb>(),
            )
        };
        self.initialize_link(INTERRUPT_RING_OFF);
        self.command(Trb {
            parameter: self.phys(INTERRUPT_RING_OFF) | 1,
            status: 0,
            control: (TRB_TYPE_SET_DEQUEUE << 10)
                | ((self.endpoint_id as u32) << 16)
                | ((self.slot_id as u32) << 24),
        })?;
        self.submit_interrupt();
        Ok(())
    }

    fn command(&mut self, trb: Trb) -> Result<Trb, Error> {
        Self::push_ring_raw(self.dma, self.dma_phys, &mut self.command_ring, trb);
        self.ring_doorbell(0, 0);
        let mut spins = SPIN_LIMIT;
        while spins != 0 {
            if let Some(event) = self.pop_event() {
                if event.trb_type() == TRB_TYPE_COMMAND_COMPLETION {
                    return if event.completion_code() == COMPLETION_SUCCESS {
                        Ok(event)
                    } else {
                        Err(Error::CommandFailed)
                    };
                }
            }
            spins -= 1;
            syscall::yield_now();
        }
        Err(Error::ControllerTimeout)
    }

    fn wait_transfer(&mut self, endpoint: u8) -> Result<Trb, Error> {
        let mut spins = SPIN_LIMIT;
        while spins != 0 {
            if let Some(event) = self.pop_event() {
                if event.trb_type() == TRB_TYPE_TRANSFER_EVENT
                    && ((event.control >> 16) & 0x1f) as u8 == endpoint
                {
                    return match event.completion_code() {
                        COMPLETION_SUCCESS | COMPLETION_SHORT_PACKET => Ok(event),
                        _ => Err(Error::TransferFailed),
                    };
                }
            }
            spins -= 1;
            syscall::yield_now();
        }
        Err(Error::ControllerTimeout)
    }

    fn pop_event(&mut self) -> Option<Trb> {
        let pointer = unsafe {
            self.dma
                .add(EVENT_RING_OFF + self.event_dequeue * core::mem::size_of::<Trb>())
                .cast::<Trb>()
        };
        let event = unsafe { read_volatile(pointer) };
        if event.control & 1 != self.event_cycle as u32 {
            return None;
        }
        self.event_dequeue += 1;
        if self.event_dequeue == EVENT_TRB_COUNT {
            self.event_dequeue = 0;
            self.event_cycle = !self.event_cycle;
        }
        let interrupter = unsafe { self.runtime.add(0x20) };
        // ERDP: return the consumed event to the controller and clear EHB.
        unsafe {
            write_volatile(
                interrupter.add(0x18).cast::<u64>(),
                self.phys(EVENT_RING_OFF + self.event_dequeue * core::mem::size_of::<Trb>())
                    | (1 << 3),
            )
        };
        Some(event)
    }

    fn push_ep0(&mut self, trb: Trb) {
        Self::push_ring_raw(self.dma, self.dma_phys, &mut self.ep0_ring, trb);
    }

    fn push_ring_raw(dma: *mut u8, dma_phys: u64, ring: &mut ProducerRing, mut trb: Trb) {
        if ring.cycle {
            trb.control |= 1;
        } else {
            trb.control &= !1;
        }
        let pointer = unsafe {
            dma.add(ring.offset + ring.enqueue * core::mem::size_of::<Trb>())
                .cast::<Trb>()
        };
        unsafe { write_volatile(pointer, trb) };
        ring.enqueue += 1;
        if ring.enqueue == TRB_COUNT - 1 {
            let link_pointer = unsafe {
                dma.add(ring.offset + (TRB_COUNT - 1) * core::mem::size_of::<Trb>())
                    .cast::<Trb>()
            };
            let link = Trb {
                parameter: dma_phys + ring.offset as u64,
                status: 0,
                control: (TRB_TYPE_LINK << 10) | (1 << 1) | ring.cycle as u32,
            };
            unsafe { write_volatile(link_pointer, link) };
            ring.enqueue = 0;
            ring.cycle = !ring.cycle;
        }
    }

    fn initialize_link(&self, offset: usize) {
        let link = Trb {
            parameter: self.phys(offset),
            status: 0,
            control: (TRB_TYPE_LINK << 10) | (1 << 1) | 1,
        };
        unsafe {
            write_volatile(
                self.dma
                    .add(offset + (TRB_COUNT - 1) * core::mem::size_of::<Trb>())
                    .cast::<Trb>(),
                link,
            )
        };
    }

    fn ring_doorbell(&self, slot: u8, target: u8) {
        // DB[slot]: notify xHCI that the selected command/endpoint ring advanced.
        unsafe {
            write_volatile(
                self.doorbells.add(slot as usize * 4).cast::<u32>(),
                target as u32,
            )
        };
    }

    fn zero_input_context(&self) {
        unsafe {
            core::ptr::write_bytes(self.dma.add(INPUT_CONTEXT_OFF), 0, self.context_size * 34)
        };
    }

    fn input_write(&self, context: usize, dword: usize, value: u32) {
        let offset = INPUT_CONTEXT_OFF + context * self.context_size + dword * 4;
        unsafe { write_volatile(self.dma.add(offset).cast::<u32>(), value) };
    }

    fn data(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.dma.add(DATA_OFF), DATA_CAPACITY) }
    }

    fn phys(&self, offset: usize) -> u64 {
        self.dma_phys + offset as u64
    }

    fn read_op32(&self, offset: usize) -> u32 {
        unsafe { read_volatile(self.operational.add(offset).cast::<u32>()) }
    }

    fn write_op32(&self, offset: usize, value: u32) {
        unsafe { write_volatile(self.operational.add(offset).cast::<u32>(), value) }
    }

    fn write_op64(&self, offset: usize, value: u64) {
        unsafe { write_volatile(self.operational.add(offset).cast::<u64>(), value) }
    }

    fn wait_op_bits(&self, offset: usize, mask: u32, set: bool) -> Result<(), Error> {
        let mut spins = SPIN_LIMIT;
        while spins != 0 {
            if (self.read_op32(offset) & mask != 0) == set {
                return Ok(());
            }
            spins -= 1;
            syscall::yield_now();
        }
        Err(Error::ControllerTimeout)
    }
}

mod syscall {
    const SYS_PROCESS_YIELD: u64 = 21;
    const SYS_XHCI_INFO: u64 = 130;
    const SYS_MAP_MMIO: u64 = 131;
    const SYS_DMA_ALLOC: u64 = 132;

    pub fn yield_now() {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_PROCESS_YIELD => _,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
    }

    pub fn xhci_info() -> Option<(u64, usize)> {
        let address: u64;
        let size: u64;
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_XHCI_INFO => address,
                lateout("rdx") size,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
        (address != 0 && address != u64::MAX).then_some((address, size as usize))
    }

    pub fn map_mmio(physical: u64, size: usize) -> Option<*mut u8> {
        let address: u64;
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_MAP_MMIO => address,
                in("rdi") physical,
                in("rsi") size as u64,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
        (address != 0 && address != u64::MAX).then_some(address as *mut u8)
    }

    pub fn dma_alloc(pages: usize) -> Option<(*mut u8, u64)> {
        let address: u64;
        let physical: u64;
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_DMA_ALLOC => address,
                in("rdi") pages as u64,
                lateout("rdx") physical,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
        (address != 0 && address != u64::MAX).then_some((address as *mut u8, physical))
    }
}
