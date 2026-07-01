//! Geekbench-style CPU mix benchmark.
//!
//! This stays fully in safe Rust: a bounded integer mix pass, plus SHA-256 over
//! a fixed in-memory working set. No raw pointers, no syscalls, and no unbounded
//! recursion or allocation growth.

use crate::bench::rdtsc;
use crate::sha256::sha256;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;

pub const NAME: &str = "CPU Mix (Geekbench-style)";

const WORKSET_BYTES: usize = 32 * 1024;
const HASH_WINDOW: usize = 4 * 1024;
const ROUNDS: u64 = 192;
const ROUNDS_PER_STEP: u64 = 4;

pub struct CpuRunner {
    buf: Vec<u8>,
    round: u64,
    mix: u64,
    digest: [u32; 8],
    cycles: u64,
}

impl CpuRunner {
    pub fn new() -> Self {
        let mut buf = vec![0u8; WORKSET_BYTES];
        let mut seed = 0xA5A5_5A5A_F0F0_0F0Fu64;
        for byte in &mut buf {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            *byte = (seed >> 24) as u8;
        }

        Self {
            buf,
            round: 0,
            mix: 0x9E37_79B9_7F4A_7C15,
            digest: [0; 8],
            cycles: 0,
        }
    }

    pub fn name(&self) -> &'static str {
        NAME
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    pub fn progress_bp(&self) -> u16 {
        ((self.round.saturating_mul(10_000)) / ROUNDS) as u16
    }

    pub fn step(&mut self) -> bool {
        if self.round >= ROUNDS {
            return true;
        }

        let start = rdtsc();
        let end = (self.round + ROUNDS_PER_STEP).min(ROUNDS);
        while self.round < end {
            self.run_round();
            self.round += 1;
        }
        self.cycles = self.cycles.saturating_add(rdtsc() - start);

        if self.round >= ROUNDS {
            black_box((self.mix, self.digest));
            true
        } else {
            false
        }
    }

    fn run_round(&mut self) {
        let round_tag = self.round as u8;
        let mut acc = self.mix ^ self.round.wrapping_mul(0xD6E8_FEB8_6659_FD93);

        for block in self.buf[..HASH_WINDOW].chunks_exact(64) {
            let mut local = acc ^ (block[0] as u64);
            for &byte in block {
                local ^= byte as u64;
                local = local.rotate_left(7).wrapping_mul(0x9E37_79B1_85EB_CA87);
            }
            acc ^= local.rotate_left((round_tag & 31) as u32);
        }

        let salt = round_tag.wrapping_add((acc & 0xFF) as u8);
        self.buf[0] ^= salt;
        self.buf[1] = self.buf[1].wrapping_add((acc >> 8) as u8);
        self.buf[2] ^= (acc >> 16) as u8;
        self.mix = acc;
        self.digest = sha256(&self.buf[..HASH_WINDOW]);
    }
}
