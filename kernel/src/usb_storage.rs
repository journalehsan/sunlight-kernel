//! Boot discovery for a dedicated xHCI storage controller. No auto-mount or trust.
use crate::{memory::vmm::VirtualMemoryManager, usb_xhci::Controller};
use core::sync::atomic::{AtomicU64, Ordering};
use sunlight_block::{usb_msc::Disk, BlockDevice};
use sunlight_virtio::pci::{pci_read32, pci_write32};

pub static DISK: spin::Mutex<Option<Disk<Controller>>> = spin::Mutex::new(None);
pub static DEVICE_ID: AtomicU64 = AtomicU64::new(0);
static RESERVED: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

pub fn reserved(bar: u64) -> bool {
    RESERVED.iter().any(|p| p.load(Ordering::Acquire) == bar)
}

pub fn init(vmm: &mut VirtualMemoryManager, hhdm: u64) {
    let mut count = 0;
    for bus in 0..8 {
        for slot in 0..32 {
            for function in 0..8 {
                if count == RESERVED.len() {
                    return;
                }
                let class = unsafe { pci_read32(bus, slot, function, 8) };
                if (class >> 8) & 0xffffff != 0x0c0330 {
                    continue;
                }
                let low = unsafe { pci_read32(bus, slot, function, 0x10) };
                if low & 1 != 0 || low & !15 == 0 {
                    continue;
                }
                let wide = (low >> 1) & 3 == 2;
                let high = if wide {
                    unsafe { pci_read32(bus, slot, function, 0x14) }
                } else {
                    0
                };
                let bar = (low & !15) as u64 | ((high as u64) << 32);
                let command = unsafe { pci_read32(bus, slot, function, 4) } as u16;
                // BAR sizing before controller ownership, with decoding and DMA disabled.
                unsafe {
                    pci_write32(bus, slot, function, 4, (command & !6) as u32);
                    pci_write32(bus, slot, function, 0x10, u32::MAX);
                    if wide {
                        pci_write32(bus, slot, function, 0x14, u32::MAX);
                    }
                }
                let mask_low = unsafe { pci_read32(bus, slot, function, 0x10) };
                let mask_high = if wide {
                    unsafe { pci_read32(bus, slot, function, 0x14) }
                } else {
                    0
                };
                unsafe {
                    pci_write32(bus, slot, function, 0x10, low);
                    if wide {
                        pci_write32(bus, slot, function, 0x14, high);
                    }
                    pci_write32(bus, slot, function, 4, (command | 6) as u32);
                }
                let mask = (mask_low & !15) as u64 | ((mask_high as u64) << 32);
                let size = (!mask).wrapping_add(1) & if wide { u64::MAX } else { u32::MAX as u64 };
                if !(4096..=1024 * 1024).contains(&size) || !size.is_power_of_two() {
                    continue;
                }
                // Distinct uncached mapping, never use a possibly write-back huge HHDM MMIO leaf.
                let virtual_bar = 0xffff_c100_0000_0000 + count as u64 * 0x20_0000;
                if crate::try_map_kernel_mmio_range(
                    vmm,
                    &mut crate::PMM.lock(),
                    virtual_bar,
                    bar,
                    size,
                )
                .is_err()
                {
                    continue;
                }
                // Pin all DMA memory even on failure; a timed-out controller can retain pointers.
                let Some(physical) = crate::PMM.lock().alloc_frames(crate::usb_xhci::DMA_PAGES)
                else {
                    return;
                };
                let dma = (physical.as_u64() + hhdm) as *mut u8;
                unsafe {
                    core::ptr::write_bytes(dma, 0, crate::usb_xhci::DMA_PAGES * 4096);
                }
                RESERVED[count].store(bar, Ordering::Release);
                crate::serial_println!(
                    "[USB-MSC] probing PCI {}:{}:{} bar={:#x}",
                    bus,
                    slot,
                    function,
                    bar
                );
                let controller = unsafe {
                    Controller::initialize(
                        virtual_bar as *mut u8,
                        size as usize,
                        dma,
                        physical.as_u64(),
                    )
                };
                match controller {
                    Ok(controller) => {
                        // Keep ownership even on SCSI/geometry rejection. Ring-3 cannot reset this controller.
                        match Disk::initialize(controller) {
                            Ok(mut disk) => {
                                let operations = sunlight_block::registry::READ
                                    | sunlight_block::registry::WRITE
                                    | sunlight_block::registry::FLUSH
                                    | sunlight_block::registry::USB;
                                let Some(id) = crate::storage::DEVICES
                                    .lock()
                                    .register_disk(disk.block_count(), operations)
                                else {
                                    return;
                                };
                                match sunlight_block::partition::discover(&mut disk) {
                                    Ok(table) => {
                                        if crate::storage::DEVICES
                                            .lock()
                                            .register_partitions(id, &table)
                                            .is_none()
                                        {
                                            crate::serial_println!(
                                                "[USB-MSC] partition registry exhausted"
                                            );
                                        }
                                        for p in table.partitions() {
                                            crate::serial_println!(
                                                "[USB-MSC] {:?} parent={} start={} blocks={}",
                                                table.layout,
                                                id,
                                                p.start,
                                                p.blocks
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        crate::serial_println!(
                                            "[USB-MSC] media discovery rejected: {:?}",
                                            e
                                        );
                                    }
                                }
                                crate::serial_println!("[USB-MSC] registered USB-backed disk id={} lun=0 blocks={} sector=512",id,disk.block_count());
                                *DISK.lock() = Some(disk);
                                DEVICE_ID.store(id, Ordering::Release);
                            }
                            Err(e) => {
                                crate::serial_println!(
                                    "[USB-MSC] SCSI/sector profile rejected: {:?}",
                                    e
                                );
                            }
                        }
                        return;
                    }
                    Err(e) => {
                        crate::serial_println!("[USB-MSC] controller profile rejected: {:?}", e);
                        // Stop before permitting legacy HID to reinitialize. If halt cannot
                        // be confirmed, retain reservation and all DMA buffers permanently.
                        let mmio = virtual_bar as *mut u8;
                        let op = unsafe { mmio.add(core::ptr::read_volatile(mmio) as usize) };
                        unsafe {
                            let cmd = op.cast::<u32>();
                            core::ptr::write_volatile(cmd, core::ptr::read_volatile(cmd) & !1);
                        }
                        let start = unsafe { core::arch::x86_64::_rdtsc() };
                        let budget = crate::arch::x86_64::interrupts::tsc_hz();
                        while unsafe { core::ptr::read_volatile(op.add(4).cast::<u32>()) } & 1 == 0
                            && unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < budget
                        {
                            core::hint::spin_loop();
                        }
                        if unsafe { core::ptr::read_volatile(op.add(4).cast::<u32>()) } & 1 != 0 {
                            RESERVED[count].store(0, Ordering::Release);
                        }
                        count += 1;
                    }
                }
            }
        }
    }
}
