//! Actor-private observation state: no shared atomics, locks or heap storage.
//! Counters fail closed on overflow; a failed observation cannot become success.
#[derive(Debug)]
pub struct Counters {
    pub compact: bool,
    measuring: bool,
    finished: bool,
    pub count: u64,
    pub bytes: u64,
    pub full: u64,
    pub overflow: bool,
    pub order_valid: bool,
    pub start_sequence: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub samples: [u64; 64],
    pub samples_len: usize,
    pub dropped: u64,
}
impl Counters {
    pub const fn new() -> Self {
        Self {
            compact: false,
            measuring: false,
            finished: false,
            count: 0,
            bytes: 0,
            full: 0,
            overflow: false,
            order_valid: true,
            start_sequence: 0,
            start_ms: 0,
            end_ms: 0,
            samples: [0; 64],
            samples_len: 0,
            dropped: 0,
        }
    }
    pub fn begin(&mut self, next: u64, measuring: bool, now: u64) {
        assert!(!self.finished && (!self.measuring || !measuring));
        *self = Self {
            compact: true,
            measuring,
            start_sequence: next,
            start_ms: now,
            ..Self::new()
        };
    }
    fn add(value: &mut u64, amount: u64, overflow: &mut bool) {
        if let Some(next) = value.checked_add(amount) {
            *value = next;
        } else {
            *overflow = true;
        }
    }
    pub fn wants_sample(&self) -> bool {
        self.measuring && self.samples_len < 64
    }
    pub fn success(&mut self, sequence: u64, bytes: u64, timestamp: Option<u64>) {
        if !self.measuring {
            return;
        }
        if let Some(now) = timestamp {
            if self.samples_len < 64 {
                self.samples[self.samples_len] = now;
                self.samples_len += 1;
            } else {
                Self::add(&mut self.dropped, 1, &mut self.overflow);
            }
        } else {
            Self::add(&mut self.dropped, 1, &mut self.overflow);
        }
        self.order_valid &= self.start_sequence.checked_add(self.count) == Some(sequence);
        Self::add(&mut self.count, 1, &mut self.overflow);
        Self::add(&mut self.bytes, bytes, &mut self.overflow);
    }
    pub fn full(&mut self) {
        if self.measuring {
            Self::add(&mut self.full, 1, &mut self.overflow);
        }
    }
    pub fn finish(&mut self, now: u64) {
        assert!(self.measuring && !self.finished);
        self.end_ms = now;
        self.measuring = false;
        self.finished = true;
    }
    #[cfg(not(test))]
    pub fn report(&self, generation: u64, next: u64) {
        // Only once per completed actor window, outside the transfer path.
        sunlight_ipc::debug_log(&alloc::format!("[FL-M53-ACTOR] generation={} pid={} start_sequence={} next={} count={} bytes={} full={} overflow={} dropped={} sample_count={} order={} start_ms={} end_ms={} clock=kernel-bsp-ms resolution_ms=10 coherent=0 authentic=0", generation, sunlight_libc::getpid(), self.start_sequence,next,self.count,self.bytes,self.full,u8::from(self.overflow),self.dropped,self.samples_len,u8::from(self.order_valid),self.start_ms,self.end_ms));
        // DebugLog accepts at most 256 bytes. Four timestamps per row remain
        // within that ABI limit even at the supported clock horizon.
        for (chunk, values) in self.samples[..self.samples_len].chunks(4).enumerate() {
            let times = values
                .iter()
                .map(|v| alloc::format!("{}", v))
                .collect::<alloc::vec::Vec<_>>()
                .join(",");
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-M53-SAMPLES] generation={} pid={} first={} times={}",
                generation,
                sunlight_libc::getpid(),
                chunk * 4,
                times
            ));
        }
    }
}
#[cfg(not(test))]
pub fn clock_probe(cpu: u64) {
    super::super::doorbell::pin(cpu);
    let begin = sunlight_ipc::monotonic_millis();
    let mut last = begin;
    let mut regressions = 0u64;
    for _ in 0..256 {
        let now = sunlight_ipc::monotonic_millis();
        regressions += u64::from(now < last);
        last = now;
    }
    sunlight_ipc::debug_log(&alloc::format!("[FL-M53-CLOCK] pid={} cpu={} reads=257 begin_ms={} end_ms={} regressions={} resolution_ms=10 source=kernel-bsp-ms", sunlight_libc::getpid(),cpu,begin,last,regressions));
    assert_eq!(regressions, 0);
}

/// M5.4: one local probe of the opt-in qualified clock while the actor is
/// actually resident on `cpu`. Returns `(cpu_ok, regressions, span_ns,
/// cpu_mismatch)`. Bounded: it waits for at most 4096 yields for migration and
/// never blocks.
#[cfg(not(test))]
fn local_qualified_probe(cpu: u32) -> (bool, u64, u64, u64) {
    use sunlight_ipc::measurement_clock as mc;
    super::super::doorbell::pin(cpu as u64);
    let mut resident = false;
    for _ in 0..256 {
        match sunlight_ipc::measurement_stamp() {
            Some(s) if s.cpu == cpu => {
                resident = true;
                break;
            }
            Some(_) => sunlight_ipc::process_yield(),
            None => break,
        }
    }
    let Some(first) = sunlight_ipc::measurement_stamp() else {
        return (false, 0, 0, 0);
    };
    let mut last = first;
    let mut regressions = 0u64;
    let mut cpu_mismatch = u64::from(first.cpu != cpu);
    for _ in 0..256 {
        let Some(now) = sunlight_ipc::measurement_stamp() else {
            break;
        };
        cpu_mismatch += u64::from(now.cpu != cpu);
        regressions += u64::from(mc::elapsed_ns(&last, &now).is_err());
        last = now;
    }
    let span = last.ns.saturating_sub(first.ns);
    (resident, regressions, span, cpu_mismatch)
}

/// M5.4 clock-qualification, rejection and cross-CPU contract probes.
///
/// Emits `[FL-M54-CLOCK]` per probed CPU, `[FL-M54-XCPU]` for the cross-CPU
/// pair, and `[FL-M54-REJECT]` for the deterministic rejection self-check.
/// When the kernel cannot publish a qualified clock, every line carries
/// `status=unsupported` and the metric stays NOT_QUALIFIED instead of falling
/// back to the coarse timer.
#[cfg(not(test))]
pub fn qualified_clock_probe(restore_cpu: u64) {
    use sunlight_ipc::measurement_clock as mc;
    let pid = sunlight_libc::getpid();
    // Each CPU independently re-validates its own counter frequency against the
    // PIT before any duration is reported. A failed check invalidates the
    // kernel clock, and every later stamp is then refused.
    // Validate once per guest: re-running the bounded re-measurement for every
    // case would advance the calibration epoch mid-window and needlessly add
    // observer effect.
    let already_validated = sunlight_ipc::measurement_stamp_raw()
        .map(|w| (w[5] as u32) & mc::FLAG_FREQUENCY_STABILITY_CHECKED != 0)
        .unwrap_or(false);
    let mut accepted_any = already_validated;
    if !already_validated {
        for cpu in [0u32, 1u32] {
            super::super::doorbell::pin(cpu as u64);
            let sample = sunlight_ipc::measurement_clock_recalibrate();
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-M54-CLOCKSYNC] pid={} cpu={} accepted={} stability_ppm={} flags={}",
                pid,
                cpu,
                u8::from(sample.is_some()),
                sample.map(|w| w[6]).unwrap_or(u32::MAX as u64),
                sample.map(|w| w[5]).unwrap_or(0)
            ));
            accepted_any |= sample.is_some();
        }
    }
    // Re-read the settled calibration (frequency and epoch may have changed
    // during validation) rather than reporting the calibration that produced
    // the sample.
    let validated = if accepted_any {
        sunlight_ipc::measurement_stamp_raw()
    } else {
        None
    };
    match validated {
        None => {
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-M54-CLOCK] pid={} status=unsupported reason=frequency-stability-rejected source=none",
                pid
            ));
        }
        Some(words) => {
            for cpu in [0u32, 1u32] {
                let (resident, regressions, span, mismatch) = local_qualified_probe(cpu);
                let cal = mc::Calibration {
                    cpu,
                    epoch: words[1] as u32,
                    tsc_hz: words[3],
                    origin: 0,
                    mul: 0,
                    uncertainty_ppm: words[4] as u32,
                    flags: words[5] as u32,
                };
                let uncertainty = cal.uncertainty_ns(span);
                // Keep this line inside the 256-byte DebugLog ABI (the kernel
                // truncates silently past it). Clock identity and absolute
                // frequency live on the once-per-boot kernel line; the probe
                // carries only the per-probe facts plus the bound it applied.
                sunlight_ipc::debug_log(&alloc::format!(
                    "[FL-M54-CLOCK] pid={} cpu={} status={} resident={} reads=257 span_ns={} regressions={} cpu_mismatch={} epoch={} unc_ppm={} unc_ns={} flags={} stab_ok={} contract=same-cpu-only",
                    pid,
                    cpu,
                    if resident { "ready" } else { "not-resident" },
                    u8::from(resident),
                    span,
                    regressions,
                    mismatch,
                    words[1],
                    words[4],
                    uncertainty,
                    words[5],
                    u8::from(words[5] as u32 & mc::FLAG_FREQUENCY_STABILITY_CHECKED != 0)
                ));
            }
            // Cross-CPU: a stamp on CPU 1 then a stamp on CPU 0 must be
            // rejected; their raw difference is reported only to show that it
            // is not a duration.
            let mut min_delta = i64::MAX;
            let mut max_delta = i64::MIN;
            let mut pairs = 0u64;
            let mut rejected = 0u64;
            for _ in 0..4 {
                let (ok1, _, _, _) = local_qualified_probe(1);
                let a = sunlight_ipc::measurement_stamp();
                let (ok0, _, _, _) = local_qualified_probe(0);
                let b = sunlight_ipc::measurement_stamp();
                if !(ok1 && ok0) {
                    continue;
                }
                let (Some(a), Some(b)) = (a, b) else { continue };
                // Only a pair whose two stamps actually landed on different
                // CPUs exercises the cross-CPU rejection; the pin can lose the
                // race and leave both stamps on one CPU, which proves nothing.
                if a.cpu == b.cpu {
                    continue;
                }
                pairs += 1;
                let delta = b.ns as i64 - a.ns as i64;
                min_delta = min_delta.min(delta);
                max_delta = max_delta.max(delta);
                if matches!(mc::elapsed_ns(&a, &b), Err(mc::ClockError::CrossCpu)) {
                    rejected += 1;
                }
            }
            if pairs == 0 {
                min_delta = 0;
                max_delta = 0;
            }
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-M54-XCPU] pid={} pairs={} rejected={} min_delta_ns={} max_delta_ns={} contract=cross-cpu-subtraction-rejected",
                pid, pairs, rejected, min_delta, max_delta
            ));
        }
    }
    rejection_self_check(pid);
    resource_snapshot(pid);
    super::super::doorbell::pin(restore_cpu);
}

/// M5.4 Phase D evidence: a bounded, *scoped* resource snapshot.
///
/// Exact per-actor allocator attribution is not available: the allocator
/// exposes only aggregate used/free figures and the host services keep running
/// concurrently. This therefore reports a reconciled scoped delta over the
/// whole system, with the interference limits stated in the record, and it does
/// not claim that a zero bookkeeping delta proves physical reclamation.
#[cfg(not(test))]
fn resource_snapshot(pid: u64) {
    match sunlight_libc::sysinfo() {
        Ok(info) => sunlight_ipc::debug_log(&alloc::format!(
            "[FL-M54-RES] pid={} scope=whole-system total_kb={} used_kb={} swap_used_kb={} swap_compressed_kb={} attribution=scoped-global-delta physical_reclaim=not-proven interference=concurrent-services,ramfs,desktop,dma clock_hz_unit=1ns",
            pid,
            info.total_ram_kb,
            info.used_ram_kb,
            info.swap_used_kb,
            info.swap_compressed_kb
        )),
        Err(_) => sunlight_ipc::debug_log(&alloc::format!(
            "[FL-M54-RES] pid={} scope=unavailable attribution=none physical_reclaim=not-proven",
            pid
        )),
    }
}

/// Deterministic rejection self-check on the exact shared code the kernel uses.
/// `all_rejected=1` only when every illegal measurement is refused.
#[cfg(not(test))]
fn rejection_self_check(pid: u64) {
    use sunlight_ipc::measurement_clock as mc;
    let valid = mc::Calibration {
        cpu: 0,
        epoch: 7,
        tsc_hz: 2_000_000_000,
        origin: 1_000,
        mul: mc::fixed_point_mul(2_000_000_000).unwrap_or(0),
        uncertainty_ppm: 100,
        flags: mc::FLAG_INVARIANT_TSC,
    };
    let mut invalid = valid;
    invalid.mul = 0;
    let backward = valid.to_ns(999).is_err();
    let uncalibrated = invalid.to_ns(10_000).is_err();
    let overflow = mc::Calibration { tsc_hz: 500_000_000, mul: mc::fixed_point_mul(500_000_000).unwrap_or(0), ..valid }
        .to_ns(u64::MAX)
        .is_err();
    let a = mc::Stamp { cpu: 0, epoch: 7, ns: 100, flags: 0 };
    let cross = mc::Stamp { cpu: 1, epoch: 7, ns: 200, flags: 0 };
    let stale = mc::Stamp { cpu: 0, epoch: 8, ns: 200, flags: 0 };
    let regressed = mc::Stamp { cpu: 0, epoch: 7, ns: 50, flags: 0 };
    let cross_cpu = matches!(mc::elapsed_ns(&a, &cross), Err(mc::ClockError::CrossCpu));
    let stale_epoch = matches!(mc::elapsed_ns(&a, &stale), Err(mc::ClockError::StaleEpoch));
    let time_backward = matches!(mc::elapsed_ns(&a, &regressed), Err(mc::ClockError::Backward));
    let incomplete = sunlight_ipc::measurement_stamp().is_none();
    // `unsupported` is reported for classification, not as a rejection: the
    // five illegal-measurement classes below must all be refused regardless of
    // whether the platform can produce a qualified stamp at all.
    let all = backward && uncalibrated && overflow && cross_cpu && stale_epoch && time_backward;
    sunlight_ipc::debug_log(&alloc::format!(
        "[FL-M54-REJECT] pid={} backward={} uncalibrated={} overflow={} cross_cpu={} stale_epoch={} time_backward={} unsupported={} all_rejected={}",
        pid,
        u8::from(backward),
        u8::from(uncalibrated),
        u8::from(overflow),
        u8::from(cross_cpu),
        u8::from(stale_epoch),
        u8::from(time_backward),
        u8::from(incomplete),
        u8::from(all)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_windows_and_exact_counts() {
        let mut p = Counters::new();
        let mut c = Counters::new();
        p.begin(0, false, 10);
        p.success(0, 8, None);
        assert_eq!(p.count, 0);
        p.begin(16, true, 20);
        c.begin(16, true, 20);
        p.success(16, 23, Some(20));
        p.full();
        c.success(16, 23, Some(30));
        p.finish(30);
        c.finish(30);
        assert_eq!((p.count, p.bytes, p.full), (1, 23, 1));
        assert_eq!((c.count, c.bytes, c.full), (1, 23, 0));
        assert!(p.order_valid && c.order_valid && !p.overflow);
    }
    #[test]
    fn overflow_and_order_are_explicit() {
        let mut p = Counters::new();
        p.begin(u64::MAX, true, 0);
        p.count = u64::MAX;
        p.bytes = u64::MAX;
        p.full = u64::MAX;
        p.success(0, 1, None);
        p.full();
        assert!(p.overflow && !p.order_valid);
        assert_eq!(p.count, u64::MAX);
    }
    #[test]
    fn bounded_samples_report_drops() {
        let mut p = Counters::new();
        p.begin(0, true, 0);
        for seq in 0..100 {
            let timestamp = if p.wants_sample() { Some(seq) } else { None };
            p.success(seq, 1, timestamp);
        }
        p.finish(101);
        assert_eq!((p.count, p.samples_len, p.dropped), (100, 64, 36));
        assert!(p.order_valid && !p.overflow);
    }
    #[test]
    #[should_panic]
    fn duplicate_finish_rejected() {
        let mut p = Counters::new();
        p.begin(0, true, 0);
        p.finish(1);
        p.finish(2);
    }
}
