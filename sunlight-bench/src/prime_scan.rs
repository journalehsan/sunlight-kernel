//! Lightweight dense prime sieve benchmark.

use crate::bench::rdtsc;
use alloc::vec::Vec;
use core::hint::black_box;

pub const NAME: &str = "Prime Sieve 100k (dense)";

const LIMIT: usize = 100_000;
const ROOT_LIMIT: usize = 317;
const MARK_BUDGET: usize = 8_192;
const COUNT_BUDGET: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Mark,
    Count,
    Done,
}

pub struct PrimeRunner {
    is_prime: Vec<u8>,
    phase: Phase,
    p: usize,
    next_multiple: usize,
    count_index: usize,
    count: u64,
    cycles: u64,
}

impl PrimeRunner {
    pub fn new() -> Self {
        let mut is_prime = alloc::vec![1u8; LIMIT];
        is_prime[0] = 0;
        is_prime[1] = 0;
        Self {
            is_prime,
            phase: Phase::Mark,
            p: 2,
            next_multiple: 0,
            count_index: 2,
            count: 0,
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
        match self.phase {
            Phase::Mark => ((self.p.min(ROOT_LIMIT) as u32 * 8_000) / ROOT_LIMIT as u32) as u16,
            Phase::Count => {
                8_000 + ((self.count_index.min(LIMIT) as u32 * 2_000) / LIMIT as u32) as u16
            }
            Phase::Done => 10_000,
        }
    }

    pub fn release(&mut self) {
        self.is_prime = alloc::vec::Vec::new();
    }

    pub fn step(&mut self) -> bool {
        if self.phase == Phase::Done {
            return true;
        }

        let start = rdtsc();
        match self.phase {
            Phase::Mark => self.step_mark(),
            Phase::Count => self.step_count(),
            Phase::Done => {}
        }
        self.cycles = self.cycles.saturating_add(rdtsc() - start);

        if self.phase == Phase::Done {
            black_box(self.count);
            true
        } else {
            false
        }
    }

    fn step_mark(&mut self) {
        let mut budget = MARK_BUDGET;
        while self.p < ROOT_LIMIT && budget > 0 {
            if self.is_prime[self.p] == 0 {
                self.p += 1;
                self.next_multiple = 0;
                continue;
            }

            if self.next_multiple == 0 {
                self.next_multiple = self.p * self.p;
            }

            while self.next_multiple < LIMIT && budget > 0 {
                self.is_prime[self.next_multiple] = 0;
                self.next_multiple += self.p;
                budget -= 1;
            }

            if self.next_multiple >= LIMIT {
                self.p += 1;
                self.next_multiple = 0;
            }
        }

        if self.p >= ROOT_LIMIT {
            self.phase = Phase::Count;
            self.count_index = 2;
            self.count = 0;
        }
    }

    fn step_count(&mut self) {
        let end = (self.count_index + COUNT_BUDGET).min(LIMIT);
        while self.count_index < end {
            self.count += self.is_prime[self.count_index] as u64;
            self.count_index += 1;
        }

        if self.count_index >= LIMIT {
            self.phase = Phase::Done;
        }
    }
}
