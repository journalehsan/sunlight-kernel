//! Native PCM producer for the game's procedural soundtrack.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};

use sunlight_audiod::{AudioClient, AudioClientError};
use sunlight_ipc::{monotonic_millis, process_yield};
use sunlight_silicon_echoes::music;

const CHUNK_FRAMES: u64 = 1024;
const TARGET_BUFFERED_FRAMES: u32 = 5 * CHUNK_FRAMES as u32;

struct Shared {
    active: AtomicBool,
    shutdown: AtomicBool,
    done: AtomicBool,
}

pub struct Soundtrack {
    shared: Option<Box<Shared>>,
    _worker: sunlight_libc::thread::JoinHandle,
}

impl Soundtrack {
    pub fn start() -> Option<Self> {
        let shared = Box::new(Shared {
            active: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            done: AtomicBool::new(false),
        });
        let pointer = (&*shared as *const Shared).cast_mut().cast::<u8>();
        let worker = unsafe { sunlight_libc::thread::spawn(worker_entry, pointer) }.ok()?;
        Some(Self {
            shared: Some(shared),
            _worker: worker,
        })
    }

    pub fn set_active(&self, active: bool) {
        if let Some(shared) = &self.shared {
            shared.active.store(active, Ordering::Release);
        }
    }
}

impl Drop for Soundtrack {
    fn drop(&mut self) {
        let Some(shared) = &self.shared else { return };
        shared.shutdown.store(true, Ordering::Release);
        let deadline = monotonic_millis().saturating_add(2_000);
        while !shared.done.load(Ordering::Acquire) && monotonic_millis() < deadline {
            process_yield();
        }
        if !shared.done.load(Ordering::Acquire) {
            // The thread could still access the allocation after an IPC stall.
            if let Some(shared) = self.shared.take() {
                let _ = Box::leak(shared);
            }
        }
    }
}

extern "C" fn worker_entry(arg: *mut u8) -> *mut u8 {
    let shared = unsafe { &*(arg.cast::<Shared>()) };
    run(shared);
    shared.done.store(true, Ordering::Release);
    core::ptr::null_mut()
}

fn wait_until(shared: &Shared, delay_ms: u64) {
    let deadline = monotonic_millis().saturating_add(delay_ms);
    while monotonic_millis() < deadline && !shared.shutdown.load(Ordering::Acquire) {
        if !shared.active.load(Ordering::Acquire) {
            break;
        }
        process_yield();
    }
}

fn stop_our_stream(client: &AudioClient, submitted_frames: u64) {
    // STOP is global in audio.v1. Avoid clearing another application's stream
    // if it took ownership while this window was losing focus.
    if submitted_frames != 0
        && client
            .stream_status()
            .map(|status| status.submitted_frames == submitted_frames)
            .unwrap_or(false)
    {
        let _ = client.stop_stream();
    }
}

fn run(shared: &Shared) {
    let client = AudioClient::new();
    let mut chunk = [0u8; CHUNK_FRAMES as usize * 4];
    let mut frame = 0u64;
    let mut submitted = 0u64;
    let mut ready = false;
    while !shared.shutdown.load(Ordering::Acquire) {
        if !shared.active.load(Ordering::Acquire) {
            stop_our_stream(&client, submitted);
            submitted = 0;
            frame = 0;
            while !shared.active.load(Ordering::Acquire) && !shared.shutdown.load(Ordering::Acquire)
            {
                process_yield();
            }
            continue;
        }
        if !ready {
            ready = client
                .snapshot()
                .map(|snapshot| {
                    snapshot.available()
                        && snapshot.sample_rate_hz == music::SAMPLE_RATE
                        && snapshot.channels == 2
                        && snapshot.bits == 16
                })
                .unwrap_or(false);
            if !ready {
                wait_until(shared, 1_000);
                continue;
            }
        }
        let status = match client.stream_status() {
            Ok(status) => status,
            Err(_) => {
                ready = false;
                wait_until(shared, 250);
                continue;
            }
        };
        if submitted != 0 && status.submitted_frames != submitted {
            // Another client reset the single application stream.
            submitted = 0;
            frame = 0;
        }
        if status.buffered_frames >= TARGET_BUFFERED_FRAMES {
            wait_until(shared, 8);
            continue;
        }
        music::render(&mut chunk, frame);
        match client.submit_pcm_chunk(&chunk) {
            Ok(status) => {
                submitted = status.submitted_frames;
                frame = (frame + CHUNK_FRAMES) % music::LOOP_FRAMES;
            }
            Err(AudioClientError::Overflow) => wait_until(shared, 100),
            Err(_) => {
                ready = false;
                wait_until(shared, 250);
            }
        }
    }
    stop_our_stream(&client, submitted);
}
