//! One persistent recording worker; capture and file descriptors never cross to the UI.

use crate::{
    state::{duration_millis, transition, Action, State},
    storage::{
        native::{io, NativeFs},
        Error, Recording,
    },
    wav::Format,
};
use alloc::{boxed::Box, format, string::String};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering},
};
use sunlight_audiod::{
    capture::{CaptureState, CAPTURE_PERIOD_BYTES},
    AudioClientError, CaptureClient,
};
use sunlight_ipc::{
    debug_log, endpoint_create, endpoint_destroy, ipc_recv_timeout, monotonic_millis,
    process_is_alive, process_yield,
};
use sunlight_libc as libc;
use sunlight_media::{MediaPlayer, PlaybackState};

const BATCH: usize = 32768;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Command {
    Record = 1,
    Pause,
    Resume,
    Stop,
    Play,
    PausePlayback,
    ResumePlayback,
    Save,
    New,
}

struct Target {
    bytes: [u8; libc::MAX_PATH],
    len: usize,
    replace: bool,
}
struct Shared {
    command: AtomicU8,
    shutdown: AtomicBool,
    done: AtomicBool,
    state: AtomicU8,
    frames: AtomicU64,
    rate: AtomicU32,
    channels: AtomicU8,
    peak: AtomicU16,
    saved: AtomicBool,
    error: AtomicU8,
    errno: AtomicI32,
    target: UnsafeCell<Target>,
    owner_pid: u64,
}
// The UI writes target only while reserving command=255; worker reads it only
// after the release publication of Save. The slot remains busy until completion.
unsafe impl Sync for Shared {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State,
    pub frames: u64,
    pub rate: u32,
    pub channels: u8,
    pub peak: u16,
    pub saved: bool,
    pub busy: bool,
    pub error: Option<Error>,
}
impl Snapshot {
    pub fn duration_ms(self) -> u64 {
        duration_millis(self.frames, self.rate)
    }
}

pub struct Worker {
    shared: Option<Box<Shared>>,
    handle: Option<libc::thread::JoinHandle>,
}
impl Worker {
    pub fn new() -> Self {
        let shared = Box::new(Shared {
            command: AtomicU8::new(0),
            shutdown: AtomicBool::new(false),
            done: AtomicBool::new(false),
            state: AtomicU8::new(State::Idle as u8),
            frames: AtomicU64::new(0),
            rate: AtomicU32::new(48000),
            channels: AtomicU8::new(2),
            peak: AtomicU16::new(0),
            saved: AtomicBool::new(false),
            error: AtomicU8::new(0),
            errno: AtomicI32::new(0),
            owner_pid: libc::getpid(),
            target: UnsafeCell::new(Target {
                bytes: [0; libc::MAX_PATH],
                len: 0,
                replace: false,
            }),
        });
        let ptr = (&*shared as *const Shared).cast_mut().cast::<u8>();
        let handle = unsafe { libc::thread::spawn(entry, ptr) }.ok();
        if handle.is_none() {
            shared.set_error(Error::Storage);
            shared.state.store(State::Error as u8, Ordering::Release);
        }
        Self {
            shared: Some(shared),
            handle,
        }
    }
    fn shared(&self) -> &Shared {
        self.shared.as_deref().unwrap()
    }
    pub fn snapshot(&self) -> Snapshot {
        let s = self.shared();
        Snapshot {
            state: State::from_u8(s.state.load(Ordering::Acquire)),
            frames: s.frames.load(Ordering::Acquire),
            rate: s.rate.load(Ordering::Acquire),
            channels: s.channels.load(Ordering::Acquire),
            peak: s.peak.load(Ordering::Acquire),
            saved: s.saved.load(Ordering::Acquire),
            busy: s.command.load(Ordering::Acquire) != 0,
            error: s.get_error(),
        }
    }
    pub fn send(&self, command: Command) -> Result<(), Error> {
        if self.handle.is_none() {
            return Err(Error::Storage);
        }
        transition(self.snapshot().state, action(command)).map_err(|_| Error::State)?;
        self.shared()
            .command
            .compare_exchange(0, command as u8, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error::State)
    }
    pub fn save(&self, path: &str, replace: bool) -> Result<(), Error> {
        if self.handle.is_none()
            || path.is_empty()
            || path.len() >= libc::MAX_PATH
            || path.as_bytes().contains(&0)
        {
            return Err(Error::State);
        }
        transition(self.snapshot().state, Action::Save).map_err(|_| Error::State)?;
        let s = self.shared();
        s.command
            .compare_exchange(0, 255, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::State)?;
        let target = unsafe { &mut *s.target.get() };
        target.bytes[..path.len()].copy_from_slice(path.as_bytes());
        target.len = path.len();
        target.replace = replace;
        s.command.store(Command::Save as u8, Ordering::Release);
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.handle.is_none() {
            return;
        }
        self.shared().shutdown.store(true, Ordering::Release);
        // Disk commands are bounded in memory, but allow a slow disk to finish
        // cleanup. Never free a shared allocation while its worker can use it.
        let deadline = monotonic_millis().saturating_add(5000);
        while !self.shared().done.load(Ordering::Acquire) && monotonic_millis() < deadline {
            process_yield();
        }
        if !self.shared().done.load(Ordering::Acquire) {
            debug_log("[VOICE-RECORDER] shutdown-timeout; process teardown required");
            if let Some(shared) = self.shared.take() {
                let _ = Box::leak(shared);
            }
        }
    }
}

fn action(command: Command) -> Action {
    match command {
        Command::Record => Action::Record,
        Command::Pause => Action::Pause,
        Command::Resume => Action::Resume,
        Command::Stop => Action::Stop,
        Command::Play => Action::Play,
        Command::PausePlayback => Action::PausePlayback,
        Command::ResumePlayback => Action::ResumePlayback,
        Command::Save => Action::Save,
        Command::New => Action::New,
    }
}

impl Shared {
    fn set_error(&self, error: Error) {
        let (kind, code) = match error {
            Error::Io(n) => (1, n),
            Error::Format => (2, 0),
            Error::Length => (3, 0),
            Error::State => (4, 0),
            Error::Storage => (5, 0),
            Error::Capture => (6, 0),
            Error::Busy => (7, 0),
            Error::Interrupted => (8, 0),
            Error::Empty => (9, 0),
            Error::Playback => (10, 0),
            Error::Cancelled => (11, 0),
        };
        self.errno.store(code, Ordering::Relaxed);
        self.error.store(kind, Ordering::Release);
    }
    fn get_error(&self) -> Option<Error> {
        Some(match self.error.load(Ordering::Acquire) {
            0 => return None,
            1 => Error::Io(self.errno.load(Ordering::Relaxed)),
            2 => Error::Format,
            3 => Error::Length,
            4 => Error::State,
            5 => Error::Storage,
            6 => Error::Capture,
            7 => Error::Busy,
            8 => Error::Interrupted,
            9 => Error::Empty,
            10 => Error::Playback,
            _ => Error::Cancelled,
        })
    }
    fn publish(&self, state: State, frames: u64) {
        self.frames.store(frames, Ordering::Release);
        self.state.store(state as u8, Ordering::Release);
    }
}

fn capture_error(e: AudioClientError) -> Error {
    match e {
        AudioClientError::Overflow => Error::Busy,
        AudioClientError::InvalidFormat => Error::Format,
        _ => Error::Capture,
    }
}

pub fn recording_directory() -> String {
    format!("{}/Musics", storage_home())
}

fn storage_home() -> String {
    if let Some(home) =
        libc::env::getenv(b"HOME").and_then(|home| libc::file_ops::normalize(home).ok())
    {
        if libc::stat(home.as_bytes()).is_ok_and(|stat| stat.file_type == libc::FT_DIR) {
            return home;
        }
    }
    format!("/tmp/voice-recorder-{}", libc::getuid())
}

fn ensure_directory(path: &str) -> Result<(), Error> {
    match libc::stat(path.as_bytes()) {
        Ok(stat) if stat.file_type == libc::FT_DIR => Ok(()),
        Ok(_) => Err(Error::Io(20)),
        Err(libc::sys::Errno::NoEntry) => {
            match libc::mkdir(path.as_bytes(), 0o700) {
                Ok(()) => Ok(()),
                // Another recorder may have created the folder after stat.
                Err(_)
                    if libc::stat(path.as_bytes()).is_ok_and(|s| s.file_type == libc::FT_DIR) =>
                {
                    Ok(())
                }
                Err(error) => {
                    debug_log(&format!(
                        "[VOICE-RECORDER] mkdir failed path={} error={:?}\n",
                        path, error
                    ));
                    Err(io(error))
                }
            }
        }
        Err(error) => Err(io(error)),
    }
}

fn temp_directory() -> Result<String, Error> {
    let home = storage_home();
    ensure_directory(&home)?;
    let temp = format!("{}/.voice-recorder-{}", home, libc::getuid());
    ensure_directory(&temp)?;
    ensure_directory(&recording_directory())?;
    let mut page = [libc::DirEntry::zeroed(); 64];
    let mut offset = 0;
    // Names are exclusively created by this app. Remove only candidates whose
    // owning process has vanished; never delete another live recorder's file.
    loop {
        let count = libc::read_dir_from(temp.as_bytes(), &mut page, offset).map_err(io)?;
        let mut removed = false;
        for item in &page[..count] {
            if let Ok(name) = core::str::from_utf8(item.name_bytes()) {
                if let Some(rest) = name
                    .strip_prefix("session-")
                    .filter(|_| name.ends_with(".part"))
                {
                    if let Some(pid) = rest.split('-').next().and_then(|s| s.parse::<u64>().ok()) {
                        if !process_is_alive(pid)
                            && libc::unlink(format!("{}/{}", temp, name).as_bytes()).is_ok()
                        {
                            removed = true;
                        }
                    }
                }
            }
        }
        if count < page.len() {
            break;
        }
        // Removing a page can shift offsets. Re-scan from zero after removal;
        // bounded candidates per process make normal startup small.
        if removed {
            offset = 0;
        } else {
            offset += count;
        }
    }
    Ok(temp)
}

struct Session {
    capture: Option<CaptureClient>,
    recording: Option<Recording<NativeFs>>,
    player: MediaPlayer,
    batch: [u8; BATCH],
    used: usize,
    state: State,
    last_audio: u64,
    last_heartbeat: u64,
    peak_max: u16,
}

impl Session {
    fn new() -> Self {
        Self {
            capture: None,
            recording: None,
            player: MediaPlayer::new(),
            batch: [0; BATCH],
            used: 0,
            state: State::Idle,
            last_audio: 0,
            last_heartbeat: 0,
            peak_max: 0,
        }
    }
    fn frames(&self) -> u64 {
        self.recording.as_ref().map_or(0, |r| r.frames)
    }
    fn flush(&mut self, shared: &Shared) -> Result<(), Error> {
        if self.used != 0 {
            self.recording
                .as_mut()
                .ok_or(Error::State)?
                .append(&self.batch[..self.used])?;
            self.used = 0;
            shared.frames.store(self.frames(), Ordering::Release);
        }
        Ok(())
    }
    fn read(&mut self, shared: &Shared) -> Result<bool, Error> {
        let status = self
            .capture
            .as_mut()
            .ok_or(Error::State)?
            .read(&mut self.batch[self.used..self.used + CAPTURE_PERIOD_BYTES])
            .map_err(capture_error)?;
        if status.dropped_buffers != 0 || status.state == CaptureState::Closed {
            debug_log(&format!("[VOICE-RECORDER] capture-interrupted state={:?} received={} dropped={} queued={} bytes={}\n", status.state, status.completed_buffers, status.dropped_buffers, status.queued_buffers, status.bytes_read));
            return Err(Error::Interrupted);
        }
        shared.peak.store(status.peak, Ordering::Relaxed);
        self.peak_max = self.peak_max.max(status.peak);
        if status.bytes_read == 0 {
            return Ok(false);
        }
        self.last_audio = monotonic_millis();
        self.used += status.bytes_read;
        if self.used == BATCH {
            self.flush(shared)?;
        }
        Ok(true)
    }
    fn stop_capture(&mut self, shared: &Shared) -> Result<(), Error> {
        self.capture
            .as_ref()
            .ok_or(Error::State)?
            .stop()
            .map_err(capture_error)?;
        // STOP freezes production and retains its bounded FIFO. Drain all
        // completed periods, then commit our partial batch before finalization.
        for _ in 0..=8 {
            if !self.read(shared)? {
                break;
            }
        }
        self.flush(shared)?;
        shared.peak.store(0, Ordering::Release);
        Ok(())
    }
    fn abandon(&mut self) {
        self.capture = None;
        self.recording = None;
        self.used = 0;
    }
    fn command(&mut self, command: Command, shared: &Shared) -> Result<(), Error> {
        let next = transition(self.state, action(command)).map_err(|_| Error::State)?;
        match command {
            Command::Record => {
                let dir = temp_directory()?;
                let mut capture = CaptureClient::new();
                if !capture.input_available().map_err(capture_error)? {
                    return Err(Error::Capture);
                }
                let status = capture.open().map_err(capture_error)?;
                let format = Format {
                    rate: status.rate_hz,
                    channels: status.channels,
                    bits: status.bits,
                }
                .validate()?;
                let path = format!(
                    "{}/session-{}-{}.part",
                    dir,
                    shared.owner_pid,
                    monotonic_millis()
                );
                let recording = Recording::create(NativeFs, path, format)?;
                let started = capture.start().map_err(capture_error)?;
                debug_log(&format!(
                    "[VOICE-RECORDER] started received={} dropped={} queued={}\n",
                    started.completed_buffers, started.dropped_buffers, started.queued_buffers
                ));
                self.capture = Some(capture);
                self.recording = Some(recording);
                self.used = 0;
                self.peak_max = 0;
                self.last_audio = monotonic_millis();
                self.last_heartbeat = self.last_audio;
                shared.rate.store(format.rate, Ordering::Release);
                shared.channels.store(format.channels, Ordering::Release);
                shared.saved.store(false, Ordering::Release);
                debug_log(&format!(
                    "[VOICE-RECORDER] capture rate={} channels={} bits={} batch_bytes={}\n",
                    format.rate, format.channels, format.bits, BATCH
                ));
            }
            Command::Pause => {
                self.stop_capture(shared)?;
                self.last_heartbeat = monotonic_millis();
            }
            Command::Resume => {
                self.capture
                    .as_ref()
                    .ok_or(Error::State)?
                    .start()
                    .map_err(capture_error)?;
                self.last_audio = monotonic_millis();
            }
            Command::Stop if matches!(self.state, State::Recording | State::Paused) => {
                self.state = State::Finalizing;
                shared.publish(self.state, self.frames());
                if self.capture.is_some() {
                    self.stop_capture(shared)?;
                }
                self.capture
                    .as_mut()
                    .ok_or(Error::State)?
                    .close()
                    .map_err(capture_error)?;
                self.capture = None;
                let recording = self.recording.as_mut().ok_or(Error::State)?;
                recording.finalize()?;
                let frames = recording.frames;
                self.state = transition(State::Finalizing, Action::Finalized { frames })
                    .map_err(|_| Error::State)?;
                debug_log(&format!(
                    "[VOICE-RECORDER] finalized frames={} duration_ms={} peak={} file={}\n",
                    frames,
                    duration_millis(frames, recording.format.rate),
                    self.peak_max,
                    recording.path
                ));
                if frames == 0 {
                    self.recording = None;
                    shared.set_error(Error::Empty);
                }
                return Ok(());
            }
            Command::Play => {
                let path = &self.recording.as_ref().ok_or(Error::State)?.path;
                self.player.open(path).map_err(|_| Error::Playback)?;
                wait_player(&self.player, shared)?;
                if self.player.snapshot().state != PlaybackState::Ready {
                    return Err(Error::Playback);
                }
                self.player.play().map_err(|_| Error::Playback)?;
                wait_player(&self.player, shared)?;
            }
            Command::PausePlayback => {
                self.player.pause().map_err(|_| Error::Playback)?;
                wait_player(&self.player, shared)?;
            }
            Command::ResumePlayback => {
                self.player.play().map_err(|_| Error::Playback)?;
                wait_player(&self.player, shared)?;
            }
            Command::Stop => {
                self.player.stop().map_err(|_| Error::Playback)?;
                wait_player(&self.player, shared)?;
            }
            Command::Save => {
                let target = unsafe { &*shared.target.get() };
                let path =
                    core::str::from_utf8(&target.bytes[..target.len]).map_err(|_| Error::State)?;
                let path = libc::file_ops::normalize(path).map_err(|_| Error::State)?;
                let parent = path
                    .rsplit_once('/')
                    .map(|(p, _)| if p.is_empty() { "/" } else { p })
                    .ok_or(Error::State)?;
                let staging = format!(
                    "{}/.voice-save-{}-{}.part",
                    parent,
                    shared.owner_pid,
                    monotonic_millis()
                );
                self.recording
                    .as_mut()
                    .ok_or(Error::State)?
                    .save_as_with_cancel(&path, &staging, target.replace, || {
                        shared.shutdown.load(Ordering::Acquire)
                    })?;
                shared.saved.store(true, Ordering::Release);
                debug_log(&format!(
                    "[VOICE-RECORDER] saved file={} frames={}\n",
                    path,
                    self.frames()
                ));
            }
            Command::New => {
                self.player.close().map_err(|_| Error::Playback)?;
                wait_player(&self.player, shared)?;
                self.abandon();
                shared.saved.store(false, Ordering::Release);
            }
        }
        self.state = next;
        Ok(())
    }
}

fn wait_player(player: &MediaPlayer, shared: &Shared) -> Result<(), Error> {
    let deadline = monotonic_millis() + 3000;
    while player.command_pending() {
        if shared.shutdown.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        if monotonic_millis() >= deadline {
            return Err(Error::Playback);
        }
        process_yield();
    }
    if player.snapshot().error.is_some() {
        return Err(Error::Playback);
    }
    Ok(())
}

extern "C" fn entry(arg: *mut u8) -> *mut u8 {
    let shared = unsafe { &*arg.cast::<Shared>() };
    let ep = endpoint_create();
    {
        let mut session = Session::new();
        let mut last_diagnostic = 0;
        loop {
            if shared.shutdown.load(Ordering::Acquire) {
                session.capture = None;
                let recording = session.recording.take();
                drop(session); // Stop playback and close its source first.
                drop(recording);
                break;
            }
            let value = shared.command.load(Ordering::Acquire);
            let command = match value {
                1 => Some(Command::Record),
                2 => Some(Command::Pause),
                3 => Some(Command::Resume),
                4 => Some(Command::Stop),
                5 => Some(Command::Play),
                6 => Some(Command::PausePlayback),
                7 => Some(Command::ResumePlayback),
                8 => Some(Command::Save),
                9 => Some(Command::New),
                _ => None,
            };
            if let Some(command) = command {
                shared.error.store(0, Ordering::Release);
                if let Err(e) = session.command(command, shared) {
                    shared.set_error(e);
                    // Save failure preserves the finalized recording. Playback
                    // failure similarly permits Save As and another Play attempt.
                    if !matches!(
                        command,
                        Command::Save
                            | Command::Play
                            | Command::PausePlayback
                            | Command::ResumePlayback
                    ) {
                        session.abandon();
                        session.state = State::Error;
                    } else if command != Command::Save {
                        let _ = session.player.stop();
                        session.state = State::Recorded;
                    }
                    debug_log(&format!("[VOICE-RECORDER] error={:?}\n", e));
                }
                shared.publish(session.state, session.frames());
                shared.command.store(0, Ordering::Release);
                debug_log(&format!(
                    "[VOICE-RECORDER] state={:?} frames={}\n",
                    session.state,
                    session.frames()
                ));
            }
            let result = if session.state == State::Recording {
                // At most one FIFO of IPC reads per pass, with a persistent batch.
                let mut result = Ok(());
                for _ in 0..8 {
                    match session.read(shared) {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(e) => {
                            result = Err(e);
                            break;
                        }
                    }
                }
                if result.is_ok() && monotonic_millis().saturating_sub(session.last_audio) > 3000 {
                    debug_log("[VOICE-RECORDER] capture-no-frames timeout=3000ms");
                    result = Err(Error::Interrupted);
                }
                result
            } else if session.state == State::Paused
                && monotonic_millis().saturating_sub(session.last_heartbeat) >= 1000
            {
                session.last_heartbeat = monotonic_millis();
                session
                    .capture
                    .as_ref()
                    .ok_or(Error::State)
                    .and_then(|c| c.status().map_err(capture_error))
                    .and_then(|s| {
                        if s.state == CaptureState::Open {
                            Ok(())
                        } else {
                            Err(Error::Interrupted)
                        }
                    })
            } else {
                Ok(())
            };
            if let Err(e) = result {
                debug_log(&format!("[VOICE-RECORDER] capture-error={:?}\n", e));
                session.abandon();
                session.state = State::Error;
                shared.set_error(e);
                shared.peak.store(0, Ordering::Release);
                shared.publish(session.state, 0);
            }
            if matches!(session.state, State::Playing | State::PlaybackPaused) {
                match session.player.snapshot().state {
                    PlaybackState::Ended | PlaybackState::Ready => {
                        session.state = State::Recorded;
                        shared.publish(session.state, session.frames());
                        debug_log("[VOICE-RECORDER] playback-ended");
                    }
                    PlaybackState::Error => {
                        session.state = State::Recorded;
                        shared.set_error(Error::Playback);
                        shared.publish(session.state, session.frames());
                    }
                    _ => {}
                }
            }
            let now = monotonic_millis();
            if session.state == State::Recording && now.saturating_sub(last_diagnostic) >= 10000 {
                last_diagnostic = now;
                let heap = libc::alloc::heap_stats();
                debug_log(&format!(
                    "[VOICE-RECORDER] metrics frames={} requested={} backing={} live={} peak={}\n",
                    session.frames(),
                    heap.requested_user_bytes,
                    heap.allocated_backing_bytes,
                    heap.live_allocation_count,
                    heap.high_water_allocated_bytes
                ));
            }
            let timeout = if session.state == State::Recording {
                5
            } else {
                25
            };
            if ep.0 != u64::MAX {
                let _ = ipc_recv_timeout(ep, timeout);
            } else {
                process_yield();
            }
        }
    } // player drops and flushes before acknowledging shutdown.
    if ep.0 != u64::MAX {
        endpoint_destroy(ep);
    }
    debug_log("[VOICE-RECORDER] cleanup-complete");
    shared.done.store(true, Ordering::Release);
    core::ptr::null_mut()
}
