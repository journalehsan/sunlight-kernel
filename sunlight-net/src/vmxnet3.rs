use crate::backend::NetDeviceCounters;
use crate::NetError;
use core::sync::atomic::{fence, Ordering};
use sunlight_ipc::Vmxnet3InitStage;

#[used]
#[no_mangle]
pub static SUNLIGHT_VMXNET3_BUILD_MARKER: [u8; 38] = *b"SUNLIGHT_VMXNET3_BUILD_20260711-AUDIT\0";

#[no_mangle]
pub extern "C" fn sunlight_vmxnet3_probe_marker() {}

#[used]
static _VMXNET3_AUDIT_REF: unsafe extern "C" fn() = sunlight_vmxnet3_probe_marker;

pub const VMXNET3_RING_SIZE: usize = 32;
pub const VMXNET3_RX_BUFFER_SIZE: usize = 2048;
pub const VMXNET3_SHARED_PAGES: usize = 1;
pub const VMXNET3_QUEUE_DESC_PAGES: usize = 1;
pub const VMXNET3_RING_PAGES: usize = 4;

const REG_VRRS: usize = 0x00;
const REG_UVRS: usize = 0x08;
const REG_DSAL: usize = 0x10;
const REG_DSAH: usize = 0x18;
const REG_CMD: usize = 0x20;
const REG_MACL: usize = 0x28;
const REG_MACH: usize = 0x30;
const REG_TXPROD: usize = 0x600;
const REG_RXPROD: usize = 0x800;

const CMD_ACTIVATE_DEV: u32 = 0xCAFE_0000;
const CMD_RESET_DEV: u32 = 0xCAFE_0002;
const CMD_UPDATE_RX_MODE: u32 = 0xCAFE_0003;
const CMD_GET_LINK: u32 = 0xF00D_0002;
const REV1_MAGIC: u32 = 3_133_079_265;
const RX_MODE: u32 = 0x01 | 0x02 | 0x04;
const INIT_GEN: u32 = 1;
pub const DRIVER_REVISION_MASK: u32 = 1;
pub const DRIVER_UPT_MASK: u32 = 1;

const SHARED_SIZE: usize = 800;
const QUEUE_DESC_SIZE: usize = 512;
const TX_QUEUE_CONF: usize = 16;
const RX_QUEUE_CONF: usize = 256 + 16;

const TX_RING_OFFSET: usize = 0;
const TX_COMP_OFFSET: usize = 512;
const RX_RING_OFFSET: usize = 1024;
const RX_COMP_OFFSET: usize = 1536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vmxnet3InitError {
    Reset(u32),
    RevisionUnsupported(u32),
    UptUnsupported(u32),
    InvalidMac(u32, u32),
    MalformedMacRegisters(u32, u32),
    Activate(u32),
    UpdateRxMode(u32),
}

#[derive(Clone, Copy, Debug)]
pub enum Vmxnet3InitEvent {
    Revision {
        device_mask: u32,
        driver_mask: u32,
        selected: u32,
    },
    Upt {
        device_mask: u32,
        driver_mask: u32,
        selected: u32,
    },
    Mac([u8; 6]),
    Dma {
        shared: u64,
        queue_desc: u64,
        rings: u64,
    },
    Rings {
        tx: u64,
        rx: u64,
    },
    Activated,
}

impl Vmxnet3InitEvent {
    pub const fn stage(self) -> Vmxnet3InitStage {
        match self {
            Self::Revision { .. } => Vmxnet3InitStage::RevisionSelected,
            Self::Upt { .. } => Vmxnet3InitStage::UptSelected,
            Self::Mac(_) => Vmxnet3InitStage::MacRead,
            Self::Dma { .. } => Vmxnet3InitStage::DmaAllocated,
            Self::Rings { .. } => Vmxnet3InitStage::RingsInitialized,
            Self::Activated => Vmxnet3InitStage::DeviceActivated,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Vmxnet3PersistentState {
    pub revision: u32,
    pub upt: u32,
    pub shared: u64,
    pub queue_desc: u64,
    pub tx_ring: u64,
    pub rx_ring: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FirstTxDescriptor {
    pub index: u16,
    pub dma_address: u64,
    pub length: u16,
    pub flags: u32,
    pub generation: u8,
    pub producer: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TxDesc {
    addr: u64,
    word2: u32,
    word3: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RxDesc {
    addr: u64,
    word2: u32,
    word3: u32,
}

pub struct Vmxnet3 {
    bar0: u64,
    mac: [u8; 6],
    link_up: bool,
    revision: u32,
    upt: u32,
    shared_phys: u64,
    queue_desc_phys: u64,
    rings_phys: u64,
    tx_ring_virt: u64,
    tx_comp_virt: u64,
    rx_ring_virt: u64,
    rx_comp_virt: u64,
    tx_buf_phys: [u64; VMXNET3_RING_SIZE],
    tx_buf_virt: [u64; VMXNET3_RING_SIZE],
    rx_buf_phys: [u64; VMXNET3_RING_SIZE],
    rx_buf_virt: [u64; VMXNET3_RING_SIZE],
    tx_next: u16,
    tx_gen: u32,
    tx_comp_next: u16,
    tx_comp_gen: u32,
    tx_in_flight: u16,
    rx_comp_next: u16,
    rx_comp_gen: u32,
    rx_fill: u16,
    rx_fill_gen: u32,
    counters: NetDeviceCounters,
    first_tx: Option<FirstTxDescriptor>,
    first_rx_len: u16,
    first_rx_ethertype: u16,
}

unsafe impl Send for Vmxnet3 {}

impl Vmxnet3 {
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn init<F>(
        bar0: u64,
        bar1: u64,
        shared_phys: u64,
        shared_virt: u64,
        queue_desc_phys: u64,
        queue_desc_virt: u64,
        rings_phys: u64,
        rings_virt: u64,
        tx_buf_phys: [u64; VMXNET3_RING_SIZE],
        tx_buf_virt: [u64; VMXNET3_RING_SIZE],
        rx_buf_phys: [u64; VMXNET3_RING_SIZE],
        rx_buf_virt: [u64; VMXNET3_RING_SIZE],
        mut trace: F,
    ) -> Result<Self, Vmxnet3InitError>
    where
        F: FnMut(Vmxnet3InitEvent),
    {
        // RESET_DEV is a set command with no completion value defined by the
        // VMXNET3 ABI (the Linux reference driver also does not read it back).
        write32(bar1, REG_CMD, CMD_RESET_DEV);
        let revisions = read32(bar1, REG_VRRS);
        let revision_common = revisions & DRIVER_REVISION_MASK;
        if revision_common == 0 {
            return Err(Vmxnet3InitError::RevisionUnsupported(revisions));
        }
        let selected_revision = 1 << revision_common.trailing_zeros();
        write32(bar1, REG_VRRS, selected_revision);
        trace(Vmxnet3InitEvent::Revision {
            device_mask: revisions,
            driver_mask: DRIVER_REVISION_MASK,
            selected: selected_revision,
        });
        let upt_versions = read32(bar1, REG_UVRS);
        let upt_common = upt_versions & DRIVER_UPT_MASK;
        if upt_common == 0 {
            return Err(Vmxnet3InitError::UptUnsupported(upt_versions));
        }
        let selected_upt = 1 << upt_common.trailing_zeros();
        write32(bar1, REG_UVRS, selected_upt);
        trace(Vmxnet3InitEvent::Upt {
            device_mask: upt_versions,
            driver_mask: DRIVER_UPT_MASK,
            selected: selected_upt,
        });

        (shared_virt as *mut u8).write_bytes(0, 4096);
        (queue_desc_virt as *mut u8).write_bytes(0, 4096);
        (rings_virt as *mut u8).write_bytes(0, VMXNET3_RING_PAGES * 4096);

        let mac_lo = read32(bar1, REG_MACL);
        let mac_hi = read32(bar1, REG_MACH);
        let mac = [
            mac_lo as u8,
            (mac_lo >> 8) as u8,
            (mac_lo >> 16) as u8,
            (mac_lo >> 24) as u8,
            mac_hi as u8,
            (mac_hi >> 8) as u8,
        ];
        if mac_hi & 0xffff_0000 != 0 {
            return Err(Vmxnet3InitError::MalformedMacRegisters(mac_lo, mac_hi));
        }
        if mac == [0; 6] || mac == [0xff; 6] || mac[0] & 1 != 0 {
            return Err(Vmxnet3InitError::InvalidMac(mac_lo, mac_hi));
        }
        write32(bar1, REG_MACL, mac_lo);
        write32(bar1, REG_MACH, mac_hi & 0xFFFF);
        trace(Vmxnet3InitEvent::Mac(mac));
        trace(Vmxnet3InitEvent::Dma {
            shared: shared_phys,
            queue_desc: queue_desc_phys,
            rings: rings_phys,
        });

        write_u32(shared_virt, 0, REV1_MAGIC);
        write_u32(shared_virt, 4, SHARED_SIZE as u32);
        write_u32(shared_virt, 8, 1);
        write_u32(shared_virt, 12, 2 | (1 << 2));
        write_u32(shared_virt, 16, 1);
        write_u32(shared_virt, 20, 1);
        write_u64(shared_virt, 40, queue_desc_phys);
        write_u32(shared_virt, 52, QUEUE_DESC_SIZE as u32);
        write_u32(shared_virt, 56, 1500);
        write_u16(shared_virt, 60, 1);
        write_u8(shared_virt, 62, 1);
        write_u8(shared_virt, 63, 1);
        // One logical interrupt is described because queue descriptors require
        // an index, but all interrupts are disabled: completion processing is
        // driven by the bounded NetRx poll cadence.
        write_u8(shared_virt, 80, 0);
        write_u8(shared_virt, 81, 1);
        write_u8(shared_virt, 82, 0);
        write_u32(shared_virt, 108, 1);
        // Vmxnet3_DriverShared.devRead.rxFilterConf.rxMode (revision 1).
        write_u32(shared_virt, 120, RX_MODE);

        write_u64(
            queue_desc_virt,
            TX_QUEUE_CONF,
            rings_phys + TX_RING_OFFSET as u64,
        );
        write_u64(
            queue_desc_virt,
            TX_QUEUE_CONF + 16,
            rings_phys + TX_COMP_OFFSET as u64,
        );
        write_u64(queue_desc_virt, TX_QUEUE_CONF + 24, u64::MAX);
        write_u32(
            queue_desc_virt,
            TX_QUEUE_CONF + 40,
            VMXNET3_RING_SIZE as u32,
        );
        write_u32(
            queue_desc_virt,
            TX_QUEUE_CONF + 48,
            VMXNET3_RING_SIZE as u32,
        );

        write_u64(
            queue_desc_virt,
            RX_QUEUE_CONF,
            rings_phys + RX_RING_OFFSET as u64,
        );
        write_u64(queue_desc_virt, RX_QUEUE_CONF + 8, 0);
        write_u64(
            queue_desc_virt,
            RX_QUEUE_CONF + 16,
            rings_phys + RX_COMP_OFFSET as u64,
        );
        write_u64(queue_desc_virt, RX_QUEUE_CONF + 24, u64::MAX);
        write_u32(
            queue_desc_virt,
            RX_QUEUE_CONF + 40,
            VMXNET3_RING_SIZE as u32,
        );
        write_u32(queue_desc_virt, RX_QUEUE_CONF + 44, 0);
        write_u32(
            queue_desc_virt,
            RX_QUEUE_CONF + 48,
            VMXNET3_RING_SIZE as u32,
        );

        let rx_ring = (rings_virt + RX_RING_OFFSET as u64) as *mut RxDesc;
        for index in 0..VMXNET3_RING_SIZE {
            let generation = if index + 1 == VMXNET3_RING_SIZE {
                INIT_GEN ^ 1
            } else {
                INIT_GEN
            };
            rx_ring.add(index).write_volatile(RxDesc {
                addr: rx_buf_phys[index],
                word2: (VMXNET3_RX_BUFFER_SIZE as u32) | (generation << 31),
                word3: 0,
            });
        }
        trace(Vmxnet3InitEvent::Rings {
            tx: rings_phys + TX_RING_OFFSET as u64,
            rx: rings_phys + RX_RING_OFFSET as u64,
        });

        fence(Ordering::SeqCst);
        write32(bar1, REG_DSAL, shared_phys as u32);
        write32(bar1, REG_DSAH, (shared_phys >> 32) as u32);
        let activate = command(bar1, CMD_ACTIVATE_DEV);
        if activate != 0 {
            return Err(Vmxnet3InitError::Activate(activate));
        }
        trace(Vmxnet3InitEvent::Activated);
        let update_rx_mode = command(bar1, CMD_UPDATE_RX_MODE);
        if update_rx_mode != 0 {
            return Err(Vmxnet3InitError::UpdateRxMode(update_rx_mode));
        }
        write32(bar0, REG_RXPROD, (VMXNET3_RING_SIZE - 1) as u32);

        let mut counters = NetDeviceCounters::default();
        counters.device_resets = 1;
        counters.device_activations = 1;
        counters.rx_buffers_posted = (VMXNET3_RING_SIZE - 1) as u64;

        Ok(Self {
            bar0,
            mac,
            link_up: command(bar1, CMD_GET_LINK) & 1 != 0,
            revision: selected_revision,
            upt: selected_upt,
            shared_phys,
            queue_desc_phys,
            rings_phys,
            tx_ring_virt: rings_virt + TX_RING_OFFSET as u64,
            tx_comp_virt: rings_virt + TX_COMP_OFFSET as u64,
            rx_ring_virt: rings_virt + RX_RING_OFFSET as u64,
            rx_comp_virt: rings_virt + RX_COMP_OFFSET as u64,
            tx_buf_phys,
            tx_buf_virt,
            rx_buf_phys,
            rx_buf_virt,
            tx_next: 0,
            tx_gen: INIT_GEN,
            tx_comp_next: 0,
            tx_comp_gen: INIT_GEN,
            tx_in_flight: 0,
            rx_comp_next: 0,
            rx_comp_gen: INIT_GEN,
            rx_fill: (VMXNET3_RING_SIZE - 1) as u16,
            rx_fill_gen: INIT_GEN,
            counters,
            first_tx: None,
            first_rx_len: 0,
            first_rx_ethertype: 0,
        })
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    pub fn link_up(&self) -> bool {
        self.link_up
    }

    pub fn counters(&self) -> NetDeviceCounters {
        self.counters
    }

    pub fn tx_available(&self) -> bool {
        (self.tx_in_flight as usize) < VMXNET3_RING_SIZE - 1
    }

    pub fn rx_available(&self) -> bool {
        self.rx_fill != self.rx_comp_next || self.rx_fill_gen != self.rx_comp_gen
    }

    pub fn persistent_state_valid(&self) -> bool {
        self.mac != [0; 6]
            && self.tx_ring_virt != 0
            && self.tx_comp_virt != 0
            && self.rx_ring_virt != 0
            && self.rx_comp_virt != 0
            && self.tx_buf_phys.iter().all(|address| *address != 0)
            && self.tx_buf_virt.iter().all(|address| *address != 0)
            && self.rx_buf_phys.iter().all(|address| *address != 0)
            && self.rx_buf_virt.iter().all(|address| *address != 0)
            && self.shared_phys != 0
            && self.queue_desc_phys != 0
            && self.rings_phys != 0
    }

    pub fn persistent_state(&self) -> Vmxnet3PersistentState {
        Vmxnet3PersistentState {
            revision: self.revision,
            upt: self.upt,
            shared: self.shared_phys,
            queue_desc: self.queue_desc_phys,
            tx_ring: self.rings_phys + TX_RING_OFFSET as u64,
            rx_ring: self.rings_phys + RX_RING_OFFSET as u64,
        }
    }

    pub fn first_tx_descriptor(&self) -> Option<FirstTxDescriptor> {
        self.first_tx
    }

    pub fn first_rx(&self) -> Option<(u16, u16)> {
        (self.first_rx_len != 0).then_some((self.first_rx_len, self.first_rx_ethertype))
    }

    unsafe fn drain_tx_completions(&mut self) {
        while self.tx_in_flight != 0 {
            let word3 = ((self.tx_comp_virt + self.tx_comp_next as u64 * 16 + 12) as *const u32)
                .read_volatile();
            if word3 >> 31 != self.tx_comp_gen {
                break;
            }
            fence(Ordering::Acquire);
            self.tx_comp_next += 1;
            if self.tx_comp_next as usize == VMXNET3_RING_SIZE {
                self.tx_comp_next = 0;
                self.tx_comp_gen ^= 1;
            }
            self.tx_in_flight -= 1;
            self.counters.tx_completed = self.counters.tx_completed.saturating_add(1);
        }
    }

    unsafe fn replenish_rx(&mut self) {
        let descriptor = (self.rx_ring_virt as *mut RxDesc).add(self.rx_fill as usize);
        descriptor.write_volatile(RxDesc {
            addr: self.rx_buf_phys[self.rx_fill as usize],
            word2: (VMXNET3_RX_BUFFER_SIZE as u32) | (self.rx_fill_gen << 31),
            word3: 0,
        });
        fence(Ordering::Release);
        self.rx_fill += 1;
        if self.rx_fill as usize == VMXNET3_RING_SIZE {
            self.rx_fill = 0;
            self.rx_fill_gen ^= 1;
        }
        write32(self.bar0, REG_RXPROD, self.rx_fill as u32);
        self.counters.rx_buffers_posted = self.counters.rx_buffers_posted.saturating_add(1);
    }

    pub unsafe fn send(&mut self, frame: &[u8]) -> Result<(), NetError> {
        self.counters.tx_requests = self.counters.tx_requests.saturating_add(1);
        self.drain_tx_completions();
        if frame.len() > 1514 {
            self.counters.tx_errors = self.counters.tx_errors.saturating_add(1);
            return Err(NetError::QueueError);
        }
        if self.tx_in_flight as usize >= VMXNET3_RING_SIZE - 1 {
            self.counters.tx_ring_full = self.counters.tx_ring_full.saturating_add(1);
            self.counters.tx_errors = self.counters.tx_errors.saturating_add(1);
            return Err(NetError::QueueError);
        }
        let index = self.tx_next as usize;
        core::ptr::copy_nonoverlapping(
            frame.as_ptr(),
            self.tx_buf_virt[index] as *mut u8,
            frame.len(),
        );

        let descriptor = (self.tx_ring_virt as *mut TxDesc).add(index);
        // Publish GEN last. The device owns a descriptor only after observing
        // the expected generation bit.
        core::ptr::addr_of_mut!((*descriptor).addr).write_volatile(self.tx_buf_phys[index]);
        core::ptr::addr_of_mut!((*descriptor).word3).write_volatile((1 << 12) | (1 << 13));
        core::ptr::addr_of_mut!((*descriptor).word2).write_volatile(frame.len() as u32);
        fence(Ordering::Release);
        core::ptr::addr_of_mut!((*descriptor).word2)
            .write_volatile(frame.len() as u32 | (self.tx_gen << 14));
        fence(Ordering::Release);

        let submitted_index = self.tx_next;
        let submitted_gen = self.tx_gen;
        self.tx_next += 1;
        if self.tx_next as usize == VMXNET3_RING_SIZE {
            self.tx_next = 0;
            self.tx_gen ^= 1;
        }
        write32(self.bar0, REG_TXPROD, self.tx_next as u32);
        self.counters.tx_notifications = self.counters.tx_notifications.saturating_add(1);
        self.tx_in_flight += 1;
        self.counters.tx_submitted = self.counters.tx_submitted.saturating_add(1);
        self.counters.tx_bytes = self.counters.tx_bytes.saturating_add(frame.len() as u64);
        if self.first_tx.is_none() {
            self.first_tx = Some(FirstTxDescriptor {
                index: submitted_index,
                dma_address: self.tx_buf_phys[index],
                length: frame.len() as u16,
                flags: (1 << 12) | (1 << 13),
                generation: submitted_gen as u8,
                producer: self.tx_next,
            });
        }
        Ok(())
    }

    pub unsafe fn recv(&mut self, frame: &mut [u8]) -> usize {
        self.counters.polls = self.counters.polls.saturating_add(1);
        self.drain_tx_completions();
        let completion = self.rx_comp_virt + self.rx_comp_next as u64 * 16;
        let word3 = ((completion + 12) as *const u32).read_volatile();
        if word3 >> 31 != self.rx_comp_gen {
            return 0;
        }
        fence(Ordering::SeqCst);
        let word0 = (completion as *const u32).read_volatile();
        let word2 = ((completion + 8) as *const u32).read_volatile();
        let index = (word0 & 0xFFF) as usize;
        let eop = word0 & (1 << 14) != 0;
        let sop = word0 & (1 << 15) != 0;
        let queue_id = (word0 >> 16) & 0x3ff;
        let len = (word2 & 0x3FFF) as usize;
        let error = word2 & (1 << 14) != 0;

        self.rx_comp_next += 1;
        if self.rx_comp_next as usize == VMXNET3_RING_SIZE {
            self.rx_comp_next = 0;
            self.rx_comp_gen ^= 1;
        }
        self.replenish_rx();
        self.counters.rx_completed = self.counters.rx_completed.saturating_add(1);
        if index >= VMXNET3_RING_SIZE || error || len == 0 || !sop || !eop || queue_id != 0 {
            self.counters.rx_bad_completion = self.counters.rx_bad_completion.saturating_add(1);
            self.counters.rx_dropped = self.counters.rx_dropped.saturating_add(1);
            self.counters.rx_errors = self.counters.rx_errors.saturating_add(1);
            return 0;
        }

        if len > VMXNET3_RX_BUFFER_SIZE || len > frame.len() {
            self.counters.rx_dropped = self.counters.rx_dropped.saturating_add(1);
            self.counters.rx_errors = self.counters.rx_errors.saturating_add(1);
            return 0;
        }
        let copy_len = len;
        core::ptr::copy_nonoverlapping(
            self.rx_buf_virt[index] as *const u8,
            frame.as_mut_ptr(),
            copy_len,
        );
        self.counters.rx_delivered = self.counters.rx_delivered.saturating_add(1);
        self.counters.rx_bytes = self.counters.rx_bytes.saturating_add(copy_len as u64);
        if self.first_rx_len == 0 {
            self.first_rx_len = copy_len as u16;
            self.first_rx_ethertype = if copy_len >= 14 {
                u16::from_be_bytes([frame[12], frame[13]])
            } else {
                0
            };
        }
        copy_len
    }
}

unsafe fn read32(base: u64, offset: usize) -> u32 {
    ((base + offset as u64) as *const u32).read_volatile()
}

unsafe fn write32(base: u64, offset: usize, value: u32) {
    ((base + offset as u64) as *mut u32).write_volatile(value);
}

unsafe fn command(bar1: u64, value: u32) -> u32 {
    write32(bar1, REG_CMD, value);
    read32(bar1, REG_CMD)
}

unsafe fn write_u8(base: u64, offset: usize, value: u8) {
    ((base + offset as u64) as *mut u8).write_volatile(value);
}

unsafe fn write_u16(base: u64, offset: usize, value: u16) {
    ((base + offset as u64) as *mut u16).write_volatile(value);
}

unsafe fn write_u32(base: u64, offset: usize, value: u32) {
    ((base + offset as u64) as *mut u32).write_volatile(value);
}

unsafe fn write_u64(base: u64, offset: usize, value: u64) {
    ((base + offset as u64) as *mut u64).write_volatile(value);
}
