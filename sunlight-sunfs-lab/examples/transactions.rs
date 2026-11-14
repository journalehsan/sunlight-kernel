//! Diagnostic hosted measurement; timing and allocator measurements are not gates.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
use sunlight_sunfs::{bootstrap::Bootstrap, engine::Filesystem, geometry::*};
use sunlight_sunfs_check::{check, Bytes, Status};
use sunlight_sunfs_lab::simulator::Simulator;
struct Count;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let n = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            let live = if n >= l.size() {
                LIVE.fetch_add(n - l.size(), Ordering::Relaxed) + n - l.size()
            } else {
                LIVE.fetch_sub(l.size() - n, Ordering::Relaxed) - (l.size() - n)
            };
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        q
    }
}
#[global_allocator]
static ALLOC: Count = Count;
fn main() {
    for bps in [64, 256, 512] {
        let capacity = 16 * 1024 * 1024;
        let g = Geometry::calculate(Parameters::laboratory(capacity, bps)).unwrap();
        let mut s = Simulator::new(capacity as usize, 0).unwrap();
        Bootstrap::plan(g, [0x32; 16])
            .unwrap()
            .publish(&mut s)
            .unwrap();
        let mut fs = Filesystem::mount(s).unwrap();
        let id = fs.create(b"hello.txt").unwrap();
        fs.write(&id, 0, b"Hello SunlightOS\n").unwrap();
        for payload in [16usize, 8193] {
            if payload > 16 {
                fs.write(&id, 0, &vec![0x41; payload]).unwrap();
            }
            let plan = fs.admission().unwrap();
            let first = fs.device().trace.last().unwrap().id + 1;
            let before = LIVE.load(Ordering::Relaxed);
            PEAK.store(before, Ordering::Relaxed);
            let start = Instant::now();
            fs.fsync(&id).unwrap();
            let commit_ns = start.elapsed().as_nanos();
            let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
            let events: Vec<_> = fs.device().trace.iter().filter(|e| e.id >= first).collect();
            let bytes_written: usize = events
                .iter()
                .filter(|e| e.operation == "write")
                .map(|e| e.length)
                .sum();
            let flushes = events.iter().filter(|e| e.operation == "flush").count();
            let publication_count = events
                .iter()
                .filter(|e| {
                    e.operation == "write" && (8192..=16384).contains(&e.offset) && e.digest != 0
                })
                .count();
            let start = Instant::now();
            let report = check(&mut Bytes(fs.device().durable()));
            let checker_ns = start.elapsed().as_nanos();
            assert_eq!(report.status, Status::Clean);
            let accounted = fs.accounting();
            println!("{{\"segment_blocks\":{bps},\"payload_bytes\":{payload},\"bytes_written\":{bytes_written},\"data_bytes_written\":{},\"metadata_bytes_written_including_cp_retirement\":{},\"metadata_write_amplification\":{},\"checkpoint_commits\":{publication_count},\"flush_count\":{flushes},\"commit_ns\":{commit_ns},\"checker_ns\":{checker_ns},\"checker_pages_read\":{},\"dependency_blocks\":{},\"reserved_segment_blocks\":{},\"placement_overhead_blocks\":{},\"peak_incremental_heap_bytes_including_simulator\":{peak},\"protected_union_blocks\":{}}}",plan.data_blocks*4096,bytes_written as u64-plan.data_blocks*4096,(bytes_written as f64-plan.data_blocks as f64*4096.0)/payload as f64,report.pages_read,plan.dependency_blocks,plan.reserved_segment_blocks,plan.placement_overhead_blocks,accounted.protected_blocks);
        }
        let mut s = fs.into_device();
        s.power_loss();
        s.reset().unwrap();
        let start = Instant::now();
        let recovered = Filesystem::mount(s).unwrap();
        let recovery_ns = start.elapsed().as_nanos();
        println!("{{\"segment_blocks\":{bps},\"engine_recovery_ns\":{recovery_ns},\"selected_generation\":{}}}",recovered.generation());
    }
}
