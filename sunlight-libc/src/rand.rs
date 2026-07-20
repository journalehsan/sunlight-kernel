//! Userland random number generation.
//!
//! Two tiers, matching SunlightOS's microkernel split:
//!
//! * **Fast / non-crypto** (`GRND_NONCRYPTO`): a local `xoroshiro64**` generator
//!   seeded from the kernel's conditioned entropy stream. Runs entirely in the
//!   caller's address space — no IPC, no allocation. It also fails if secure
//!   entropy is unavailable, avoiding a predictable fallback state.
//!
//! * **Cryptographic** (default): delegated over capability IPC to the `rand`
//!   service (ring 3, spawned by sunlightd), which runs a ChaCha20 CSPRNG. The
//!   request is chunked into 32-byte register-IPC replies (the real inline
//!   budget — `words[0..3]`), so any length works without shared memory.
//!
//! The crypto path **fails closed**: if the service is dead or unreachable,
//! `getrandom` returns `-1` rather than silently downgrading to the non-crypto
//! engine. Callers (e.g. TLS key generation) must treat that as fatal.

use crate::sys::{syscall0, SYS_GET_ENTROPY, SYS_SECURE_ENTROPY_READY};
use sunlight_ipc::{ipc_call, nameserver_lookup, IpcMsg, RandMsg};

/// Service the request locally with the fast non-crypto generator.
pub const GRND_NONCRYPTO: u32 = 0x0001;

/// Whether the kernel approved-source collector completed successfully.
#[inline]
fn secure_entropy_ready() -> bool {
    // SAFETY: SecureEntropyReady takes no arguments and touches no user memory.
    unsafe { syscall0(SYS_SECURE_ENTROPY_READY) == 1 }
}

/// One conditioned entropy word from the kernel.
#[inline]
fn raw_entropy() -> u64 {
    // SAFETY: GetEntropy takes no arguments and touches no user memory.
    unsafe { syscall0(SYS_GET_ENTROPY) }
}

/// `xoroshiro64**` (Blackman & Vigna) — fast, 64-bit state, NON-cryptographic.
pub struct Xoroshiro64StarStar {
    s: [u32; 2],
}

impl Xoroshiro64StarStar {
    /// Seed from one kernel entropy word. The all-zero state is invalid for
    /// xoroshiro, so reject it instead of installing a predictable fallback.
    pub fn new() -> Option<Self> {
        if !secure_entropy_ready() {
            return None;
        }
        let seed = raw_entropy();
        let s = [seed as u32, (seed >> 32) as u32];
        if s[0] == 0 && s[1] == 0 {
            return None;
        }
        Some(Self { s })
    }

    pub fn next_u32(&mut self) -> u32 {
        let s0 = self.s[0];
        let mut s1 = self.s[1];
        // result = rotl(s0 * 0x9E3779BB, 5) * 5
        let result = s0.wrapping_mul(0x9E37_79BB).rotate_left(5).wrapping_mul(5);
        s1 ^= s0;
        self.s[0] = s0.rotate_left(26) ^ s1 ^ (s1 << 9);
        self.s[1] = s1.rotate_left(13);
        result
    }

    /// Fill `buf` with non-crypto random bytes.
    pub fn fill_bytes(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(4) {
            let bytes = self.next_u32().to_le_bytes();
            let n = chunk.len();
            chunk.copy_from_slice(&bytes[..n]);
        }
    }
}

/// Crypto path: pull `buf.len()` bytes from the `rand` service, 32 bytes per
/// IPC round-trip. Returns `false` on any failure (service missing, malformed
/// reply) so the caller can fail closed.
fn fill_secure(buf: &mut [u8]) -> bool {
    if !secure_entropy_ready() {
        return false;
    }
    let Some(cap) = nameserver_lookup("rand") else {
        return false;
    };

    let mut off = 0usize;
    while off < buf.len() {
        let want = core::cmp::min(buf.len() - off, RandMsg::MAX_CHUNK);
        let reply = ipc_call(cap, IpcMsg::with_label(RandMsg::GET).word(0, want as u64));
        if reply.label != RandMsg::REPLY {
            return false;
        }
        let got = want;
        if got == 0 || got > RandMsg::MAX_CHUNK {
            return false;
        }
        // Unpack up to 32 bytes from words[0..3].
        let mut tmp = [0u8; RandMsg::MAX_CHUNK];
        for i in 0..4 {
            tmp[i * 8..i * 8 + 8].copy_from_slice(&reply.words[i].to_le_bytes());
        }
        buf[off..off + got].copy_from_slice(&tmp[..got]);
        off += got;
    }
    true
}

/// Fill `buf` with random bytes.
///
/// Returns the number of bytes written, or `-1` on failure. With
/// `GRND_NONCRYPTO` it always succeeds locally; without it, a failure of the
/// `rand` service yields `-1` (fail closed — never a silent downgrade).
pub fn getrandom(buf: &mut [u8], flags: u32) -> isize {
    if flags & GRND_NONCRYPTO != 0 {
        return match Xoroshiro64StarStar::new() {
            Some(mut rng) => {
                rng.fill_bytes(buf);
                buf.len() as isize
            }
            None => -1,
        };
    }
    if fill_secure(buf) {
        buf.len() as isize
    } else {
        -1
    }
}
