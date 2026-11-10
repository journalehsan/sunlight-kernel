//! Kernel bulk transport adapted from the existing ring-3 xHCI HID core.
//! Exclusive dedicated controller, one USB2 high-speed root device, one TD at a time.
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
use sunlight_block::usb_msc::{Error, Interface, Transport};

pub const DMA_PAGES: usize = 5;
const TRB_COUNT: usize = 16;
const EVENT_TRB_COUNT: usize = 16;

const DCBAA_OFF: usize = 0x0000;
const COMMAND_RING_OFF: usize = 0x0800;
const EVENT_RING_OFF: usize = 0x0900;
const ERST_OFF: usize = 0x0a00;
const DEVICE_CONTEXT_OFF: usize = 0x1000;
const INPUT_CONTEXT_OFF: usize = 0x2000;
const EP0_RING_OFF: usize = 0x3000;
const BULK_IN_RING_OFF: usize = 0x3100;
const BULK_OUT_RING_OFF: usize = 0x3200;
const DATA_OFF: usize = 0x4000;
const DATA_CAPACITY: usize = 512;

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

const TRB_TYPE_TRANSFER_EVENT: u32 = 32;
const TRB_TYPE_COMMAND_COMPLETION: u32 = 33;

const COMPLETION_SUCCESS: u8 = 1;
const COMPLETION_SHORT_PACKET: u8 = 13;

const REQUEST_GET_DESCRIPTOR: u8 = 6;

const REQUEST_SET_CONFIGURATION: u8 = 9;

const DESCRIPTOR_DEVICE: u8 = 1;
const DESCRIPTOR_CONFIGURATION: u8 = 2;

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

pub struct Controller {
    mmio: *mut u8,
    bar_size: usize,
    operational: *mut u8,
    runtime: *mut u8,
    doorbells: *mut u8,
    dma: *mut u8,
    dma_phys: u64,
    context_size: usize,
    port: u8,
    port_speed: u8,
    slot_id: u8,
    interface: Option<Interface>,
    dead: bool,
    command_ring: ProducerRing,
    ep0_ring: ProducerRing,
    bulk_in: ProducerRing,
    bulk_out: ProducerRing,
    event_dequeue: usize,
    event_cycle: bool,
}

// SAFETY: all access after boot is serialized by usb_storage::DISK.
// MMIO belongs exclusively to this controller; DMA frames are never freed.
unsafe impl Send for Controller {}

impl Controller {
    /// Caller supplies exclusive, uncached MMIO and pinned, zeroed contiguous DMA.
    /// Memory is retained for the kernel lifetime even after failure.
    pub unsafe fn initialize(
        mmio: *mut u8,
        bar_size: usize,
        dma: *mut u8,
        dma_phys: u64,
    ) -> Result<Self, Error> {
        let cap_length = unsafe { read_volatile(mmio) as usize };
        let hcsparams1 = unsafe { read_volatile(mmio.add(0x04).cast::<u32>()) };
        let hcsparams2 = unsafe { read_volatile(mmio.add(0x08).cast::<u32>()) };
        let hccparams1 = unsafe { read_volatile(mmio.add(0x10).cast::<u32>()) };
        let dboff = unsafe { read_volatile(mmio.add(0x14).cast::<u32>()) } as usize & !3;
        let rtsoff = unsafe { read_volatile(mmio.add(0x18).cast::<u32>()) } as usize & !0x1f;
        if cap_length < 0x20
            || cap_length & 3 != 0
            || hcsparams1 & 0xff == 0
            || cap_length + PORTSC_BASE + ((hcsparams1 >> 24) as usize) * PORT_STRIDE > bar_size
            || dboff + 8 > bar_size
            || rtsoff + 0x40 > bar_size
        {
            return Err(Error::Unsupported);
        }
        let operational = unsafe { mmio.add(cap_length) };
        let runtime = unsafe { mmio.add(rtsoff) };
        let doorbells = unsafe { mmio.add(dboff) };
        let context_size = if hccparams1 & (1 << 2) != 0 { 64 } else { 32 };

        crate::serial_println!(
            "[USB-MSC] xHCI caps slots={} ports={} context={} scratch={:#x}",
            hcsparams1 & 255,
            hcsparams1 >> 24,
            context_size,
            hcsparams2
        );
        let scratch_lo = (hcsparams2 >> 27) & 0x1f;
        let scratch_hi = (hcsparams2 >> 16) & 0x1f;
        if (scratch_hi << 5) | scratch_lo != 0 {
            return Err(Error::Unsupported);
        }

        let mut driver = Self {
            mmio,
            bar_size,
            operational,
            runtime,
            doorbells,
            dma,
            dma_phys,
            context_size,
            port: 0,
            port_speed: 0,
            slot_id: 0,
            interface: None,
            dead: false,
            command_ring: ProducerRing::new(COMMAND_RING_OFF),
            ep0_ring: ProducerRing::new(EP0_RING_OFF),
            bulk_in: ProducerRing::new(BULK_IN_RING_OFF),
            bulk_out: ProducerRing::new(BULK_OUT_RING_OFF),
            event_dequeue: 0,
            event_cycle: true,
        };

        driver.take_ownership(hccparams1)?;
        driver.reset_controller()?;
        if driver.read_op32(PAGESIZE) & 1 == 0 {
            return Err(Error::Unsupported);
        }
        driver.setup_rings(hcsparams1)?;
        driver.start_controller()?;
        driver.reset_connected_port((hcsparams1 >> 24) as u8)?;
        driver.enumerate_storage()?;
        Ok(driver)
    }

    fn take_ownership(&mut self, hccparams1: u32) -> Result<(), Error> {
        let mut offset = ((hccparams1 >> 16) & 0xffff) as usize * 4;
        let mut hops = 0;
        while offset != 0 && hops < 64 {
            if offset + 8 > self.bar_size {
                return Err(Error::Unsupported);
            }
            let header = unsafe { read_volatile(self.mmio.add(offset).cast::<u32>()) };
            if header & 0xff == 1 {
                let legacy = unsafe { self.mmio.add(offset).cast::<u32>() };
                let mut value = unsafe { read_volatile(legacy) };
                value |= 1 << 24;
                // USBLEGSUP: request ownership for the operating-system driver.
                unsafe { write_volatile(legacy, value) };
                let deadline = Deadline::new();
                while unsafe { read_volatile(legacy) } & (1 << 16) != 0 {
                    if deadline.expired() {
                        return Err(Error::Timeout);
                    }
                    core::hint::spin_loop();
                }
                break;
            }
            let next = ((header >> 8) & 0xff) as usize * 4;
            offset = if next == 0 { 0 } else { offset + next };
            if offset != 0 && offset + 8 > self.bar_size {
                return Err(Error::Unsupported);
            }
            hops += 1;
        }
        if offset != 0 && hops == 64 {
            return Err(Error::Unsupported);
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
        self.initialize_link(BULK_IN_RING_OFF);
        self.initialize_link(BULK_OUT_RING_OFF);

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
        if (1..=max_ports)
            .filter(|port| {
                self.read_op32(PORTSC_BASE + (*port as usize - 1) * PORT_STRIDE) & PORT_CCS != 0
            })
            .count()
            != 1
        {
            return Err(Error::Unsupported);
        }
        for port in 1..=max_ports {
            let offset = PORTSC_BASE + (port as usize - 1) * PORT_STRIDE;
            let status = self.read_op32(offset);
            if status & PORT_CCS == 0 {
                continue;
            }
            // PORTSC.PR: reset the attached device; do not echo RW1C change bits.
            self.write_op32(offset, (status & PORT_WRITE_PRESERVE) | PORT_PR);
            let deadline = Deadline::new();
            loop {
                let current = self.read_op32(offset);
                if current & PORT_PR == 0 && current & PORT_PED != 0 {
                    self.port = port;
                    self.port_speed = ((current >> PORT_SPEED_SHIFT) & 0x0f) as u8;
                    crate::serial_println!(
                        "[USB-MSC] reset port={} speed={}",
                        port,
                        self.port_speed
                    );
                    if self.port_speed != 3 {
                        return Err(Error::Unsupported);
                    }
                    // PORTSC change bits: acknowledge reset/connect changes after sampling.
                    self.write_op32(offset, (current & PORT_WRITE_PRESERVE) | PORT_CHANGE_BITS);
                    return Ok(());
                }
                if deadline.expired() {
                    break;
                }
                core::hint::spin_loop();
            }
        }
        Err(Error::Unsupported)
    }

    fn enumerate_storage(&mut self) -> Result<(), Error> {
        let completion = self.command(Trb {
            parameter: 0,
            status: 0,
            control: TRB_TYPE_ENABLE_SLOT << 10,
        })?;
        self.slot_id = (completion.control >> 24) as u8;
        if self.slot_id == 0 {
            return Err(Error::Transport);
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
        if device_length != 18
            || self.data()[0] != 18
            || self.data()[1] != DESCRIPTOR_DEVICE
            || self.data()[7] != 64
        {
            return Err(Error::Descriptor);
        }

        let header_length = self.control_in(
            0x80,
            REQUEST_GET_DESCRIPTOR,
            (DESCRIPTOR_CONFIGURATION as u16) << 8,
            0,
            9,
        )?;
        if header_length < 9 || self.data()[1] != DESCRIPTOR_CONFIGURATION {
            return Err(Error::Descriptor);
        }
        let total_length = u16::from_le_bytes([self.data()[2], self.data()[3]]) as usize;
        if total_length < 9 || total_length > DATA_CAPACITY {
            return Err(Error::Descriptor);
        }
        let config_length = self.control_in(
            0x80,
            REQUEST_GET_DESCRIPTOR,
            (DESCRIPTOR_CONFIGURATION as u16) << 8,
            0,
            total_length as u16,
        )?;

        let interface = sunlight_block::usb_msc::interface(&self.data()[..config_length])?;
        self.control_no_data(
            0,
            REQUEST_SET_CONFIGURATION,
            interface.configuration as u16,
            0,
        )?;
        self.prepare_bulk_context(interface);
        self.command(Trb {
            parameter: self.phys(INPUT_CONTEXT_OFF),
            status: 0,
            control: (TRB_TYPE_CONFIGURE_ENDPOINT << 10) | ((self.slot_id as u32) << 24),
        })?;
        self.interface = Some(interface);
        crate::serial_println!("[USB-MSC] xHCI port={} slot={} class=08 subclass=06 protocol=50 interface={} bulk-in={:#x} bulk-out={:#x}",
            self.port, self.slot_id, interface.number, interface.input.address, interface.output.address);
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

    fn prepare_bulk_context(&mut self, interface: Interface) {
        self.zero_input_context();
        let input = endpoint_id(interface.input.address);
        let output = endpoint_id(interface.output.address);
        self.input_write(0, 1, 1 | (1 << input) | (1 << output));
        // Copy slot context written by Address Device before changing Context Entries.
        for dword in 0..4 {
            let value = unsafe {
                read_volatile(self.dma.add(DEVICE_CONTEXT_OFF + dword * 4).cast::<u32>())
            };
            self.input_write(1, dword, value);
        }
        self.input_write(
            1,
            0,
            ((self.port_speed as u32) << 20) | ((input.max(output) as u32) << 27),
        );
        for (id, kind, ring) in [(input, 6, BULK_IN_RING_OFF), (output, 2, BULK_OUT_RING_OFF)] {
            let ctx = 1 + id as usize;
            self.input_write(ctx, 1, (3 << 1) | (kind << 3) | (512 << 16));
            self.input_write(ctx, 2, self.phys(ring) as u32 | 1);
            self.input_write(ctx, 3, (self.phys(ring) >> 32) as u32);
            self.input_write(ctx, 4, 512);
        }
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
        if length as usize > DATA_CAPACITY {
            return Err(Error::Descriptor);
        }
        unsafe { core::ptr::write_bytes(self.dma.add(DATA_OFF), 0, DATA_CAPACITY) };
        let data_trb = self.push_ep0(Trb {
            parameter: self.phys(DATA_OFF),
            status: length as u32,
            control: (TRB_TYPE_DATA << 10) | (1 << 16) | (1 << 5) | (1 << 2),
        });
        let status_trb = self.push_ep0(Trb {
            parameter: 0,
            status: 0,
            control: (TRB_TYPE_STATUS << 10) | (1 << 5),
        });
        self.ring_doorbell(self.slot_id, 1);
        let event = self.wait_transfer(1, data_trb)?;
        self.wait_transfer(1, status_trb)?;
        let residual = (event.status & 0x00ff_ffff) as usize;
        if residual > length as usize {
            return Err(Error::Transport);
        }
        Ok(length as usize - residual)
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
        let status_trb = self.push_ep0(Trb {
            parameter: 0,
            status: 0,
            control: (TRB_TYPE_STATUS << 10) | (1 << 16) | (1 << 5),
        });
        self.ring_doorbell(self.slot_id, 1);
        self.wait_transfer(1, status_trb).map(|_| ())
    }

    fn command(&mut self, trb: Trb) -> Result<Trb, Error> {
        let pointer = Self::push_ring_raw(self.dma, self.dma_phys, &mut self.command_ring, trb);
        self.ring_doorbell(0, 0);
        let deadline = Deadline::new();
        while !deadline.expired() {
            if let Some(event) = self.pop_event() {
                if event.trb_type() == TRB_TYPE_COMMAND_COMPLETION && event.parameter == pointer {
                    return if event.completion_code() == COMPLETION_SUCCESS {
                        Ok(event)
                    } else {
                        Err(Error::Transport)
                    };
                }
            }
            core::hint::spin_loop();
        }
        self.dead = true;
        Err(Error::Timeout)
    }

    fn wait_transfer(&mut self, endpoint: u8, pointer: u64) -> Result<Trb, Error> {
        let deadline = Deadline::new();
        while !deadline.expired() {
            if let Some(event) = self.pop_event() {
                if event.trb_type() == TRB_TYPE_TRANSFER_EVENT
                    && ((event.control >> 16) & 0x1f) as u8 == endpoint
                    && (event.control >> 24) as u8 == self.slot_id
                    && event.parameter == pointer
                {
                    return match event.completion_code() {
                        COMPLETION_SUCCESS | COMPLETION_SHORT_PACKET => Ok(event),
                        6 => Err(Error::Stall),
                        _ => Err(Error::Transport),
                    };
                }
            }
            core::hint::spin_loop();
        }
        self.dead = true;
        Err(Error::Timeout)
    }

    fn pop_event(&mut self) -> Option<Trb> {
        let pointer = unsafe {
            self.dma
                .add(EVENT_RING_OFF + self.event_dequeue * core::mem::size_of::<Trb>())
                .cast::<Trb>()
        };
        let control = unsafe { read_volatile(core::ptr::addr_of!((*pointer).control)) };
        if control & 1 != self.event_cycle as u32 {
            return None;
        }
        fence(Ordering::Acquire);
        let event = unsafe { read_volatile(pointer) };
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

    fn push_ep0(&mut self, trb: Trb) -> u64 {
        Self::push_ring_raw(self.dma, self.dma_phys, &mut self.ep0_ring, trb)
    }

    fn push_ring_raw(dma: *mut u8, dma_phys: u64, ring: &mut ProducerRing, mut trb: Trb) -> u64 {
        let physical = dma_phys + (ring.offset + ring.enqueue * 16) as u64;
        if ring.cycle {
            trb.control |= 1;
        } else {
            trb.control &= !1;
        }
        let pointer = unsafe {
            dma.add(ring.offset + ring.enqueue * core::mem::size_of::<Trb>())
                .cast::<Trb>()
        };
        unsafe {
            write_volatile(core::ptr::addr_of_mut!((*pointer).parameter), trb.parameter);
            write_volatile(core::ptr::addr_of_mut!((*pointer).status), trb.status);
            fence(Ordering::Release);
            write_volatile(core::ptr::addr_of_mut!((*pointer).control), trb.control);
        };
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
        physical
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
        fence(Ordering::SeqCst);
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
        let deadline = Deadline::new();
        while !deadline.expired() {
            if (self.read_op32(offset) & mask != 0) == set {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::Timeout)
    }
}

fn endpoint_id(address: u8) -> u8 {
    (address & 15) * 2 + (address >> 7)
}
struct Deadline {
    start: u64,
    budget: u64,
}
impl Deadline {
    fn new() -> Self {
        Self {
            start: unsafe { core::arch::x86_64::_rdtsc() },
            budget: crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(2),
        }
    }
    fn expired(&self) -> bool {
        self.budget == 0
            || unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(self.start) >= self.budget
    }
}
impl Transport for Controller {
    fn bulk(&mut self, input: bool, data: &mut [u8]) -> Result<usize, Error> {
        if self.dead {
            return Err(Error::Offline);
        }
        if data.len() > DATA_CAPACITY || data.is_empty() {
            return Err(Error::Unsupported);
        }
        if self.read_op32(PORTSC_BASE + (self.port as usize - 1) * PORT_STRIDE)
            & (PORT_CCS | PORT_PED)
            != PORT_CCS | PORT_PED
        {
            self.dead = true;
            return Err(Error::Offline);
        }
        unsafe {
            core::ptr::write_bytes(self.dma.add(DATA_OFF), 0, DATA_CAPACITY);
        }
        if !input {
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr(), self.dma.add(DATA_OFF), data.len())
            };
        }
        let interface = self.interface.ok_or(Error::Offline)?;
        let endpoint = if input {
            interface.input
        } else {
            interface.output
        };
        let trb = Trb {
            parameter: self.phys(DATA_OFF),
            status: data.len() as u32,
            control: (TRB_TYPE_NORMAL << 10) | (1 << 5) | (1 << 2),
        };
        let ring = if input {
            &mut self.bulk_in
        } else {
            &mut self.bulk_out
        };
        let pointer = Self::push_ring_raw(self.dma, self.dma_phys, ring, trb);
        let id = endpoint_id(endpoint.address);
        self.ring_doorbell(self.slot_id, id);
        let event = self.wait_transfer(id, pointer)?;
        let residual = (event.status & 0xffffff) as usize;
        if residual > data.len() {
            self.dead = true;
            return Err(Error::Transport);
        }
        let actual = data.len() - residual;
        if input {
            unsafe {
                core::ptr::copy_nonoverlapping(self.dma.add(DATA_OFF), data.as_mut_ptr(), actual)
            };
        }
        Ok(actual)
    }
    fn clear_halt(&mut self, input: bool) -> Result<(), Error> {
        if self.dead {
            return Err(Error::Offline);
        }
        let interface = self.interface.ok_or(Error::Offline)?;
        let address = if input {
            interface.input.address
        } else {
            interface.output.address
        };
        let id = endpoint_id(address);
        self.control_no_data(2, 1, 0, address as u16)?;
        // Reset Endpoint is valid for Halted endpoints only. Running endpoints
        // are stopped before setting their dequeue during class reset recovery.
        let state = unsafe {
            read_volatile(
                self.dma
                    .add(DEVICE_CONTEXT_OFF + id as usize * self.context_size)
                    .cast::<u32>(),
            )
        } & 7;
        if state == 2 {
            self.command(Trb {
                parameter: 0,
                status: 0,
                control: (14 << 10) | ((id as u32) << 16) | ((self.slot_id as u32) << 24),
            })?;
        } else if state == 1 {
            self.command(Trb {
                parameter: 0,
                status: 0,
                control: (15 << 10) | ((id as u32) << 16) | ((self.slot_id as u32) << 24),
            })?;
        }
        let ring = if input {
            &mut self.bulk_in
        } else {
            &mut self.bulk_out
        };
        let dequeue = self.dma_phys + (ring.offset + ring.enqueue * 16) as u64 | ring.cycle as u64;
        self.command(Trb {
            parameter: dequeue,
            status: 0,
            control: (16 << 10) | ((id as u32) << 16) | ((self.slot_id as u32) << 24),
        })?;
        Ok(())
    }
    fn reset(&mut self) -> Result<(), Error> {
        if self.dead {
            return Err(Error::Offline);
        }
        let interface = self.interface.ok_or(Error::Offline)?;
        self.control_no_data(0x21, 0xff, 0, interface.number as u16)?;
        self.clear_halt(true)?;
        self.clear_halt(false)
    }
}
