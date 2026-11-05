//! Secure entropy collection and conditioning for cryptographic consumers.
//!
//! Only reviewed sources can make this subsystem ready:
//! * a QEMU-provided legacy `virtio-rng` device, backed by host entropy;
//! * x86 `RDSEED`; or
//! * x86 `RDRAND` when `RDSEED` is unavailable.
//!
//! Timing and TSC values are intentionally excluded. If no approved source
//! yields a complete seed, the kernel remains unready and secure callers fail.

mod conditioner;
use conditioner::ChaCha20;

use crate::memory::pmm::PhysicalMemoryManager;
use core::sync::atomic::{AtomicU8, Ordering};
use spin::Mutex;
use x86_64::VirtAddr;

const SEED_BYTES: usize = 40;
const STATUS_UNREADY: u8 = 0;
const STATUS_VIRTIO: u8 = 1;
const STATUS_RDSEED: u8 = 2;
const STATUS_RDRAND: u8 = 3;

static STATUS: AtomicU8 = AtomicU8::new(STATUS_UNREADY);
static CONDITIONER: Mutex<Option<ChaCha20>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    VirtioRng,
    RdSeed,
    RdRand,
}

impl Source {
    const fn status(self) -> u8 {
        match self {
            Self::VirtioRng => STATUS_VIRTIO,
            Self::RdSeed => STATUS_RDSEED,
            Self::RdRand => STATUS_RDRAND,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::VirtioRng => "virtio-rng",
            Self::RdSeed => "RDSEED",
            Self::RdRand => "RDRAND",
        }
    }
}

/// Collect, condition, and mark entropy ready. This runs before user-space.
///
/// `false` means no approved source was available. Callers must leave
/// cryptographic services unavailable in that case.
pub fn init(pmm: &mut PhysicalMemoryManager, hhdm_offset: VirtAddr) -> Option<Source> {
    // Boot-only initialization must never restart an already published stream.
    if let Some(source) = source() {
        return Some(source);
    }
    let mut seed = [0u8; SEED_BYTES];
    let source = collect_seed(&mut seed, pmm, hhdm_offset)?;
    *CONDITIONER.lock() = Some(ChaCha20::from_seed(&seed));
    // Wipe the stack seed so it is not left readable after init.
    for byte in seed.iter_mut() {
        // SAFETY: `byte` points into the local `seed` array.
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    STATUS.store(source.status(), Ordering::Release);
    Some(source)
}

/// Whether cryptographic randomness may be issued.
#[inline]
pub fn is_ready() -> bool {
    STATUS.load(Ordering::Acquire) != STATUS_UNREADY
}

/// The source that qualified the current boot, if any.
pub fn source() -> Option<Source> {
    match STATUS.load(Ordering::Acquire) {
        STATUS_VIRTIO => Some(Source::VirtioRng),
        STATUS_RDSEED => Some(Source::RdSeed),
        STATUS_RDRAND => Some(Source::RdRand),
        _ => None,
    }
}

/// Fill `out` with conditioned cryptographic bytes.
///
/// Returns `false` instead of manufacturing a fallback when the subsystem is
/// not ready. The syscall layer converts that to its documented error result.
pub fn fill(out: &mut [u8]) -> bool {
    let mut conditioner = CONDITIONER.lock();
    if !is_ready() {
        return false;
    }
    let Some(conditioner) = conditioner.as_mut() else {
        return false;
    };
    let filled = conditioner.fill(out);
    if !filled {
        STATUS.store(STATUS_UNREADY, Ordering::Release);
    }
    filled
}

pub fn next_u64() -> Option<u64> {
    let mut bytes = [0u8; 8];
    fill(&mut bytes).then(|| u64::from_le_bytes(bytes))
}

fn collect_seed(
    seed: &mut [u8; SEED_BYTES],
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Option<Source> {
    if unsafe { fill_virtio_rng(seed, pmm, hhdm_offset.as_u64()) } {
        return Some(Source::VirtioRng);
    }
    if fill_rdseed(seed) {
        return Some(Source::RdSeed);
    }
    fill_rdrand(seed).then_some(Source::RdRand)
}

const VIRTIO_REG_DEVICE_FEATURES: u16 = 0x00;
const VIRTIO_REG_DRIVER_FEATURES: u16 = 0x04;
const VIRTIO_REG_QUEUE_PFN: u16 = 0x08;
const VIRTIO_REG_QUEUE_NUM: u16 = 0x0c;
const VIRTIO_REG_QUEUE_SEL: u16 = 0x0e;
const VIRTIO_REG_QUEUE_NOTIFY: u16 = 0x10;
const VIRTIO_REG_DEVICE_STATUS: u16 = 0x12;
const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTQ_DESC_F_WRITE: u16 = 2;

#[repr(C)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

/// Collect boot seed bytes from QEMU's reviewed host-entropy device.
///
/// SAFETY: called during single-threaded kernel boot while the PMM owns the
/// queue and buffer frames and ring-0 port I/O is available.
unsafe fn fill_virtio_rng(
    out: &mut [u8],
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: u64,
) -> bool {
    use sunlight_virtio::pci::{inl, inw, outb, outl, outw};

    let Some((_bus, _slot, _func, io_base)) = sunlight_virtio::find_virtio_rng() else {
        return false;
    };
    if out.is_empty() || out.len() > 4096 {
        return false;
    }
    outb(io_base + VIRTIO_REG_DEVICE_STATUS, 0);
    outb(
        io_base + VIRTIO_REG_DEVICE_STATUS,
        VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER,
    );
    outl(
        io_base + VIRTIO_REG_DRIVER_FEATURES,
        inl(io_base + VIRTIO_REG_DEVICE_FEATURES),
    );
    outw(io_base + VIRTIO_REG_QUEUE_SEL, 0);
    let queue_size = inw(io_base + VIRTIO_REG_QUEUE_NUM);
    // The fixed three-page layout supports at most 256 descriptors. Reject
    // larger queues before touching memory outside that allocation.
    if queue_size == 0 || queue_size > 256 || !queue_size.is_power_of_two() {
        return false;
    }
    let Some(queue_phys) = pmm.alloc_frames(3) else {
        return false;
    };
    let Some(buffer_phys) = pmm.alloc_frame() else {
        for index in 0..3 {
            pmm.free_frame(queue_phys + index * 4096u64);
        }
        return false;
    };
    let queue_virt = hhdm_offset + queue_phys.as_u64();
    let buffer_virt = hhdm_offset + buffer_phys.as_u64();

    (queue_virt as *mut u8).write_bytes(0, 3 * 4096);
    (buffer_virt as *mut u8).write_bytes(0, 4096);
    outl(
        io_base + VIRTIO_REG_QUEUE_PFN,
        (queue_phys.as_u64() >> 12) as u32,
    );
    outb(
        io_base + VIRTIO_REG_DEVICE_STATUS,
        VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK,
    );

    let desc = queue_virt as *mut VirtqDesc;
    (*desc).addr = buffer_phys.as_u64();
    (*desc).len = out.len() as u32;
    (*desc).flags = VIRTQ_DESC_F_WRITE;
    (*desc).next = 0;

    let avail = queue_virt + queue_size as u64 * 16;
    ((avail + 4) as *mut u16).write_volatile(0);
    ((avail + 2) as *mut u16).write_volatile(1);
    core::sync::atomic::fence(Ordering::SeqCst);
    outw(io_base + VIRTIO_REG_QUEUE_NOTIFY, 0);

    let used = (avail + 6 + queue_size as u64 * 2 + 4095) & !4095;
    let used_idx = (used + 2) as *const u16;
    let mut spins = 50_000_000u32;
    while used_idx.read_volatile() == 0 {
        spins = spins.saturating_sub(1);
        if spins == 0 {
            return false;
        }
        core::hint::spin_loop();
    }
    core::sync::atomic::fence(Ordering::SeqCst);
    let used_len = (used + 8) as *const u32;
    if used_len.read_volatile() < out.len() as u32 {
        return false;
    }
    out.copy_from_slice(core::slice::from_raw_parts(
        buffer_virt as *const u8,
        out.len(),
    ));
    // Completion proves the submitted write finished. Disable the queue before
    // reuse; if reset is not acknowledged, conservatively retain DMA backing.
    outb(io_base + VIRTIO_REG_DEVICE_STATUS, 0);
    if sunlight_virtio::pci::inb(io_base + VIRTIO_REG_DEVICE_STATUS) == 0 {
        for offset in 0..4096 {
            (buffer_virt as *mut u8).add(offset).write_volatile(0);
        }
        core::sync::atomic::fence(Ordering::SeqCst);
        pmm.free_frame(buffer_phys);
        for index in 0..3 {
            pmm.free_frame(queue_phys + index * 4096u64);
        }
    }
    true
}

fn fill_rdseed(out: &mut [u8]) -> bool {
    let has_rdseed = core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 18) != 0;
    has_rdseed && fill_words(out, rdseed_word)
}

fn fill_rdrand(out: &mut [u8]) -> bool {
    let has_rdrand = core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0;
    has_rdrand && fill_words(out, rdrand_word)
}

fn fill_words(out: &mut [u8], word: fn() -> Option<u64>) -> bool {
    for chunk in out.chunks_mut(8) {
        let Some(value) = word() else {
            return false;
        };
        chunk.copy_from_slice(&value.to_le_bytes()[..chunk.len()]);
    }
    true
}

fn rdseed_word() -> Option<u64> {
    for _ in 0..10 {
        let value: u64;
        let ok: u8;
        unsafe {
            core::arch::asm!(
                "rdseed {value}",
                "setc {ok}",
                value = out(reg) value,
                ok = out(reg_byte) ok,
                options(nomem, nostack),
            );
        }
        if ok != 0 {
            return Some(value);
        }
    }
    None
}

fn rdrand_word() -> Option<u64> {
    for _ in 0..10 {
        let value: u64;
        let ok: u8;
        unsafe {
            core::arch::asm!(
                "rdrand {value}",
                "setc {ok}",
                value = out(reg) value,
                ok = out(reg_byte) ok,
                options(nomem, nostack),
            );
        }
        if ok != 0 {
            return Some(value);
        }
    }
    None
}
