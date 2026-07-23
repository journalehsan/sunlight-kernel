#![no_std]
#![no_main]

//! SunlightOS random service ("rand").
//!
//! A ring-3 CSPRNG, spawned and supervised by sunlightd. libc's crypto path
//! (`getrandom` without `GRND_NONCRYPTO`) routes here over capability IPC. The
//! engine is ChaCha20, keyed only after the kernel's approved-source entropy
//! collector reports ready. It never falls back to TSC-derived bytes or the
//! non-cryptographic xoroshiro generator.
//!
//! Wire protocol (`RandMsg`): a GET carries the requested length in `words[0]`,
//! clamped to 32 bytes (the register-IPC inline budget). The REPLY packs exactly
//! that many bytes into `words[0..3]`. Callers wanting more loop; nothing here
//! uses shared memory. STATS is an additive, non-sensitive telemetry query.

use rand_service::engine::{secure_wipe, ChaCha20, EntropySource, ReseedReason, BLOCK_BYTES};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg, RandMsg,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Kernel conditioned secure seed word (syscall 87).
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

#[inline]
fn secure_entropy_ready() -> bool {
    let ret: u64;
    // SAFETY: syscall 89 takes no arguments and touches no memory.
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 89u64,
            lateout("rax") ret,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret == 1
}

/// Production entropy backend: kernel conditioned stream via syscalls.
struct KernelEntropy;

impl EntropySource for KernelEntropy {
    fn ready(&mut self) -> bool {
        secure_entropy_ready()
    }

    fn next_u64(&mut self) -> Option<u64> {
        if !secure_entropy_ready() {
            return None;
        }
        // The kernel returns 0 when unready; we already gated on ready, but a
        // single zero word is a legitimate (if rare) conditioned sample. The
        // engine rejects an all-zero *key* after collecting the full seed.
        Some(raw_entropy())
    }
}

fn handle(msg: &IpcMsg, rng: &mut ChaCha20, src: &mut KernelEntropy) -> IpcMsg {
    match msg.label {
        RandMsg::GET => {
            // Reject lengths that cannot be expressed as a chunk (only the low
            // word is used). Clamp to the register-IPC budget.
            let requested = msg.words[0];
            // Overflow-safe clamp: values larger than MAX_CHUNK become MAX_CHUNK.
            let want = if requested == 0 {
                0usize
            } else if requested > RandMsg::MAX_CHUNK as u64 {
                RandMsg::MAX_CHUNK
            } else {
                requested as usize
            };

            if want == 0 {
                rng.record_rejection();
                return IpcMsg::with_label(RandMsg::ERROR);
            }

            let mut buf = [0u8; RandMsg::MAX_CHUNK];
            if !rng.fill(&mut buf[..want], src) {
                secure_wipe(&mut buf);
                return IpcMsg::with_label(RandMsg::ERROR);
            }

            // Pack the requested bytes into words[0..3]. Unused trailing bytes
            // in the 32-byte window stay zero and are not generator output.
            let mut reply = IpcMsg::with_label(RandMsg::REPLY);
            for i in 0..4 {
                let mut w = [0u8; 8];
                w.copy_from_slice(&buf[i * 8..i * 8 + 8]);
                reply = reply.word(i, u64::from_le_bytes(w));
            }
            secure_wipe(&mut buf);
            reply
        }
        RandMsg::STATS => {
            let s = rng.stats();
            // Pack counters into 16-bit lanes; overflow saturates (telemetry only).
            let reseed = s.reseed_count.min(0xFFFF);
            let ent_fail = s.entropy_failures.min(0xFFFF);
            let rejected = s.rejected_requests.min(0xFFFF);
            let not_ready = s.not_ready_count.min(0xFFFF);
            let packed = reseed | (ent_fail << 16) | (rejected << 32) | (not_ready << 48);
            IpcMsg::with_label(RandMsg::REPLY)
                .word(0, u64::from(s.ready))
                .word(1, s.total_requests)
                .word(2, s.total_bytes)
                .word(3, packed)
                .word(4, s.last_reseed_reason.as_u64())
                // word 5: block size / reseed threshold (non-sensitive policy).
                .word(5, (BLOCK_BYTES as u64) | (RESEED_THRESHOLD_BLOCKS << 32))
        }
        _ => {
            rng.record_rejection();
            IpcMsg::with_label(RandMsg::ERROR)
        }
    }
}

/// Re-export the engine constant under a local name for the STATS word packing.
const RESEED_THRESHOLD_BLOCKS: u64 = rand_service::RESEED_BLOCKS;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[RAND] Starting rand_service");
    if !secure_entropy_ready() {
        debug_log("[RAND] secure entropy unavailable; refusing to register");
        loop {
            core::hint::spin_loop();
        }
    }

    let mut src = KernelEntropy;
    // Service restart always reseeds from fresh kernel entropy — never from
    // wall-clock, PID, TSC, or a saved stream position.
    let Some(mut rng) = ChaCha20::new(&mut src, ReseedReason::ServiceRestart) else {
        debug_log("[RAND] initial seed failed; refusing to register");
        loop {
            core::hint::spin_loop();
        }
    };

    let ep = endpoint_create();
    nameserver_register("rand", ep);
    debug_log("[RAND] Registered as 'rand'");

    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle(&msg, &mut rng, &mut src);
        msg = ipc_reply_and_wait(ep, reply);
    }
}
