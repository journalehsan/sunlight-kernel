//! High-level player API and bounded native playback worker.

use alloc::boxed::Box;
#[cfg(target_os = "none")]
use alloc::vec::Vec;
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
};

#[cfg(target_os = "none")]
use crate::{
    decoder::{AudioDecoder, ProbeDecoder, MAX_COMPRESSED_BYTES},
    output::{AudioSink, SunlightAudioSink},
    state::{transition, PlaybackAction},
};
use crate::{
    error::{MediaError, MediaErrorKind},
    types::{AudioStreamInfo, MediaTime, PcmFormat, PlaybackState},
    visualization::{VisualizationFrame, MAX_VISUALIZATION_BINS},
};

const MAX_PATH_BYTES: usize = sunlight_libc::MAX_PATH - 1;
const COMMAND_NONE: u8 = 0;
const COMMAND_OPEN: u8 = 1;
const COMMAND_PLAY: u8 = 2;
const COMMAND_PAUSE: u8 = 3;
const COMMAND_STOP: u8 = 4;
const COMMAND_SEEK: u8 = 5;
#[cfg(target_os = "none")]
const COMMAND_SHUTDOWN: u8 = 6;

#[cfg(any(target_os = "none", test))]
fn validate_output_contract(info: AudioStreamInfo) -> Result<(), MediaError> {
    if info.sample_rate_hz != sunlight_audio::NATIVE_RATE_HZ
        || !matches!(info.channels, 1 | 2)
        || info.sample_format != PcmFormat::Signed16LeInterleaved
    {
        return Err(MediaError::new(
            MediaErrorKind::UnsupportedSampleFormat,
            (info.sample_rate_hz / 100).saturating_add(info.channels as u32),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaSnapshot {
    /// Monotonically increasing source identity. Consumers use this to reject
    /// snapshots captured for a source that has since been replaced.
    pub generation: u64,
    pub state: PlaybackState,
    pub position: MediaTime,
    pub stream: Option<AudioStreamInfo>,
    pub volume: u8,
    pub error: Option<MediaError>,
    pub visualization: VisualizationFrame,
}

struct PathSlot {
    bytes: [u8; MAX_PATH_BYTES],
    len: usize,
}

struct Shared {
    command: AtomicU8,
    command_locked: AtomicBool,
    generation: AtomicU64,
    state: AtomicU8,
    error: AtomicU8,
    error_detail: AtomicU64,
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    duration_known: AtomicBool,
    rate_hz: AtomicU64,
    channels: AtomicU8,
    seekable: AtomicBool,
    seek_ms: AtomicU64,
    volume: AtomicU8,
    path_locked: AtomicBool,
    path: UnsafeCell<PathSlot>,
    visualization_len: AtomicU8,
    visualization: [AtomicU8; MAX_VISUALIZATION_BINS],
    #[cfg(target_os = "none")]
    done: AtomicBool,
}

// SAFETY: `path` is accessed only while `path_locked` is held. Every other
// field is atomic and the allocation stays pinned for the worker lifetime.
unsafe impl Sync for Shared {}

impl Shared {
    fn new() -> Self {
        Self {
            command: AtomicU8::new(COMMAND_NONE),
            command_locked: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            state: AtomicU8::new(PlaybackState::Idle as u8),
            error: AtomicU8::new(MediaErrorKind::None as u8),
            error_detail: AtomicU64::new(0),
            position_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            duration_known: AtomicBool::new(false),
            rate_hz: AtomicU64::new(0),
            channels: AtomicU8::new(0),
            seekable: AtomicBool::new(false),
            seek_ms: AtomicU64::new(0),
            volume: AtomicU8::new(68),
            path_locked: AtomicBool::new(false),
            path: UnsafeCell::new(PathSlot {
                bytes: [0; MAX_PATH_BYTES],
                len: 0,
            }),
            visualization_len: AtomicU8::new(0),
            visualization: core::array::from_fn(|_| AtomicU8::new(0)),
            #[cfg(target_os = "none")]
            done: AtomicBool::new(false),
        }
    }

    #[cfg(target_os = "none")]
    fn publish_error(&self, error: MediaError) {
        self.error_detail
            .store(error.detail as u64, Ordering::Release);
        self.error.store(error.kind as u8, Ordering::Release);
        self.state
            .store(PlaybackState::Error as u8, Ordering::Release);
    }

    #[cfg(target_os = "none")]
    fn clear_error(&self) {
        self.error
            .store(MediaErrorKind::None as u8, Ordering::Release);
        self.error_detail.store(0, Ordering::Release);
    }

    #[cfg(target_os = "none")]
    fn publish_stream(&self, info: AudioStreamInfo) {
        self.rate_hz
            .store(info.sample_rate_hz as u64, Ordering::Release);
        self.channels.store(info.channels, Ordering::Release);
        self.seekable.store(info.seekable, Ordering::Release);
        if let Some(duration) = info.duration {
            self.duration_ms
                .store(duration.as_millis(), Ordering::Release);
            self.duration_known.store(true, Ordering::Release);
        } else {
            self.duration_known.store(false, Ordering::Release);
            self.duration_ms.store(0, Ordering::Release);
        }
    }

    #[cfg(target_os = "none")]
    fn publish_visualization(&self, frame: &VisualizationFrame) {
        for (target, value) in self.visualization.iter().zip(frame.bins.iter()) {
            target.store(*value, Ordering::Relaxed);
        }
        self.visualization_len.store(frame.len, Ordering::Release);
    }

    #[cfg(target_os = "none")]
    fn clear_visualization(&self) {
        for target in &self.visualization {
            target.store(0, Ordering::Relaxed);
        }
        self.visualization_len.store(0, Ordering::Release);
    }
}

pub struct MediaPlayer {
    shared: Option<Box<Shared>>,
    #[cfg(target_os = "none")]
    _worker: Option<sunlight_libc::thread::JoinHandle>,
}

impl MediaPlayer {
    pub fn new() -> Self {
        let shared = Box::new(Shared::new());
        #[cfg(target_os = "none")]
        let worker = {
            let ptr = (&*shared as *const Shared).cast_mut().cast::<u8>();
            match unsafe { sunlight_libc::thread::spawn(worker_entry, ptr) } {
                Ok(handle) => Some(handle),
                Err(_) => {
                    shared.publish_error(MediaError::new(MediaErrorKind::Worker, 1));
                    None
                }
            }
        };
        Self {
            shared: Some(shared),
            #[cfg(target_os = "none")]
            _worker: worker,
        }
    }

    fn shared(&self) -> &Shared {
        self.shared.as_deref().expect("media shared state")
    }

    fn command(&self, command: u8) -> Result<(), MediaError> {
        let shared = self.shared();
        if shared
            .command_locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(MediaError::new(MediaErrorKind::Busy, command as u32));
        }
        let result = shared
            .command
            .compare_exchange(COMMAND_NONE, command, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| MediaError::new(MediaErrorKind::Busy, command as u32));
        shared.command_locked.store(false, Ordering::Release);
        result
    }

    pub fn snapshot(&self) -> MediaSnapshot {
        let shared = self.shared();
        let rate_hz = shared.rate_hz.load(Ordering::Acquire) as u32;
        let channels = shared.channels.load(Ordering::Acquire);
        let stream = (rate_hz != 0 && channels != 0).then(|| AudioStreamInfo {
            sample_rate_hz: rate_hz,
            channels,
            sample_format: PcmFormat::Signed16LeInterleaved,
            duration: shared
                .duration_known
                .load(Ordering::Acquire)
                .then(|| MediaTime::from_millis(shared.duration_ms.load(Ordering::Acquire))),
            seekable: shared.seekable.load(Ordering::Acquire),
        });
        let kind = MediaErrorKind::from_u8(shared.error.load(Ordering::Acquire));
        let mut visualization = VisualizationFrame::empty();
        visualization.len = shared
            .visualization_len
            .load(Ordering::Acquire)
            .min(MAX_VISUALIZATION_BINS as u8);
        for (target, source) in visualization
            .bins
            .iter_mut()
            .zip(shared.visualization.iter())
        {
            *target = source.load(Ordering::Relaxed);
        }
        MediaSnapshot {
            generation: shared.generation.load(Ordering::Acquire),
            state: PlaybackState::from_u8(shared.state.load(Ordering::Acquire)),
            position: MediaTime::from_millis(shared.position_ms.load(Ordering::Acquire)),
            stream,
            volume: shared.volume.load(Ordering::Acquire).min(100),
            error: (kind != MediaErrorKind::None)
                .then(|| MediaError::new(kind, shared.error_detail.load(Ordering::Acquire) as u32)),
            visualization,
        }
    }

    pub fn open(&mut self, path: &str) -> Result<(), MediaError> {
        if path.is_empty() || path.len() > MAX_PATH_BYTES || path.as_bytes().contains(&0) {
            return Err(MediaError::new(MediaErrorKind::FileOpen, 1));
        }
        let shared = self.shared();
        if PlaybackState::from_u8(shared.state.load(Ordering::Acquire)) == PlaybackState::Loading {
            return Err(MediaError::new(MediaErrorKind::Busy, COMMAND_OPEN as u32));
        }
        if shared
            .command_locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(MediaError::new(MediaErrorKind::Busy, COMMAND_OPEN as u32));
        }
        if shared.command.load(Ordering::Acquire) != COMMAND_NONE {
            shared.command_locked.store(false, Ordering::Release);
            return Err(MediaError::new(MediaErrorKind::Busy, COMMAND_OPEN as u32));
        }
        if shared
            .path_locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            shared.command_locked.store(false, Ordering::Release);
            return Err(MediaError::new(MediaErrorKind::Busy, COMMAND_OPEN as u32));
        }
        // SAFETY: this thread owns `path_locked` until the release below.
        unsafe {
            let slot = &mut *shared.path.get();
            slot.bytes[..path.len()].copy_from_slice(path.as_bytes());
            slot.len = path.len();
        }
        // Publish Loading synchronously. This closes the small window in which
        // a second Open could replace the path slot before the worker copied it
        // and gives clients an immediate, authoritative control state.
        shared.generation.fetch_add(1, Ordering::AcqRel);
        shared
            .error
            .store(MediaErrorKind::None as u8, Ordering::Release);
        shared.error_detail.store(0, Ordering::Release);
        shared.position_ms.store(0, Ordering::Release);
        shared.duration_ms.store(0, Ordering::Release);
        shared.duration_known.store(false, Ordering::Release);
        shared.rate_hz.store(0, Ordering::Release);
        shared.channels.store(0, Ordering::Release);
        shared.seekable.store(false, Ordering::Release);
        shared
            .state
            .store(PlaybackState::Loading as u8, Ordering::Release);
        shared.path_locked.store(false, Ordering::Release);
        shared.command.store(COMMAND_OPEN, Ordering::Release);
        shared.command_locked.store(false, Ordering::Release);
        Ok(())
    }

    pub fn play(&mut self) -> Result<(), MediaError> {
        self.command(COMMAND_PLAY)
    }

    pub fn pause(&mut self) -> Result<(), MediaError> {
        self.command(COMMAND_PAUSE)
    }

    pub fn stop(&mut self) -> Result<(), MediaError> {
        self.command(COMMAND_STOP)
    }

    pub fn seek(&mut self, position: MediaTime) -> Result<(), MediaError> {
        let shared = self.shared();
        if shared
            .command_locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(MediaError::new(MediaErrorKind::Busy, COMMAND_SEEK as u32));
        }
        if shared.command.load(Ordering::Acquire) != COMMAND_NONE {
            shared.command_locked.store(false, Ordering::Release);
            return Err(MediaError::new(MediaErrorKind::Busy, COMMAND_SEEK as u32));
        }
        shared
            .seek_ms
            .store(position.as_millis(), Ordering::Release);
        shared.command.store(COMMAND_SEEK, Ordering::Release);
        shared.command_locked.store(false, Ordering::Release);
        Ok(())
    }

    pub fn set_volume(&mut self, volume: u8) -> Result<(), MediaError> {
        if volume > 100 {
            return Err(MediaError::new(MediaErrorKind::InvalidState, 100));
        }
        self.shared().volume.store(volume, Ordering::Release);
        Ok(())
    }
}

impl Default for MediaPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MediaPlayer {
    fn drop(&mut self) {
        #[cfg(target_os = "none")]
        if self._worker.is_some() {
            // Stop the shared application stream immediately; worker command
            // delivery is still required for decoder/source teardown, but
            // window close must not leave queued or DMA-resident sound behind.
            let _ = sunlight_audiod::AudioClient::new().stop_stream();
            self.shared()
                .state
                .store(PlaybackState::Idle as u8, Ordering::Release);
            let deadline = sunlight_ipc::monotonic_millis().saturating_add(2_000);
            while self.command(COMMAND_SHUTDOWN).is_err()
                && sunlight_ipc::monotonic_millis() < deadline
            {
                sunlight_ipc::process_yield();
            }
            while !self.shared().done.load(Ordering::Acquire)
                && sunlight_ipc::monotonic_millis() < deadline
            {
                sunlight_ipc::process_yield();
            }
            if !self.shared().done.load(Ordering::Acquire) {
                if let Some(shared) = self.shared.take() {
                    let _ = Box::leak(shared);
                }
            }
        }
    }
}

/// Owns compressed storage and a decoder borrowing that stable allocation.
/// `decoder` is declared first so it is dropped before `source`.
#[cfg(target_os = "none")]
struct LoadedMedia {
    decoder: ProbeDecoder<'static>,
    source: Box<[u8]>,
    info: AudioStreamInfo,
}

#[cfg(target_os = "none")]
impl LoadedMedia {
    fn new(source: Box<[u8]>) -> Result<Self, MediaError> {
        let slice = unsafe { core::slice::from_raw_parts(source.as_ptr(), source.len()) };
        let decoder = ProbeDecoder::open(slice)?;
        let info = decoder.stream_info();
        Ok(Self {
            decoder,
            source,
            info,
        })
    }

    fn source_len(&self) -> usize {
        self.source.len()
    }
}

#[cfg(target_os = "none")]
extern "C" fn worker_entry(arg: *mut u8) -> *mut u8 {
    let shared = unsafe { &*(arg.cast::<Shared>()) };
    run_worker(shared);
    shared.done.store(true, Ordering::Release);
    core::ptr::null_mut()
}

#[cfg(target_os = "none")]
fn run_worker(shared: &Shared) {
    let mut loaded: Option<LoadedMedia> = None;
    let mut sink: Option<SunlightAudioSink> = None;
    let mut visualization = VisualizationFrame::empty();
    let mut first_pcm_generation = None;
    loop {
        let command = shared.command.swap(COMMAND_NONE, Ordering::AcqRel);
        if command != COMMAND_NONE {
            if command == COMMAND_SHUTDOWN {
                if let Some(output) = sink.as_mut() {
                    let _ = output.flush();
                }
                return;
            }
            if let Err(error) = handle_command(command, shared, &mut loaded, &mut sink) {
                shared.publish_error(error);
            }
        }
        if PlaybackState::from_u8(shared.state.load(Ordering::Acquire)) != PlaybackState::Playing {
            sunlight_ipc::process_yield();
            continue;
        }
        let (Some(media), Some(output)) = (loaded.as_mut(), sink.as_mut()) else {
            shared.publish_error(MediaError::new(MediaErrorKind::InvalidState, 20));
            continue;
        };
        output.set_volume(shared.volume.load(Ordering::Acquire));
        let mut decoded = [0i16; 2048];
        let channels = media.info.channels as usize;
        let capacity = 1024usize.saturating_mul(channels).min(decoded.len());
        let result = match media.decoder.decode(&mut decoded[..capacity]) {
            Ok(result) => result,
            Err(error) => {
                let _ = output.flush();
                if PlaybackState::from_u8(shared.state.load(Ordering::Acquire))
                    == PlaybackState::Playing
                {
                    shared.publish_error(error);
                }
                continue;
            }
        };
        if result.frames != 0 {
            let mut pcm = [0u8; sunlight_ipc::SHM_PAGE];
            for frame in 0..result.frames {
                let (left, right) = if channels == 1 {
                    (decoded[frame], decoded[frame])
                } else {
                    (decoded[frame * 2], decoded[frame * 2 + 1])
                };
                let offset = frame * 4;
                pcm[offset..offset + 2].copy_from_slice(&left.to_le_bytes());
                pcm[offset + 2..offset + 4].copy_from_slice(&right.to_le_bytes());
            }
            let bytes = &pcm[..result.frames * 4];
            visualization.analyze_s16_stereo(bytes, MAX_VISUALIZATION_BINS);
            shared.publish_visualization(&visualization);
            match output.write(bytes) {
                Ok(consumed) => {
                    shared.position_ms.store(
                        MediaTime::from_frames(consumed, media.info.sample_rate_hz).as_millis(),
                        Ordering::Release,
                    );
                    let generation = shared.generation.load(Ordering::Acquire);
                    if first_pcm_generation != Some(generation) {
                        log_first_pcm(
                            &decoded[..result.frames * channels],
                            result.frames,
                            consumed,
                        );
                        first_pcm_generation = Some(generation);
                    }
                }
                Err(error) => {
                    let _ = output.flush();
                    // An Open request changes state to Loading immediately.
                    // Do not let an in-flight failure from the replaced source
                    // overwrite that newer state.
                    if PlaybackState::from_u8(shared.state.load(Ordering::Acquire))
                        == PlaybackState::Playing
                    {
                        shared.publish_error(error);
                    }
                    continue;
                }
            }
        }
        if result.end_of_stream {
            match output.drain() {
                Ok(consumed) => {
                    shared.position_ms.store(
                        MediaTime::from_frames(consumed, media.info.sample_rate_hz).as_millis(),
                        Ordering::Release,
                    );
                    let next = transition(PlaybackState::Playing, PlaybackAction::End)
                        .unwrap_or(PlaybackState::Error);
                    if shared
                        .state
                        .compare_exchange(
                            PlaybackState::Playing as u8,
                            next as u8,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        shared.clear_visualization();
                        log_eof(media.info, consumed);
                    }
                }
                Err(error) => {
                    let _ = output.flush();
                    shared.publish_error(error);
                }
            }
        }
    }
}

#[cfg(target_os = "none")]
fn handle_command(
    command: u8,
    shared: &Shared,
    loaded: &mut Option<LoadedMedia>,
    sink: &mut Option<SunlightAudioSink>,
) -> Result<(), MediaError> {
    match command {
        COMMAND_OPEN => {
            if let Some(output) = sink.as_mut() {
                output.flush()?;
            }
            *loaded = None;
            *sink = None;
            shared.clear_error();
            shared.clear_visualization();
            shared.position_ms.store(0, Ordering::Release);
            shared.rate_hz.store(0, Ordering::Release);
            shared.channels.store(0, Ordering::Release);
            let current = PlaybackState::from_u8(shared.state.load(Ordering::Acquire));
            let loading = transition(current, PlaybackAction::BeginOpen)?;
            shared.state.store(loading as u8, Ordering::Release);
            let path = copy_path(shared)?;
            let source = read_source(&path)?;
            let media = LoadedMedia::new(source)?;
            validate_output_contract(media.info)?;
            log_opened_media(&media);
            let _bounded_bytes = media.source_len();
            let output = SunlightAudioSink::open(shared.volume.load(Ordering::Acquire))?;
            shared.publish_stream(media.info);
            *loaded = Some(media);
            *sink = Some(output);
            let ready = transition(loading, PlaybackAction::LoadReady)?;
            shared.state.store(ready as u8, Ordering::Release);
            Ok(())
        }
        COMMAND_PLAY => {
            let current = PlaybackState::from_u8(shared.state.load(Ordering::Acquire));
            let next = transition(current, PlaybackAction::Play)?;
            match current {
                PlaybackState::Ready | PlaybackState::Paused => {}
                PlaybackState::Ended => {
                    let media = loaded
                        .as_mut()
                        .ok_or_else(|| MediaError::new(MediaErrorKind::InvalidState, 30))?;
                    media.decoder.seek(MediaTime::ZERO)?;
                    sink.as_mut()
                        .ok_or_else(|| MediaError::new(MediaErrorKind::InvalidState, 31))?
                        .flush()?;
                    sink.as_mut()
                        .ok_or_else(|| MediaError::new(MediaErrorKind::InvalidState, 32))?
                        .set_position_frames(0);
                    shared.position_ms.store(0, Ordering::Release);
                }
                PlaybackState::Playing => {}
                _ => {}
            }
            shared.state.store(next as u8, Ordering::Release);
            Ok(())
        }
        COMMAND_PAUSE => {
            if PlaybackState::from_u8(shared.state.load(Ordering::Acquire))
                != PlaybackState::Playing
            {
                return Ok(());
            }
            let media = loaded
                .as_mut()
                .ok_or_else(|| MediaError::new(MediaErrorKind::InvalidState, 33))?;
            let output = sink
                .as_mut()
                .ok_or_else(|| MediaError::new(MediaErrorKind::InvalidState, 34))?;
            output.flush()?;
            let consumed = output.position_frames()?;
            let position = MediaTime::from_frames(consumed, media.info.sample_rate_hz);
            let actual = media.decoder.seek(position)?;
            output.set_position_frames(actual.frames_at(media.info.sample_rate_hz));
            shared.clear_visualization();
            shared
                .position_ms
                .store(actual.as_millis(), Ordering::Release);
            let next = transition(PlaybackState::Playing, PlaybackAction::Pause)?;
            shared.state.store(next as u8, Ordering::Release);
            Ok(())
        }
        COMMAND_STOP => {
            if let Some(output) = sink.as_mut() {
                output.flush()?;
                output.set_position_frames(0);
            }
            if let Some(media) = loaded.as_mut() {
                media.decoder.seek(MediaTime::ZERO)?;
                shared.position_ms.store(0, Ordering::Release);
                shared.clear_visualization();
                let current = PlaybackState::from_u8(shared.state.load(Ordering::Acquire));
                let next = transition(current, PlaybackAction::Stop { loaded: true })?;
                shared.state.store(next as u8, Ordering::Release);
            } else {
                let current = PlaybackState::from_u8(shared.state.load(Ordering::Acquire));
                let next = transition(current, PlaybackAction::Stop { loaded: false })?;
                shared.state.store(next as u8, Ordering::Release);
            }
            Ok(())
        }
        COMMAND_SEEK => {
            let state = PlaybackState::from_u8(shared.state.load(Ordering::Acquire));
            let resume = state == PlaybackState::Playing;
            let media = loaded
                .as_mut()
                .ok_or_else(|| MediaError::new(MediaErrorKind::InvalidState, 35))?;
            let output = sink
                .as_mut()
                .ok_or_else(|| MediaError::new(MediaErrorKind::InvalidState, 36))?;
            output.flush()?;
            let target = MediaTime::from_millis(shared.seek_ms.load(Ordering::Acquire));
            let actual = media.decoder.seek(target)?;
            output.set_position_frames(actual.frames_at(media.info.sample_rate_hz));
            shared
                .position_ms
                .store(actual.as_millis(), Ordering::Release);
            let next = transition(state, PlaybackAction::SeekComplete { resume })?;
            shared.state.store(next as u8, Ordering::Release);
            Ok(())
        }
        _ => Err(MediaError::new(
            MediaErrorKind::InvalidState,
            command as u32,
        )),
    }
}

#[cfg(target_os = "none")]
fn debug_log_u64(mut value: u64) {
    if value == 0 {
        sunlight_ipc::debug_log("0");
        return;
    }
    let mut reversed = [0u8; 20];
    let mut len = 0usize;
    while value != 0 {
        reversed[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    let mut output = [0u8; 20];
    for index in 0..len {
        output[index] = reversed[len - index - 1];
    }
    if let Ok(text) = core::str::from_utf8(&output[..len]) {
        sunlight_ipc::debug_log(text);
    }
}

#[cfg(target_os = "none")]
fn log_opened_media(media: &LoadedMedia) {
    let Some(wav) = media.decoder.wav_diagnostics() else {
        return;
    };
    sunlight_ipc::debug_log("[MEDIA][wav-open] encoding=pcm-s16le rate_hz=");
    debug_log_u64(media.info.sample_rate_hz as u64);
    sunlight_ipc::debug_log(" channels=");
    debug_log_u64(media.info.channels as u64);
    sunlight_ipc::debug_log(" bits=16 data_offset=");
    debug_log_u64(wav.data_offset as u64);
    sunlight_ipc::debug_log(" data_bytes=");
    debug_log_u64(wav.data_bytes as u64);
    sunlight_ipc::debug_log(" total_frames=");
    debug_log_u64(wav.total_frames);
    sunlight_ipc::debug_log(" duration_ms=");
    debug_log_u64(media.info.duration.unwrap_or(MediaTime::ZERO).as_millis());
    sunlight_ipc::debug_log("\n");
}

#[cfg(target_os = "none")]
fn log_first_pcm(samples: &[i16], frame_count: usize, consumed_frames: u64) {
    let min = samples.iter().copied().min().unwrap_or(0);
    let max = samples.iter().copied().max().unwrap_or(0);
    let peak = samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0);
    sunlight_ipc::debug_log("[MEDIA][pcm-first] frame_count=");
    debug_log_u64(frame_count as u64);
    sunlight_ipc::debug_log(" sample_count=");
    debug_log_u64(samples.len() as u64);
    sunlight_ipc::debug_log(" byte_count=");
    debug_log_u64((samples.len() * 2) as u64);
    sunlight_ipc::debug_log(" nonzero=");
    debug_log_u64(samples.iter().filter(|sample| **sample != 0).count() as u64);
    sunlight_ipc::debug_log(" min=");
    debug_log_i64(min as i64);
    sunlight_ipc::debug_log(" max=");
    debug_log_i64(max as i64);
    sunlight_ipc::debug_log(" peak=");
    debug_log_u64(peak as u64);
    sunlight_ipc::debug_log(" consumed_frames=");
    debug_log_u64(consumed_frames);
    sunlight_ipc::debug_log(" sink=48000Hz/s16le/stereo\n");
}

#[cfg(target_os = "none")]
fn debug_log_i64(value: i64) {
    if value < 0 {
        sunlight_ipc::debug_log("-");
        debug_log_u64(value.unsigned_abs());
    } else {
        debug_log_u64(value as u64);
    }
}

#[cfg(target_os = "none")]
fn log_eof(info: AudioStreamInfo, consumed_frames: u64) {
    sunlight_ipc::debug_log("[MEDIA][eof] consumed_frames=");
    debug_log_u64(consumed_frames);
    sunlight_ipc::debug_log(" position_ms=");
    debug_log_u64(MediaTime::from_frames(consumed_frames, info.sample_rate_hz).as_millis());
    sunlight_ipc::debug_log(" duration_ms=");
    debug_log_u64(info.duration.unwrap_or(MediaTime::ZERO).as_millis());
    sunlight_ipc::debug_log(" state=Ended\n");
}

#[cfg(target_os = "none")]
fn copy_path(shared: &Shared) -> Result<[u8; sunlight_libc::MAX_PATH], MediaError> {
    while shared
        .path_locked
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        sunlight_ipc::process_yield();
    }
    let mut path = [0u8; sunlight_libc::MAX_PATH];
    // SAFETY: worker owns `path_locked`.
    let slot = unsafe { &*shared.path.get() };
    if slot.len == 0 || slot.len > MAX_PATH_BYTES {
        shared.path_locked.store(false, Ordering::Release);
        return Err(MediaError::new(MediaErrorKind::FileOpen, 2));
    }
    path[..slot.len].copy_from_slice(&slot.bytes[..slot.len]);
    shared.path_locked.store(false, Ordering::Release);
    Ok(path)
}

#[cfg(target_os = "none")]
fn read_source(path: &[u8; sunlight_libc::MAX_PATH]) -> Result<Box<[u8]>, MediaError> {
    let len = path
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(path.len());
    let fd = sunlight_libc::open(&path[..len])
        .map_err(|_| MediaError::new(MediaErrorKind::FileOpen, 3))?;
    let size = match sunlight_libc::lseek(fd, 0, sunlight_libc::SEEK_END) {
        Ok(size) if size > 0 && size <= MAX_COMPRESSED_BYTES as u64 => size as usize,
        Ok(_) => {
            let _ = sunlight_libc::close(fd);
            return Err(MediaError::new(MediaErrorKind::SourceTooLarge, 1));
        }
        Err(_) => {
            let _ = sunlight_libc::close(fd);
            return Err(MediaError::new(MediaErrorKind::FileRead, 1));
        }
    };
    if sunlight_libc::lseek(fd, 0, sunlight_libc::SEEK_SET).is_err() {
        let _ = sunlight_libc::close(fd);
        return Err(MediaError::new(MediaErrorKind::FileRead, 2));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| MediaError::new(MediaErrorKind::SourceTooLarge, 2))?;
    bytes.resize(size, 0);
    let mut offset = 0usize;
    while offset < size {
        match sunlight_libc::read(fd, &mut bytes[offset..]) {
            Ok(0) => break,
            Ok(count) => offset = offset.saturating_add(count),
            Err(_) => {
                let _ = sunlight_libc::close(fd);
                return Err(MediaError::new(MediaErrorKind::FileRead, 3));
            }
        }
    }
    let _ = sunlight_libc::close(fd);
    if offset != size {
        return Err(MediaError::new(MediaErrorKind::FileRead, 4));
    }
    Ok(bytes.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_defaults_are_idle_and_empty() {
        let player = MediaPlayer::new();
        let snapshot = player.snapshot();
        assert_eq!(snapshot.state, PlaybackState::Idle);
        assert!(snapshot.stream.is_none());
        assert!(snapshot.error.is_none());
        assert_eq!(snapshot.generation, 0);
        assert_eq!(snapshot.volume, 68);
    }

    #[test]
    fn commands_are_bounded_to_one_pending_update() {
        let mut player = MediaPlayer::new();
        assert!(player.play().is_ok());
        assert_eq!(player.pause().unwrap_err().kind, MediaErrorKind::Busy);
    }

    #[test]
    fn open_immediately_publishes_loading_and_rejects_replacement_race() {
        let mut player = MediaPlayer::new();
        player.open("/music/one.ogg").unwrap();
        let snapshot = player.snapshot();
        assert_eq!(snapshot.state, PlaybackState::Loading);
        assert_eq!(snapshot.generation, 1);
        assert_eq!(
            player.open("/music/two.ogg").unwrap_err().kind,
            MediaErrorKind::Busy
        );
    }

    #[test]
    fn non_native_sample_rate_is_a_typed_unsupported_format() {
        let error = validate_output_contract(AudioStreamInfo {
            sample_rate_hz: 44_100,
            channels: 2,
            sample_format: PcmFormat::Signed16LeInterleaved,
            duration: Some(MediaTime::from_millis(194_704)),
            seekable: true,
        })
        .unwrap_err();
        assert_eq!(error.kind, MediaErrorKind::UnsupportedSampleFormat);
    }
}
