//! Host diagnostic for the alloc-only configuration, not native measurements.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static FAIL_NEXT: AtomicBool = AtomicBool::new(false);

struct Tracking;
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_NEXT.swap(false, Ordering::Relaxed) {
            return std::ptr::null_mut();
        }
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

#[test]
fn alloc_only_lifecycle_and_snapshot_allocation_failure() {
    use sunlight_hayro_native_probe::{open, Error};
    let sample = include_bytes!("../../../assets/documents/SunlightOS_PDF_Reader_Test.pdf");
    // This is the only test in its own test executable. No parallel allocators.
    FAIL_NEXT.store(true, Ordering::Relaxed);
    assert!(matches!(open(sample), Err(Error::Allocation)));
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let start = Instant::now();
    let mut operations = 0;
    for _ in 0..21 {
        let pdf = open(sample).unwrap();
        assert_eq!(pdf.pages().len(), 4);
        for page in pdf.pages().iter() {
            let mut iter = page.typed_operations();
            while iter.next().is_some() {
                operations += 1;
            }
        }
        drop(pdf);
        #[cfg(feature = "raster")]
        drop(sunlight_hayro_native_probe::vector_fixture().unwrap());
        assert_eq!(LIVE.load(Ordering::Relaxed), baseline);
    }
    let elapsed = start.elapsed();
    let peak = PEAK.load(Ordering::Relaxed);
    println!("cycles=21 operations={operations} elapsed_ms={:.3} baseline_requested={baseline} peak_requested={peak} incremental_peak={} final_requested={}",
        elapsed.as_secs_f64() * 1000.0, peak - baseline, LIVE.load(Ordering::Relaxed));
}
