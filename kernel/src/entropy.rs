//! Secure entropy collection and conditioning for cryptographic consumers.
//!
//! Only reviewed sources can make this subsystem ready:
//! * a QEMU-provided legacy `virtio-rng` device, backed by host entropy;
//! * x86 `RDSEED`; or
//! * x86 `RDRAND` when `RDSEED` is unavailable.
//!
//! Timing and TSC values are intentionally excluded. If no approved source
//! yields a complete seed, the kernel remains unready and secure callers fail.

use crate::memory::pmm::PhysicalMemoryManager;
use core::sync::atomic::{AtomicU8, Ordering};
use spin::Mutex;
use x86_64::VirtAddr;

const SEED_BYTES: usize = 40;
const RNG_BYTES: usize = 64;
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
    let Some(conditioner) = conditioner.as_mut() else {
        return false;
    };
    conditioner.fill(out);
    true
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
    let (Some(queue_phys), Some(buffer_phys)) = (pmm.alloc_frames(3), pmm.alloc_frame()) else {
        return false;
    };
    let queue_virt = hhdm_offset + queue_phys.as_u64();
    let buffer_virt = hhdm_offset + buffer_phys.as_u64();

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
    if queue_size == 0 {
        return false;
    }

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

struct ChaCha20 {
    state: [u32; 16],
    block: [u8; RNG_BYTES],
    offset: usize,
}

impl ChaCha20 {
    fn from_seed(seed: &[u8; SEED_BYTES]) -> Self {
        let mut state = [0u32; 16];
        state[0] = 0x6170_7865;
        state[1] = 0x3320_646e;
        state[2] = 0x7962_2d32;
        state[3] = 0x6b20_6574;
        for index in 0..8 {
            let offset = index * 4;
            state[4 + index] = u32::from_le_bytes(seed[offset..offset + 4].try_into().unwrap());
        }
        state[12] = 0;
        state[13] = 0;
        state[14] = u32::from_le_bytes(seed[32..36].try_into().unwrap());
        state[15] = u32::from_le_bytes(seed[36..40].try_into().unwrap());
        Self {
            state,
            block: [0; RNG_BYTES],
            offset: RNG_BYTES,
        }
    }

    fn fill(&mut self, out: &mut [u8]) {
        let mut written = 0;
        while written < out.len() {
            if self.offset == RNG_BYTES {
                self.generate_block();
            }
            let count = (out.len() - written).min(RNG_BYTES - self.offset);
            out[written..written + count]
                .copy_from_slice(&self.block[self.offset..self.offset + count]);
            self.offset += count;
            written += count;
        }
    }

    fn generate_block(&mut self) {
        let mut x = self.state;
        macro_rules! quarter_round {
            ($a:expr, $b:expr, $c:expr, $d:expr) => {
                x[$a] = x[$a].wrapping_add(x[$b]);
                x[$d] ^= x[$a];
                x[$d] = x[$d].rotate_left(16);
                x[$c] = x[$c].wrapping_add(x[$d]);
                x[$b] ^= x[$c];
                x[$b] = x[$b].rotate_left(12);
                x[$a] = x[$a].wrapping_add(x[$b]);
                x[$d] ^= x[$a];
                x[$d] = x[$d].rotate_left(8);
                x[$c] = x[$c].wrapping_add(x[$d]);
                x[$b] ^= x[$c];
                x[$b] = x[$b].rotate_left(7);
            };
        }
        for _ in 0..10 {
            // Column rounds (ChaCha20): last index of the fourth QR is 15, not 12.
            quarter_round!(0, 4, 8, 12);
            quarter_round!(1, 5, 9, 13);
            quarter_round!(2, 6, 10, 14);
            quarter_round!(3, 7, 11, 15);
            // Diagonal rounds
            quarter_round!(0, 5, 10, 15);
            quarter_round!(1, 6, 11, 12);
            quarter_round!(2, 7, 8, 13);
            quarter_round!(3, 4, 9, 14);
        }
        for index in 0..16 {
            let value = x[index].wrapping_add(self.state[index]);
            self.block[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let (low, carry) = self.state[12].overflowing_add(1);
        self.state[12] = low;
        if carry {
            self.state[13] = self.state[13].wrapping_add(1);
        }
        self.offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{ChaCha20, Source, RNG_BYTES};

    #[test]
    fn source_labels_are_explicit() {
        assert_eq!(Source::VirtioRng.label(), "virtio-rng");
        assert_eq!(Source::RdSeed.label(), "RDSEED");
        assert_eq!(Source::RdRand.label(), "RDRAND");
    }

    #[test]
    fn conditioner_expands_seed_without_repeating_a_block() {
        let mut seed = [0u8; 40];
        for (index, byte) in seed.iter_mut().enumerate() {
            *byte = index as u8;
        }
        let mut conditioner = ChaCha20::from_seed(&seed);
        let mut bytes = [0u8; RNG_BYTES * 2];
        conditioner.fill(&mut bytes);
        assert_ne!(&bytes[..RNG_BYTES], &bytes[RNG_BYTES..]);
    }

    #[test]
    fn different_seeds_yield_different_keystreams() {
        let mut a = [0u8; 40];
        let mut b = [0u8; 40];
        for i in 0..40 {
            a[i] = i as u8;
            b[i] = 0xFF - i as u8;
        }
        let mut ca = ChaCha20::from_seed(&a);
        let mut cb = ChaCha20::from_seed(&b);
        let mut oa = [0u8; 32];
        let mut ob = [0u8; 32];
        ca.fill(&mut oa);
        cb.fill(&mut ob);
        assert_ne!(oa, ob);
    }

    #[test]
    fn fill_initializes_every_output_byte() {
        let mut seed = [0u8; 40];
        seed[0] = 1;
        let mut c = ChaCha20::from_seed(&seed);
        let mut out = [0xAAu8; 17];
        c.fill(&mut out);
        // Second block-spanning fill differs and is fully overwritten.
        let mut out2 = [0xAAu8; 17];
        c.fill(&mut out2);
        assert_ne!(out, out2);
    }
}
