//! Native nonblocking cue mailbox. Reuses Echoes' audiod PCM path; one worker
//! and one mixer per window. No device initialization or IPC on the GUI thread.
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use sunlight_audiod::AudioClient;
use sunlight_empires::sound::{Cue, Mixer, Settings};
use sunlight_ipc::{monotonic_millis, process_yield};
struct Shared {
    cues: AtomicU32,
    settings: AtomicU32,
    shutdown: AtomicBool,
    done: AtomicBool,
    reset: AtomicBool,
}
pub struct Audio {
    shared: Arc<Shared>,
    thread: sunlight_libc::thread::JoinHandle,
    pub settings: Settings,
}
impl Audio {
    pub fn start() -> Option<Self> {
        let settings = Settings::default();
        let shared = Arc::new(Shared {
            cues: AtomicU32::new(0),
            settings: AtomicU32::new(pack(settings)),
            shutdown: AtomicBool::new(false),
            done: AtomicBool::new(false),
            reset: AtomicBool::new(false),
        });
        let arg = Arc::into_raw(shared.clone()) as *mut u8;
        let thread = match unsafe { sunlight_libc::thread::spawn(worker, arg) } {
            Ok(thread) => thread,
            Err(_) => {
                unsafe {
                    drop(Arc::from_raw(arg as *const Shared));
                }
                return None;
            }
        };
        Some(Self {
            shared,
            thread,
            settings,
        })
    }
    pub fn cue(&self, cue: Cue) {
        if !self.settings.mute {
            self.shared
                .cues
                .fetch_or(1 << (cue as u8), Ordering::Release);
        }
    }
    pub fn sync_settings(&self) {
        self.shared
            .settings
            .store(pack(self.settings), Ordering::Release);
    }
    pub fn reset(&self) {
        self.shared.cues.store(0, Ordering::Release);
        self.shared.reset.store(true, Ordering::Release);
    }
}
fn pack(s: Settings) -> u32 {
    u32::from(s.master)
        | (u32::from(s.effects) << 8)
        | (u32::from(s.music) << 16)
        | ((s.mute as u32) << 24)
}
fn unpack(v: u32) -> Settings {
    Settings {
        master: v as u8,
        effects: (v >> 8) as u8,
        music: (v >> 16) as u8,
        mute: v >> 24 != 0,
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        let deadline = monotonic_millis().saturating_add(2000);
        while !self.shared.done.load(Ordering::Acquire) && monotonic_millis() < deadline {
            process_yield();
        }
        // NativeThreadReaped (156) is also used by libc's pthread bridge.
        // done alone is insufficient: the trampoline still uses its stack.
        while unsafe { sunlight_libc::sys::syscall1(156, u64::from(self.thread.tid)) } != 1 {
            if monotonic_millis() >= deadline {
                return;
            }
            process_yield();
        }
        {
            let _ = sunlight_libc::mman::munmap(self.thread.stack_base, self.thread.stack_size);
            let _ = sunlight_libc::mman::munmap(self.thread.tls_base, self.thread.tls_size);
        }
    }
}
fn stop(client: &AudioClient, submitted: u64) {
    if submitted != 0
        && client
            .stream_status()
            .is_ok_and(|s| s.submitted_frames == submitted)
    {
        let _ = client.stop_stream();
    }
}
fn worker_wait() {
    let delay = sunlight_libc::time::Timespec {
        tv_sec: 0,
        tv_nsec: 8_000_000,
    };
    unsafe {
        sunlight_libc::time::nanosleep(&delay, core::ptr::null_mut());
    }
}
extern "C" fn worker(arg: *mut u8) -> *mut u8 {
    let shared = unsafe { Arc::from_raw(arg as *const Shared) };
    let client = AudioClient::new();
    let mut mixer = Mixer::default();
    let mut chunk = [0u8; 4096];
    let mut submitted = 0;
    let mut ready = false;
    let mut retry = 0;
    while !shared.shutdown.load(Ordering::Acquire) {
        let settings = unpack(shared.settings.load(Ordering::Acquire));
        if shared.reset.swap(false, Ordering::AcqRel) || (settings.mute && !mixer.settings.mute) {
            mixer.clear();
            stop(&client, submitted);
            submitted = 0;
        }
        mixer.settings = settings;
        let cues = shared.cues.swap(0, Ordering::AcqRel);
        for cue in [
            Cue::Selection,
            Cue::Movement,
            Cue::Started,
            Cue::Completed,
            Cue::Trained,
            Cue::Invalid,
            Cue::Melee,
            Cue::Arrow,
            Cue::Death,
            Cue::Victory,
            Cue::Defeat,
        ] {
            if cues & (1 << (cue as u8)) != 0 {
                mixer.cue(cue);
            }
        }
        let now = monotonic_millis();
        if now < retry {
            worker_wait();
            continue;
        }
        if !mixer.active() {
            // The service grants one badge exclusive ownership until STOP.
            // Release it once our queued/in-flight tail drains, so a silent
            // open Empires window cannot prevent Echoes from playing music.
            if submitted != 0 {
                match client.stream_status() {
                    Ok(status) if status.submitted_frames != submitted => submitted = 0,
                    Ok(status) if status.buffered_frames == 0 => {
                        stop(&client, submitted);
                        submitted = 0;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        ready = false;
                        retry = now + 250;
                    }
                }
            }
            worker_wait();
            continue;
        }
        if !ready {
            ready = client.snapshot().is_ok_and(|s| {
                s.available() && s.sample_rate_hz == 48000 && s.channels == 2 && s.bits == 16
            });
            if !ready {
                mixer.clear();
                retry = now + 1000;
                continue;
            }
        }
        match client.stream_status() {
            Ok(status) => {
                // audio.v1 is global. If another stream is busy, drop our cues
                // rather than resetting or interrupting it. No cross-app mixer.
                if submitted != 0 && status.submitted_frames != submitted {
                    submitted = 0;
                    mixer.clear();
                    continue;
                }
                if submitted == 0 && status.buffered_frames > 0 {
                    mixer.clear();
                    continue;
                }
                if status.buffered_frames >= 2048 {
                    worker_wait();
                    continue;
                }
                mixer.render(&mut chunk);
                match client.submit_pcm_chunk(&chunk) {
                    Ok(s) => submitted = s.submitted_frames,
                    Err(_) => {
                        ready = false;
                        retry = now + 250;
                        mixer.clear();
                    }
                }
            }
            Err(_) => {
                ready = false;
                retry = now + 1000;
                mixer.clear();
            }
        }
    }
    stop(&client, submitted);
    shared.done.store(true, Ordering::Release);
    core::ptr::null_mut()
}
