//! Shared selection and directory thumbnail provider. Decoding and content validation run only on the worker.
use alloc::{sync::Arc, vec::Vec};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use sunlight_ui::image::{decode_simg, inspect_image_dimensions, scale_fit, RgbaImage};

pub const INPUT_LIMIT: usize = 4 * 1024 * 1024;
const COMPRESSED_INPUT_LIMIT: usize = 512 * 1024;
pub const PIXEL_LIMIT: u64 = 1_100_000;
// Progressive JPEG coefficient storage is larger than RGBA; keep its budget smaller.
const JPEG_PIXEL_LIMIT: u64 = 128 * 1024;
const PNG_16_PIXEL_LIMIT: u64 = 256 * 1024;
const PNG_INTERLACED_PIXEL_LIMIT: u64 = 512 * 1024;
pub const EDGE: u32 = 128;
const CACHE_BYTES: usize = 1024 * 1024;
const CACHE_ENTRIES: usize = 64;

#[derive(Clone)]
pub struct Preview {
    pub image: Arc<RgbaImage>,
    pub width: u32,
    pub height: u32,
    pub format: u8,
}

fn source_pixel_limit(data: &[u8]) -> u64 {
    if data.starts_with(b"\xff\xd8") {
        return JPEG_PIXEL_LIMIT;
    }
    if data.starts_with(b"\x89PNG") {
        // decode_raw retains native-depth output alongside inflated scanlines.
        // Adam7 additionally uses pass scratch. Keep headroom for a full cache.
        if data.get(24) == Some(&16) {
            return PNG_16_PIXEL_LIMIT;
        }
        if data.get(28) == Some(&1) {
            return PNG_INTERLACED_PIXEL_LIMIT;
        }
    }
    PIXEL_LIMIT
}

fn generate(data: &[u8]) -> Option<Preview> {
    if data.len() > INPUT_LIMIT {
        return None;
    }
    // PNG retains concatenated compressed chunks as well as decoded channels;
    // JPEG retains coefficient storage. Bound encoded intermediates separately.
    if (data.starts_with(b"\x89PNG") || data.starts_with(b"\xff\xd8"))
        && data.len() > COMPRESSED_INPUT_LIMIT
    {
        return None;
    }
    let (width, height) = inspect_image_dimensions(data).ok()?;
    let pixel_limit = source_pixel_limit(data);
    if width == 0
        || height == 0
        || width > 2048
        || height > 2048
        || u64::from(width) * u64::from(height) > pixel_limit
    {
        return None;
    }
    let source = decode_simg(data).ok()?;
    let image = Arc::new(scale_fit(&source, EDGE, EDGE));
    let format = if data.starts_with(b"SIMG") {
        1
    } else if data.starts_with(b"\x89PNG") {
        2
    } else if data.starts_with(b"\xff\xd8") {
        3
    } else {
        0
    };
    Some(Preview {
        image,
        width,
        height,
        format,
    })
}

fn fingerprint(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

struct Cached {
    path: Vec<u8>,
    hash: u64,
    preview: Option<Preview>,
}
#[derive(Default)]
struct Cache {
    entries: Vec<Cached>, // least recently used first
    bytes: usize,
    hits: u64,
    misses: u64,
    peak_bytes: usize,
}
impl Cache {
    fn resolve(&mut self, path: &[u8], data: &[u8]) -> Option<Preview> {
        self.resolve_with(path, data, || generate(data))
    }
    fn resolve_with(&mut self, path: &[u8], data: &[u8], generate_preview: impl FnOnce() -> Option<Preview>) -> Option<Preview> {
        let hash = fingerprint(data);
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.path == path && e.hash == hash)
        {
            self.hits += 1;
            let entry = self.entries.remove(index);
            let result = entry.preview.clone();
            self.entries.push(entry);
            return result;
        }
        self.misses += 1;
        self.remove(path);
        let preview = generate_preview();
        let bytes = preview_bytes(&preview) + path.len() + core::mem::size_of::<Cached>() + 32;
        while !self.entries.is_empty()
            && (self.bytes + bytes + CACHE_ENTRIES * core::mem::size_of::<Cached>() > CACHE_BYTES
                || self.entries.len() >= CACHE_ENTRIES)
        {
            let old = self.entries.remove(0);
            self.bytes -= preview_bytes(&old.preview)
                + old.path.capacity()
                + core::mem::size_of::<Cached>()
                + 32;
        }
        self.bytes += bytes;
        self.peak_bytes = self.peak_bytes.max(self.bytes);
        self.entries.push(Cached {
            path: path.to_vec(),
            hash,
            preview: preview.clone(),
        });
        preview
    }
    fn remove(&mut self, path: &[u8]) {
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            let old = self.entries.remove(i);
            self.bytes -= preview_bytes(&old.preview)
                + old.path.capacity()
                + core::mem::size_of::<Cached>()
                + 32;
        }
    }
}
fn preview_bytes(preview: &Option<Preview>) -> usize {
    preview.as_ref().map_or(0, |p| {
        p.image.pixels.capacity() * 4 + core::mem::size_of::<RgbaImage>() + 32
    })
}

// Leave decoder headroom in Files' existing 16 MiB heap.
pub const GRID_CACHE_BYTES: usize = 4 * 1024 * 1024;
const GRID_CACHE_ENTRIES: usize = 512;
struct GridEntry {
    path: Vec<u8>,
    size: u64,
    preview: Option<Preview>,
}
impl GridEntry {
    fn bytes(&self) -> usize {
        core::mem::size_of::<Self>()
            + self.path.capacity()
            + 32
            + self.preview.as_ref().map_or(0, |p| {
                p.image.pixels.capacity() * 4 + core::mem::size_of::<RgbaImage>() + 32
            })
    }
}
#[derive(Default)]
struct GridCache {
    entries: Vec<GridEntry>,
    bytes: usize,
    peak: usize,
}
impl GridCache {
    fn insert(&mut self, entry: GridEntry) {
        self.remove(&entry.path);
        let cost = entry.bytes();
        // Include the entry-vector allocation separately, conservatively at its cap.
        let limit = GRID_CACHE_BYTES - GRID_CACHE_ENTRIES * core::mem::size_of::<GridEntry>();
        if cost > limit {
            return;
        }
        while !self.entries.is_empty()
            && (self.bytes + cost > limit || self.entries.len() >= GRID_CACHE_ENTRIES)
        {
            self.bytes -= self.entries.remove(0).bytes();
        }
        self.bytes += cost;
        self.peak = self.peak.max(self.bytes);
        self.entries.push(entry);
    }
    fn remove(&mut self, path: &[u8]) {
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            self.bytes -= self.entries.remove(i).bytes();
        }
    }
    fn lookup(&mut self, path: &[u8], size: u64) -> bool {
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            if self.entries[i].size != size {
                self.remove(path);
                return false;
            }
            let entry = self.entries.remove(i);
            self.entries.push(entry);
            return true;
        }
        false
    }
    fn invalidate(&mut self, directory: &[u8]) {
        self.entries.retain(|e| {
            let parent = e.path.iter().rposition(|b| *b == b'/').unwrap_or(0);
            let matches = if parent == 0 {
                directory == b"/"
            } else {
                &e.path[..parent] == directory
            };
            !matches
        });
        self.bytes = self.entries.iter().map(GridEntry::bytes).sum();
    }
}
#[derive(Clone)]
struct GridRequest {
    request: Request,
    size: u64,
}
struct GridResult {
    request: GridRequest,
    preview: Option<Preview>,
}

#[derive(Clone)]
struct Request {
    generation: u64,
    path: Vec<u8>,
}
struct ResultSlot {
    generation: u64,
    preview: Option<Preview>,
}
#[derive(Default)]
struct Slots {
    grid_request: Option<GridRequest>,
    grid_result: Option<GridResult>,
    request: Option<Request>,
    result: Option<ResultSlot>,
}
impl Slots {
    fn next_request(&mut self) -> Option<(Request, Option<GridRequest>)> {
        if let Some(request) = self.request.take() {
            Some((request, None))
        } else {
            self.grid_request
                .take()
                .map(|grid| (grid.request.clone(), Some(grid)))
        }
    }
}
struct Shared {
    locked: AtomicBool,
    slots: UnsafeCell<Slots>,
    generation: AtomicU64,
    directory_generation: AtomicU64,
    wake: AtomicU64, // deferred reply token, held only while worker is parked
    send_cap: u64,
    stop: AtomicBool,
    waiter: AtomicBool,
}
// Only with_slots accesses the cell; its acquire/release flag gives exclusive access.
unsafe impl Sync for Shared {}
impl Shared {
    fn with_slots<T>(&self, f: impl FnOnce(&mut Slots) -> T) -> Option<T> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None; // UI never waits on the worker.
        }
        let result = f(unsafe { &mut *self.slots.get() });
        self.locked.store(false, Ordering::Release);
        Some(result)
    }
    fn wake(&self) {
        let token = self.wake.swap(0, Ordering::AcqRel);
        if token != 0 {
            let _ = sunlight_ipc::ipc_complete_deferred_reply(token, sunlight_ipc::IpcMsg::empty());
        }
    }
}

pub struct Thumbnails {
    shared: Arc<Shared>,
    deferred: Option<Request>,
    pub preview: Option<Preview>,
    pub failed: bool,
    available: bool,
    endpoint: u64,
    grid_cache: GridCache,
    grid_pending: Option<GridRequest>,
}
impl Thumbnails {
    pub fn new() -> Self {
        #[cfg(not(test))]
        let endpoint = sunlight_ipc::endpoint_create().0;
        #[cfg(test)]
        let endpoint = 0;
        #[cfg(not(test))]
        let send_cap = sunlight_ipc::endpoint_bind(endpoint).0;
        #[cfg(test)]
        let send_cap = 0;
        let shared = Arc::new(Shared {
            locked: AtomicBool::new(false),
            slots: UnsafeCell::new(Slots::default()),
            generation: AtomicU64::new(0),
            directory_generation: AtomicU64::new(0),
            wake: AtomicU64::new(0),
            send_cap,
            stop: AtomicBool::new(false),
            waiter: AtomicBool::new(false),
        });
        #[cfg(not(test))]
        let available = {
            let ptr = Arc::into_raw(shared.clone());
            // Worker owns this Arc until it exits; no pointer into the app model escapes.
            if endpoint != 0
                && send_cap != 0
                && unsafe { sunlight_libc::thread::spawn(worker, ptr as *mut u8) }.is_ok()
            {
                true
            } else {
                unsafe {
                    drop(Arc::from_raw(ptr));
                }
                false
            }
        };
        #[cfg(test)]
        let available = false;
        Self {
            shared,
            deferred: None,
            preview: None,
            failed: false,
            available,
            endpoint,
            grid_cache: GridCache::default(),
            grid_pending: None,
        }
    }
    pub fn directory_changed(&mut self) {
        self.shared
            .directory_generation
            .fetch_add(1, Ordering::AcqRel);
        self.grid_pending = None;
        self.shared.with_slots(|s| {
            s.grid_request = None;
            s.grid_result = None;
        });
    }
    pub fn invalidate_directory(&mut self, directory: &[u8]) {
        self.directory_changed();
        self.grid_cache.invalidate(directory);
    }
    #[cfg(test)]
    pub fn seed_grid(&mut self, path: &[u8], size: u64, preview: Preview) {
        self.grid_cache.insert(GridEntry {
            path: path.to_vec(),
            size,
            preview: Some(preview),
        });
    }
    pub fn grid_image(&self, path: &[u8], size: u64) -> Option<&Preview> {
        self.grid_cache
            .entries
            .iter()
            .find(|e| e.path == path && e.size == size)
            .and_then(|e| e.preview.as_ref())
    }
    // Called in viewport order. Only one grid job may be pending; Preview has priority.
    pub fn request_grid(&mut self, path: &[u8], size: u64) {
        if path.len() > 256
            || size == 0
            || size > INPUT_LIMIT as u64
            || self.grid_cache.lookup(path, size)
            || self.grid_pending.is_some()
            || !self.available
        {
            return;
        }
        let request = GridRequest {
            request: Request {
                generation: self.shared.directory_generation.load(Ordering::Acquire),
                path: path.to_vec(),
            },
            size,
        };
        if self
            .shared
            .with_slots(|s| s.grid_request = Some(request.clone()))
            .is_some()
        {
            self.grid_pending = Some(request);
            self.shared.wake();
        }
    }
    pub fn request(&mut self, path: Option<&[u8]>) {
        let generation = self
            .shared
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.preview = None;
        self.failed = path.is_some() && !self.available;
        self.deferred = path.filter(|_| self.available).map(|p| Request {
            generation,
            path: p.to_vec(),
        });
        self.poll();
    }
    pub fn poll(&mut self) -> bool {
        if self.shared.stop.load(Ordering::Acquire) {
            self.available = false;
            self.failed |= self.deferred.is_some();
            self.deferred = None;
            self.grid_pending = None;
        }
        #[cfg(not(test))]
        if self.available
            && sunlight_ipc::ipc_try_recv(sunlight_ipc::EndpointId(self.endpoint)).is_some()
        {
            match sunlight_ipc::ipc_defer_reply() {
                Ok(token) => self.shared.wake.store(token, Ordering::Release),
                Err(_) => {
                    // Do not strand a worker if the kernel cannot retain its call.
                    self.shared.stop.store(true, Ordering::Release);
                    self.available = false;
                    self.failed = true;
                    self.deferred = None;
                    sunlight_ipc::ipc_reply(sunlight_ipc::IpcMsg::empty());
                }
            }
        }
        let generation = self.shared.generation.load(Ordering::Acquire);
        let mut changed = false;
        let mut submitted = false;
        let accessed = self
            .shared
            .with_slots(|slots| {
                if let Some(request) = self.deferred.take() {
                    slots.request = Some(request);
                    submitted = true;
                }
                submitted |= slots.request.is_some() || slots.grid_request.is_some();
                if let Some(result) = slots.grid_result.take() {
                    if result.request.request.generation
                        == self.shared.directory_generation.load(Ordering::Acquire)
                    {
                        self.grid_pending = None;
                        self.grid_cache.insert(GridEntry {
                            path: result.request.request.path,
                            size: result.request.size,
                            preview: result.preview,
                        });
                        #[cfg(all(not(test), debug_assertions))]
                        sunlight_ipc::debug_log(&alloc::format!(
                            "[FILES-GRID] cache={} peak={} entries={}\n",
                            self.grid_cache.bytes,
                            self.grid_cache.peak,
                            self.grid_cache.entries.len()
                        ));
                        changed = true;
                    }
                }
                if let Some(result) = slots.result.take() {
                    if result.generation == generation {
                        self.failed = result.preview.is_none();
                        self.preview = result.preview;
                        changed = true;
                    }
                }
            })
            .is_some();
        // Also resolves worker startup and lock-contention wake races. This is
        // piggybacked on normal UI events, never a new timer or runnable loop.
        if accessed && (submitted || self.shared.waiter.load(Ordering::Acquire)) {
            self.shared.wake();
        }
        changed
    }
}
impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake();
        #[cfg(not(test))]
        if self.endpoint != 0 {
            sunlight_ipc::endpoint_destroy(sunlight_ipc::EndpointId(self.endpoint));
        }
    }
}

#[cfg(not(test))]
extern "C" fn worker(arg: *mut u8) -> *mut u8 {
    use sunlight_ipc::{ipc_call, monotonic_millis, CapabilityToken, IpcMsg};
    let shared = unsafe { Arc::from_raw(arg as *const Shared) };
    let mut cache = Cache::default();
    let mut completed = None;
    let mut grid_completed = None;
    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        shared.waiter.store(true, Ordering::Release);
        let request = shared
            .with_slots(|slots| {
                shared.waiter.store(false, Ordering::Release);
                if let Some(result) = completed.take() {
                    slots.result = Some(result);
                }
                if let Some(result) = grid_completed.take() {
                    slots.grid_result = Some(result);
                }
                slots.next_request()
            })
            .flatten();
        if let Some((request, grid)) = request {
            let generation = if grid.is_some() {
                &shared.directory_generation
            } else {
                &shared.generation
            };
            if request.generation != generation.load(Ordering::Acquire) {
                continue;
            }
            let started = monotonic_millis();
            let preview = match read_source(&request.path) {
                Some(data)
                    if request.generation == generation.load(Ordering::Acquire)
                        && grid.as_ref().is_none_or(|g| g.size == data.len() as u64) =>
                {
                    if crate::preview::pdf::supports(&request.path) {
                        cache.resolve_with(&request.path, &data, || {
                            let image = crate::preview::pdf::worker_preview(&request.path, EDGE)?;
                            Some(Preview { image: Arc::new(image), width: EDGE, height: EDGE, format: 5 })
                        })
                    } else {
                        cache.resolve(&request.path, &data)
                    }
                }
                _ => {
                    cache.remove(&request.path);
                    None
                }
            };
            let elapsed = monotonic_millis().saturating_sub(started);
            // Debug instrumentation is compiled out of release builds.
            #[cfg(debug_assertions)]
            sunlight_ipc::debug_log(&alloc::format!(
                "[FILES-THUMB] ms={} hits={} misses={} cache={} peak={} active=1 queued<=1\n",
                elapsed,
                cache.hits,
                cache.misses,
                cache.bytes,
                cache.peak_bytes
            ));
            let _ = elapsed;
            if let Some(grid) = grid {
                grid_completed = Some(GridResult {
                    request: grid,
                    preview,
                });
            } else {
                completed = Some(ResultSlot {
                    generation: request.generation,
                    preview,
                });
            }
            continue;
        }
        let _ = ipc_call(CapabilityToken(shared.send_cap), IpcMsg::empty());
    }
    core::ptr::null_mut()
}

#[cfg(not(test))]
fn read_source(path: &[u8]) -> Option<Vec<u8>> {
    use sunlight_libc as libc;
    let before = libc::stat(path).ok()?;
    if before.file_type != libc::FT_FILE || before.size == 0 || before.size > INPUT_LIMIT as u64 {
        return None;
    }
    let fd = libc::open(path).ok()?;
    let result = (|| {
        let mut data = Vec::new();
        data.try_reserve_exact(before.size as usize).ok()?;
        data.resize(before.size as usize, 0);
        let mut total = 0;
        while total < data.len() {
            let end = (total + 8192).min(data.len());
            let n = libc::read(fd, &mut data[total..end]).ok()?;
            if n == 0 {
                return None;
            }
            total += n;
        }
        let mut extra = [0];
        if libc::read(fd, &mut extra).ok()? != 0 {
            return None;
        }
        let after = libc::stat(path).ok()?;
        if after.size != before.size || after.file_type != before.file_type {
            return None;
        }
        Some(data)
    })();
    let _ = libc::close(fd);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn tga(w: u16, h: u16, alpha: u8) -> Vec<u8> {
        let mut bytes = vec![0; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&w.to_le_bytes());
        bytes[14..16].copy_from_slice(&h.to_le_bytes());
        bytes[16] = 32;
        bytes[17] = 0x28;
        for _ in 0..u32::from(w) * u32::from(h) {
            bytes.extend_from_slice(&[30, 60, 90, alpha]);
        }
        bytes
    }
    #[test]
    fn png_scratch_budgets_leave_cache_headroom() {
        let mut png = include_bytes!("../../sun-img/tests/fixtures/rgba.png").to_vec();
        assert_eq!(source_pixel_limit(&png), PIXEL_LIMIT);
        png[28] = 1;
        assert_eq!(source_pixel_limit(&png), PNG_INTERLACED_PIXEL_LIMIT);
        png[24] = 16;
        assert_eq!(source_pixel_limit(&png), PNG_16_PIXEL_LIMIT);
        // Above both the 16-bit and Adam7 budgets; reject before decoder allocation.
        png[16..20].copy_from_slice(&1024u32.to_be_bytes());
        png[20..24].copy_from_slice(&1024u32.to_be_bytes());
        assert!(generate(&png).is_none());
    }

    #[test]
    fn preview_priority_keeps_bounded_grid_request() {
        let mut t = Thumbnails::new();
        t.available = true;
        t.request_grid(b"/grid.png", 100);
        t.request(Some(b"/preview.png"));
        t.shared.with_slots(|slots| {
            let (request, grid) = slots.next_request().unwrap();
            assert_eq!(request.path, b"/preview.png");
            assert!(grid.is_none());
            let (request, grid) = slots.next_request().unwrap();
            assert_eq!(request.path, b"/grid.png");
            assert!(grid.is_some());
            assert!(slots.next_request().is_none());
        });
    }
    #[test]
    fn grid_lru_budget_and_negative_entries() {
        let mut cache = GridCache::default();
        let data = tga(128, 128, 255);
        for i in 0..1000 {
            let path = alloc::format!("/pictures/{i}").into_bytes();
            cache.insert(GridEntry {
                path,
                size: data.len() as u64,
                preview: if i % 3 == 0 { None } else { generate(&data) },
            });
            assert!(
                cache.bytes + cache.entries.capacity() * core::mem::size_of::<GridEntry>()
                    <= GRID_CACHE_BYTES
            );
            assert!(cache.entries.len() <= GRID_CACHE_ENTRIES);
        }
        assert!(!cache.lookup(b"/pictures/0", data.len() as u64));
        assert!(cache.lookup(b"/pictures/999", data.len() as u64)); // negative hit
        let oldest = cache.entries[0].path.clone();
        assert!(cache.lookup(&oldest, data.len() as u64));
        assert_eq!(cache.entries.last().unwrap().path, oldest);
        std::println!(
            "grid mixed=1000 retained={} peak={} entries={} vector_capacity={}",
            cache.bytes,
            cache.peak,
            cache.entries.len(),
            cache.entries.capacity()
        );
    }
    #[test]
    fn grid_refresh_metadata_and_return_visit() {
        let mut cache = GridCache::default();
        for path in [b"/a/image".as_slice(), b"/b/image"] {
            cache.insert(GridEntry {
                path: path.to_vec(),
                size: 10,
                preview: generate(&tga(1, 1, 255)),
            });
        }
        assert!(cache.lookup(b"/a/image", 10));
        let first = cache
            .entries
            .last()
            .unwrap()
            .preview
            .as_ref()
            .unwrap()
            .image
            .clone();
        assert!(cache.lookup(b"/a/image", 10));
        assert!(Arc::ptr_eq(
            &first,
            &cache
                .entries
                .last()
                .unwrap()
                .preview
                .as_ref()
                .unwrap()
                .image
        ));
        assert!(!cache.lookup(b"/a/image", 11));
        cache.insert(GridEntry {
            path: b"/a/image".to_vec(),
            size: 10,
            preview: None,
        });
        cache.invalidate(b"/a"); // same-size edits/deletion/rename invalidate together
        assert!(!cache.lookup(b"/a/image", 10));
        assert!(cache.lookup(b"/b/image", 10));
    }
    #[test]
    fn grid_duplicates_navigation_and_stale_completions() {
        let mut t = Thumbnails::new();
        t.available = true;
        t.request_grid(b"/old/a", 100);
        t.request_grid(b"/old/a", 100);
        t.request_grid(b"/old/b", 100);
        let old = t
            .shared
            .with_slots(|s| s.grid_request.take())
            .flatten()
            .unwrap();
        assert_eq!(old.request.path, b"/old/a");
        t.directory_changed();
        t.request_grid(b"/new/a", 100);
        let new = t
            .shared
            .with_slots(|s| s.grid_request.take())
            .flatten()
            .unwrap();
        t.shared.with_slots(|s| {
            s.grid_result = Some(GridResult {
                request: old,
                preview: generate(&tga(1, 1, 255)),
            })
        });
        assert!(!t.poll());
        assert!(t.grid_cache.entries.is_empty());
        assert_eq!(t.grid_pending.as_ref().unwrap().request.path, b"/new/a");
        t.shared.with_slots(|s| {
            s.grid_result = Some(GridResult {
                request: new,
                preview: None,
            })
        });
        assert!(t.poll());
        assert!(t.grid_pending.is_none());
        t.request_grid(b"/new/a", 100); // corrupted/unreadable result is cached
        assert!(t.grid_pending.is_none());
        t.directory_changed();
        t.request_grid(b"/new/a", 100); // revisit reuses negative cache
        assert!(t.grid_pending.is_none());
        t.invalidate_directory(b"/new");
        t.request_grid(b"/new/a", 100);
        assert!(t.grid_pending.is_some());
    }

    #[test]
    fn supported_fixtures_and_alpha() {
        for bytes in [
            include_bytes!("../../sun-img/tests/fixtures/rgba.png").as_slice(),
            include_bytes!("../../sun-img/tests/fixtures/palette.png").as_slice(),
            include_bytes!("../../sun-img/tests/fixtures/adam7.png").as_slice(),
            include_bytes!("../../sun-img/tests/fixtures/baseline.jpg").as_slice(),
            include_bytes!("../../sun-img/tests/fixtures/progressive.jpg").as_slice(),
        ] {
            assert!(generate(bytes).is_some());
        }
        let p = generate(&tga(256, 128, 127)).unwrap();
        assert_eq!((p.image.width, p.image.height), (128, 64));
        assert_eq!(p.image.pixels[0] >> 24, 127);
        let bytes = sun_img_for_test_v2();
        assert_eq!(generate(&bytes).unwrap().format, 1);
    }
    fn sun_img_for_test_v2() -> Vec<u8> {
        // A real existing SIMG asset exercises the shared strict parser.
        include_bytes!("../../docs/images/simg_v2_proof/01_solar_blossom.simg").to_vec()
    }
    #[test]
    fn invalid_unsupported_and_resource_limits() {
        for data in [
            b"".as_slice(),
            b"BMbad",
            b"GIF89a",
            b"SIMG",
            b"\xff\xd8\xffbad",
        ] {
            assert!(generate(data).is_none());
        }
        let mut bad = tga(2, 2, 255);
        bad.pop();
        assert!(generate(&bad).is_none());
        assert!(generate(&vec![0; INPUT_LIMIT + 1]).is_none());
        let mut huge = tga(1, 1, 255);
        huge[12..14].copy_from_slice(&2049u16.to_le_bytes());
        assert!(generate(&huge).is_none());
        assert!(generate(&tga(1200, 1000, 255)).is_none());
    }
    #[test]
    fn reuse_modification_deletion_and_failure_cache() {
        let mut cache = Cache::default();
        let mut data = tga(2, 1, 255);
        let first = cache.resolve(b"/a", &data).unwrap();
        let again = cache.resolve(b"/a", &data).unwrap();
        assert!(Arc::ptr_eq(&first.image, &again.image));
        data[18] ^= 15;
        let modified = cache.resolve(b"/a", &data).unwrap();
        assert_ne!(first.image.pixels, modified.image.pixels);
        cache.remove(b"/a");
        assert_eq!(cache.bytes, 0);
        assert!(cache.resolve(b"/bad", b"broken").is_none());
        assert!(cache.resolve(b"/bad", b"broken").is_none());
        assert_eq!((cache.hits, cache.misses), (2, 3));
    }
    #[test]
    fn bounded_lru_under_mixed_directory_scenarios() {
        for count in [10, 100, 500, 1000] {
            let mut cache = Cache::default();
            let data = tga(128, 128, 255);
            let start = std::time::Instant::now();
            for i in 0..count {
                let path = alloc::format!("/image-{i}");
                cache.resolve(
                    path.as_bytes(),
                    if i % 3 == 0 { b"unsupported" } else { &data },
                );
                assert!(cache.bytes <= CACHE_BYTES);
                assert!(cache.entries.len() <= CACHE_ENTRIES);
            }
            let last = alloc::format!("/image-{}", count - 1);
            cache.resolve(
                last.as_bytes(),
                if (count - 1) % 3 == 0 {
                    b"unsupported"
                } else {
                    &data
                },
            );
            assert_eq!(cache.hits, 1);
            if count >= 100 {
                assert!(!cache.entries.iter().any(|e| e.path == b"/image-0"));
            }
            std::println!(
                "scenario={count} elapsed_us={} hits={} misses={} peak_cache_bytes={}",
                start.elapsed().as_micros(),
                cache.hits,
                cache.misses,
                cache.peak_bytes
            );
        }
    }
    #[test]
    fn obsolete_results_empty_selection_and_latest_request() {
        let mut thumbnails = Thumbnails::new();
        thumbnails.available = true;
        thumbnails.request(Some(b"/old"));
        thumbnails.request(Some(b"/new"));
        let queued = thumbnails
            .shared
            .with_slots(|s| s.request.take())
            .flatten()
            .unwrap();
        assert_eq!(queued.path, b"/new");
        assert_eq!(queued.generation, 2);
        thumbnails.shared.with_slots(|s| {
            s.result = Some(ResultSlot {
                generation: 1,
                preview: generate(&tga(1, 1, 255)),
            })
        });
        assert!(!thumbnails.poll());
        assert!(thumbnails.preview.is_none());
        thumbnails.shared.with_slots(|s| {
            s.result = Some(ResultSlot {
                generation: 2,
                preview: None,
            })
        });
        assert!(thumbnails.poll());
        assert!(thumbnails.failed);
        thumbnails.request(None);
        assert!(!thumbnails.failed);
        assert!(thumbnails.preview.is_none());
    }
    #[test]
    fn ui_defers_without_waiting_for_worker_lock() {
        let mut thumbnails = Thumbnails::new();
        thumbnails.available = true;
        thumbnails.shared.locked.store(true, Ordering::Relaxed);
        thumbnails.request(Some(b"/a"));
        thumbnails.request(Some(b"/b"));
        assert_eq!(thumbnails.deferred.as_ref().unwrap().path, b"/b");
        thumbnails.shared.locked.store(false, Ordering::Release);
        thumbnails.poll();
        assert!(thumbnails.deferred.is_none());
    }
}
