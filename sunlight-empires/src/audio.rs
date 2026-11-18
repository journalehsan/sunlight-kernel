//! Native nonblocking cue mailbox. Reuses Echoes' audiod PCM path; one worker
//! and one mixer per window. No device initialization or IPC on the GUI thread.
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use sunlight_audiod::AudioClient;
use sunlight_empires::music::{self, Track};
use sunlight_empires::sound::{Cue, Mixer, Settings};
use sunlight_ipc::{monotonic_millis, process_yield};
struct Shared {
    cues: AtomicU32,
    settings: AtomicU32,
    shutdown: AtomicBool,
    done: AtomicBool,
    reset: AtomicBool,
    track: AtomicU32,
    paused: AtomicBool,
}
pub struct Audio {
    shared: Arc<Shared>,
    thread: sunlight_libc::thread::JoinHandle,
    pub settings: Settings,
}
impl Audio {
    pub fn start(settings: Settings) -> Option<Self> {
        let shared = Arc::new(Shared {
            cues: AtomicU32::new(0),
            settings: AtomicU32::new(pack(settings)),
            shutdown: AtomicBool::new(false),
            done: AtomicBool::new(false),
            reset: AtomicBool::new(false),
            track: AtomicU32::new(Track::Menu as u32),
            paused: AtomicBool::new(false),
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
    pub fn music(&self, track: Track, paused: bool) {
        self.shared.track.store(track as u32, Ordering::Release);
        self.shared.paused.store(paused, Ordering::Release);
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
// Only the audio worker opens/reads resources. At most one descriptor and one
// 4096-byte music chunk exist, independent of track duration.
struct MusicFile {
    fd: sunlight_libc::Fd,
    remaining: u32,
    length: u32,
}
impl MusicFile {
    fn open(track: Track) -> Option<Self> {
        let path = alloc::format!(
            "/Applications/SunlightEmpires.sunapp/Resources/music/{}",
            track.filename()?
        );
        let fd = sunlight_libc::open(path.as_bytes()).ok()?;
        let mut header = [0; 44];
        let result = (|| {
            read_exact(fd, &mut header)?;
            let length = music::pcm_bytes(&header)?;
            Some(Self {
                fd,
                remaining: length,
                length,
            })
        })();
        if result.is_none() {
            let _ = sunlight_libc::close(fd);
        }
        result
    }
    fn render(&mut self, out: &mut [u8]) -> Option<()> {
        let mut offset = 0;
        while offset < out.len() {
            if self.remaining == 0 {
                sunlight_libc::lseek(self.fd, 44, 0).ok()?;
                self.remaining = self.length;
            }
            let count = (out.len() - offset).min(self.remaining as usize);
            read_exact(self.fd, &mut out[offset..offset + count])?;
            self.remaining -= count as u32;
            offset += count;
        }
        Some(())
    }
}
fn read_exact(fd: sunlight_libc::Fd, mut out: &mut [u8]) -> Option<()> {
    while !out.is_empty() {
        let n = sunlight_libc::read(fd, out).ok()?;
        if n == 0 {
            return None;
        }
        out = &mut out[n..];
    }
    Some(())
}
impl Drop for MusicFile {
    fn drop(&mut self) {
        let _ = sunlight_libc::close(self.fd);
    }
}
extern "C" fn worker(arg: *mut u8) -> *mut u8 {
    let shared = unsafe { Arc::from_raw(arg as *const Shared) };
    let client = AudioClient::new();
    let mut mixer = Mixer::default();
    let mut chunk = [0u8; 4096];
    let mut music_chunk = [0u8; 4096];
    let mut music_file: Option<MusicFile> = None;
    let mut track = Track::Silent;
    let mut music_retry = 0;
    let mut was_silent = false;
    let mut submitted = 0;
    let mut ready = false;
    let mut retry = 0;
    while !shared.shutdown.load(Ordering::Acquire) {
        let settings = unpack(shared.settings.load(Ordering::Acquire));
        let now = monotonic_millis();
        let desired = match shared.track.load(Ordering::Acquire) {
            0 => Track::Menu,
            1 => Track::Peace,
            2 => Track::Combat,
            _ => Track::Silent,
        };
        let silent = settings.mute || settings.master == 0 || shared.paused.load(Ordering::Acquire);
        if desired != track {
            music_file = None;
            track = desired;
            music_retry = 0;
            stop(&client, submitted);
            submitted = 0;
        }
        if silent && !was_silent {
            mixer.clear();
            stop(&client, submitted);
            submitted = 0;
        }
        was_silent = silent;
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
        if !silent
            && settings.music > 0
            && music_file.is_none()
            && now >= music_retry
            && track != Track::Silent
        {
            music_file = MusicFile::open(track);
            music_retry = now.saturating_add(5000);
            sunlight_ipc::debug_log(if music_file.is_some() {
                "[EMPIRES-MUSIC] resource opened\n"
            } else {
                "[EMPIRES-MUSIC] resource unavailable/unsupported\n"
            });
        }
        let playing = !silent && settings.music > 0 && music_file.is_some();
        if silent {
            shared.cues.store(0, Ordering::Release);
        }
        if now < retry {
            worker_wait();
            continue;
        }
        if silent || (!mixer.active() && !playing) {
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
                    worker_wait();
                    continue;
                }
                if submitted == 0 && status.buffered_frames > 0 {
                    mixer.clear();
                    worker_wait();
                    continue;
                }
                if status.buffered_frames >= 2048 {
                    worker_wait();
                    continue;
                }
                mixer.render(&mut chunk);
                if playing {
                    if music_file
                        .as_mut()
                        .unwrap()
                        .render(&mut music_chunk)
                        .is_some()
                    {
                        music::mix(&mut chunk, &music_chunk, settings.master, settings.music);
                    } else {
                        music_file = None;
                        music_retry = now.saturating_add(5000);
                        sunlight_ipc::debug_log(
                            "[EMPIRES-MUSIC] read failure; continuing without music\n",
                        );
                    }
                }
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
