//! Run with rustc --edition 2021 -O tools/tests/frame_ownership_bench.rs.

#[path = "../../kernel/src/telemetry_frames.rs"]
mod frames;

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = System.alloc(layout);
        if !result.is_null() {
            let size = layout.size().max(16).next_multiple_of(8);
            PEAK.fetch_max(LIVE.fetch_add(size, Relaxed) + size, Relaxed);
            CALLS.fetch_add(1, Relaxed);
            LARGEST.fetch_max(layout.size(), Relaxed);
        }
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size().max(16).next_multiple_of(8), Relaxed);
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: Meter = Meter;

type Row = (usize, usize, usize);
type Sample = ([Row; 3], usize);
const FRAME_LIMIT: usize = 4 * 1024 * 1024;
type Scratch = frames::FrameScratch<{ FRAME_LIMIT / 64 }, { FRAME_LIMIT / 4096 }>;

fn legacy(spaces: &[Vec<usize>; 3], groups: [usize; 3], private: impl Fn(usize) -> bool) -> Sample {
    let mut owner = BTreeMap::new();
    let mut saved = BTreeMap::new();
    for (index, mappings) in spaces.iter().enumerate() {
        let mut frames = Vec::new();
        for &frame in mappings {
            frames.push(frame);
        }
        frames.sort_unstable();
        frames.dedup();
        for &frame in &frames {
            owner.entry(frame as u64).or_insert((0usize, false)).0 += groups[index];
        }
        saved.insert(index, (mappings.len(), frames));
    }
    let mut transfer = 0;
    for (&frame, (count, is_private)) in &mut owner {
        *is_private = private(frame as usize);
        transfer += usize::from(*is_private && *count > 1);
    }
    let rows = std::array::from_fn(|index| {
        let (mapped, frames) = &saved[&index];
        let private = frames
            .iter()
            .filter(|&&frame| owner[&(frame as u64)] == (1, true))
            .count();
        (*mapped, private, frames.len() - private)
    });
    (rows, transfer)
}

fn bounded(
    scratch: &mut Scratch,
    spaces: &[Vec<usize>; 3],
    groups: [usize; 3],
    private: impl Fn(usize) -> bool,
) -> Sample {
    scratch.reset_sample();
    let mut transfer = 0;
    let mut correction_end = 0;
    let mut rows = [Row::default(); 3];
    for (index, mappings) in spaces.iter().enumerate() {
        for &frame in mappings {
            if frame < FRAME_LIMIT {
                let bit = 1 << (frame % 64);
                let (local, shared, moved, earlier) = scratch.classify_word(
                    frame / 64,
                    bit,
                    if private(frame) { bit } else { 0 },
                    groups[index] > 1,
                );
                rows[index].1 += local as usize;
                rows[index].2 += shared as usize;
                transfer += moved as usize;
                if earlier {
                    correction_end = index;
                }
            } else if scratch.first_external_in_space(frame).unwrap() {
                rows[index].2 += 1;
            }
        }
        rows[index].0 = mappings.len();
        scratch.reset_space();
    }
    for index in 0..correction_end {
        let mut private = 0;
        let mut shared = 0;
        for &frame in &spaces[index] {
            if frame < FRAME_LIMIT {
                let (local, global) = scratch.account_word(frame / 64, 1 << (frame % 64));
                private += local as usize;
                shared += global as usize;
            } else if scratch.first_external_in_space(frame).unwrap() {
                shared += 1;
            }
        }
        scratch.reset_space();
        rows[index] = (spaces[index].len(), private, shared);
    }
    (rows, transfer)
}

fn measure(method: &str, mut run: impl FnMut() -> Sample) -> Sample {
    let mut elapsed = [0; 9];
    let mut result = ([Row::default(); 3], 0);
    let baseline = LIVE.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    CALLS.store(0, Relaxed);
    LARGEST.store(0, Relaxed);
    for time in &mut elapsed {
        let start = Instant::now();
        result = std::hint::black_box(run());
        *time = start.elapsed().as_nanos();
    }
    assert_eq!(LIVE.load(Relaxed), baseline);
    elapsed.sort_unstable();
    println!(
        "{{\"method\":\"{method}\",\"median_ns\":{},\"max_ns\":{},\"peak_extra_bytes\":{},\"allocations_per_sample\":{},\"largest_request_bytes\":{}}}",
        elapsed[4], elapsed[8], PEAK.load(Relaxed) - baseline,
        CALLS.load(Relaxed) / elapsed.len(), LARGEST.load(Relaxed),
    );
    result
}

fn main() {
    static SCRATCH: std::sync::Mutex<Scratch> = std::sync::Mutex::new(Scratch::new());
    let mut scratch = SCRATCH.lock().unwrap();
    println!(
        "{{\"static_scratch_bytes\":{}}}",
        std::mem::size_of::<Scratch>()
    );
    for (name, count, mixed) in [
        ("private-small", 4096, false),
        ("alias-cow-external", 32768, true),
        ("private-pressure", 180224, false),
        ("mixed-watermark-scale", 262144, true),
    ] {
        let groups = if mixed { [1, 2, 1] } else { [1; 3] };
        let mut spaces = [(0..count).collect(), Vec::new(), Vec::new()];
        if mixed {
            spaces[0].extend([0, 1, 1, FRAME_LIMIT, FRAME_LIMIT]);
            spaces[1].extend((0..count).step_by(4));
            spaces[2].extend((0..count).step_by(11));
        }
        let private = |frame| frame < FRAME_LIMIT && (!mixed || frame % 7 != 0);
        println!("{{\"workload\":\"{name}\",\"base_frames\":{count}}}");
        let old = measure("legacy", || legacy(&spaces, groups, private));
        let new = measure("bounded", || {
            bounded(&mut scratch, &spaces, groups, private)
        });
        assert_eq!(old, new);
        println!(
            "{{\"matching_rows_mapped_private_shared\":{:?},\"matching_private_transfer_frames\":{}}}",
            old.0.map(|(mapped, private, shared)| [mapped, private, shared]),
            old.1,
        );
    }
    for frame in FRAME_LIMIT..FRAME_LIMIT + 4097 {
        if scratch.first_external_in_space(frame).is_none() {
            scratch.reset_sample();
            assert_eq!(scratch.first_external_in_space(frame), Some(true));
            return;
        }
    }
    panic!("external table should refuse overflow");
}
