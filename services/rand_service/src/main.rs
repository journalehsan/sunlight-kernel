#![no_std]
#![no_main]

//! SunlightOS random service ("rand").
//!
//! A ring-3 CSPRNG, spawned and supervised by sunlightd. libc's crypto path
//! (`getrandom` without `GRND_NONCRYPTO`) routes here over capability IPC. The
//! engine is ChaCha20, keyed from the kernel's `GetEntropy` syscall and
//! periodically reseeded.
//!
//! Wire protocol (`RandMsg`): a GET carries the requested length in `words[0]`,
//! clamped to 32 bytes (the register-IPC inline budget). The REPLY packs exactly
//! that many bytes into `words[0..3]`. Callers wanting more loop; nothing here
//! uses shared memory.

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg, RandMsg,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Raw seed word from the kernel (RDRAND / TSC jitter). Syscall 87 takes no
/// arguments and returns one u64 in rax.
#[inline]
fn raw_entropy() -> u64 {
    let ret: u64;
    // SAFETY: GetEntropy clobbers rcx/r11 per the SYSCALL ABI and touches no memory.
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 87u64,
            lateout("rax") ret,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Reseed after this many produced blocks (64 bytes each).
const RESEED_BLOCKS: u64 = 8192;

struct ChaCha20 {
    state: [u32; 16],
    blocks_since_reseed: u64,
}

impl ChaCha20 {
    fn new() -> Self {
        let mut c = Self {
            state: [0; 16],
            blocks_since_reseed: 0,
        };
        c.reseed();
        c
    }

    /// Establish a fresh 256-bit key + 64-bit nonce from kernel entropy and
    /// reset the block counter.
    fn reseed(&mut self) {
        // "expand 32-byte k"
        self.state[0] = 0x6170_7865;
        self.state[1] = 0x3320_646e;
        self.state[2] = 0x7962_2d32;
        self.state[3] = 0x6b20_6574;
        // 256-bit key (words 4..12) from 4 entropy reads.
        for i in 0..4 {
            let e = raw_entropy();
            self.state[4 + i * 2] = e as u32;
            self.state[4 + i * 2 + 1] = (e >> 32) as u32;
        }
        // 64-bit block counter (12..14) starts at 0.
        self.state[12] = 0;
        self.state[13] = 0;
        // 64-bit nonce (14..16) from one more entropy read.
        let n = raw_entropy();
        self.state[14] = n as u32;
        self.state[15] = (n >> 32) as u32;
        self.blocks_since_reseed = 0;
    }

    fn fill(&mut self, out: &mut [u8]) {
        if self.blocks_since_reseed >= RESEED_BLOCKS {
            self.reseed();
        }
        let mut idx = 0;
        let mut block = [0u8; 64];
        while idx < out.len() {
            self.block(&mut block);
            // 64-bit little-endian counter increment.
            let (lo, carry) = self.state[12].overflowing_add(1);
            self.state[12] = lo;
            if carry {
                self.state[13] = self.state[13].wrapping_add(1);
            }
            self.blocks_since_reseed += 1;
            let n = core::cmp::min(out.len() - idx, 64);
            out[idx..idx + n].copy_from_slice(&block[..n]);
            idx += n;
        }
    }

    fn block(&self, out: &mut [u8; 64]) {
        let mut x = self.state;
        macro_rules! qr {
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
            qr!(0, 4, 8, 12);
            qr!(1, 5, 9, 13);
            qr!(2, 6, 10, 14);
            qr!(3, 7, 11, 15);
            qr!(0, 5, 10, 15);
            qr!(1, 6, 11, 12);
            qr!(2, 7, 8, 13);
            qr!(3, 4, 9, 14);
        }
        for i in 0..16 {
            let v = x[i].wrapping_add(self.state[i]);
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

fn handle(msg: &IpcMsg, rng: &mut ChaCha20) -> IpcMsg {
    match msg.label {
        RandMsg::GET => {
            let want = core::cmp::min(msg.words[0] as usize, RandMsg::MAX_CHUNK);
            let mut buf = [0u8; RandMsg::MAX_CHUNK];
            rng.fill(&mut buf[..want]);
            // Pack the requested bytes into words[0..3].
            let mut reply = IpcMsg::with_label(RandMsg::REPLY);
            for i in 0..4 {
                let mut w = [0u8; 8];
                w.copy_from_slice(&buf[i * 8..i * 8 + 8]);
                reply = reply.word(i, u64::from_le_bytes(w));
            }
            reply
        }
        _ => IpcMsg::with_label(RandMsg::ERROR),
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[RAND] Starting rand_service");
    let mut rng = ChaCha20::new();

    let ep = endpoint_create();
    nameserver_register("rand", ep);
    debug_log("[RAND] Registered as 'rand'");

    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle(&msg, &mut rng);
        msg = ipc_reply_and_wait(ep, reply);
    }
}
