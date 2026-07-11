use std::alloc::{GlobalAlloc, Layout, System};
use std::env;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use golden_fish::parse_html;

struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, old_layout, new_size);
        if !new_ptr.is_null() {
            if new_size >= old_layout.size() {
                record_alloc(new_size - old_layout.size());
            } else {
                LIVE_BYTES.fetch_sub(old_layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record_alloc(size: usize) {
    let live = LIVE_BYTES.fetch_add(size, Ordering::Relaxed) + size;
    let mut peak = PEAK_BYTES.load(Ordering::Relaxed);
    while live > peak {
        match PEAK_BYTES.compare_exchange_weak(peak, live, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => peak = next,
        }
    }
}

fn main() {
    for path in env::args().skip(1) {
        let source = fs::read_to_string(&path).expect("read HTML input");
        let live_before = LIVE_BYTES.load(Ordering::Relaxed);
        let peak_before = PEAK_BYTES.load(Ordering::Relaxed);
        match parse_html(&source) {
            Ok(document) => {
                let stats = document.stats();
                let live_after = LIVE_BYTES.load(Ordering::Relaxed);
                let peak_after = PEAK_BYTES.load(Ordering::Relaxed);
                println!(
                    "{path}: source={} nodes={} depth={} text={} live_delta={} peak_delta={}",
                    source.len(),
                    stats.node_count,
                    stats.max_depth,
                    stats.total_text_bytes,
                    live_after.saturating_sub(live_before),
                    peak_after.saturating_sub(peak_before)
                );
            }
            Err(error) => println!("{path}: error={error}"),
        }
    }
}
