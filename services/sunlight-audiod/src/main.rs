//! audiod — SunlightOS system playback service (`audio.v1`).
//!
//! Owns master volume/mute, the single output stream, and the HDA driver.
//! Applications never program PCI or DMA; they talk to this service.

#![no_std]
#![no_main]

use sunlight_audio::{
    effective_system_gain,
    hda::{HdaPlayback, ENGINE_PERIOD_BYTES, PERIOD_FRAME_COUNT},
    pcm::{validate_pcm, AudioBuffer, AudioFormat, MAX_PCM_BYTES, NATIVE_RATE_HZ},
    render_persisted_buf, AudioDeviceState, AudioError, MasterVolume, OutputDeviceKind,
    PersistedAudio, SystemSoundSettings,
};
use sunlight_audiod::{
    decode_system_sound_request, restore_audio_settings, PcmQueue, QueuedSystemSound,
    StreamProgressTracker, SystemSoundEnqueue, SystemSoundMode, SystemSoundQueue, DEFAULT_TONE_HZ,
    DEFAULT_TONE_MS,
};
use sunlight_ipc::{
    debug_log, endpoint_create, hda_info, ipc_recv_timeout, ipc_reply_result, monotonic_millis,
    nameserver_register, pack_audio_status, pack_audio_stream_status, shm_free, shm_map,
    AudioStatus, AudiodMsg, IpcMsg,
};
use sunlight_libc as libc;

mod system_assets;

const CONFIG_PATH: &str = "/root/.config/sunlight/audio.toml";
const CONFIG_TMP_PATH: &str = "/root/.config/sunlight/audio.toml.tmp";
const ENGINE_WAIT_MS: u64 = 8;
const TONE_DEFAULT_FRAMES: u32 = NATIVE_RATE_HZ; // 1 second
const DMA_FIRST_LOG_FRAMES: u64 = NATIVE_RATE_HZ as u64 / 2;
const DMA_STEADY_LOG_FRAMES: u64 = NATIVE_RATE_HZ as u64 * 60;

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

struct ServiceState {
    volume: MasterVolume,
    device: Option<HdaPlayback>,
    kind: OutputDeviceKind,
    vendor_id: u16,
    device_id: u16,
    queue: PcmQueue,
    queue_owner: Option<u64>,
    stream_progress: StreamProgressTracker,
    system_settings: SystemSoundSettings,
    system_queue: SystemSoundQueue,
    active_system_sound: Option<ActiveSystemSound>,
    bad_asset_mask: u16,
    tone_frames_left: u32,
    tone_phase: u32,
    tone_hz: u32,
    last_state: AudioDeviceState,
    last_dma_log_frames: u64,
    persist_dirty: bool,
    status_diag_count: u8,
    pcm_diag_count: u8,
}

struct ActiveSystemSound {
    request: QueuedSystemSound,
    pcm: &'static [u8],
    offset: usize,
    submitted: bool,
}

impl ServiceState {
    fn new() -> Self {
        let (volume, system_settings) = restore_audio_settings(load_config_text().as_deref());
        let mut device = None;
        let mut kind = OutputDeviceKind::None;
        let mut vendor_id = 0;
        let mut device_id = 0;
        if let Some(info) = hda_info() {
            vendor_id = info.vendor_id;
            device_id = info.device_id;
            kind = OutputDeviceKind::from_pci(vendor_id, device_id);
        }
        match HdaPlayback::open() {
            Ok(mut hda) => {
                let caps = hda.capabilities();
                kind = caps.kind;
                vendor_id = caps.vendor_id;
                device_id = caps.device_id;
                if let Err(err) = hda.start() {
                    serial_println!(
                        "[AUDIOD] stream start failed stage={} code={}",
                        err.as_str(),
                        err as u8
                    );
                } else {
                    let _ = hda.fill_silence_ready();
                    serial_println!("[HDA] stream started {}Hz 16-bit stereo", NATIVE_RATE_HZ);
                    device = Some(hda);
                }
                serial_println!(
                    "[AUDIOD] output {} ({:04x}:{:04x})",
                    kind.as_str(),
                    vendor_id,
                    device_id
                );
            }
            Err(sunlight_audio::hda::HdaError::NoDevice) => {
                serial_println!("[AUDIOD] no output device (unavailable)");
            }
            Err(err) => {
                serial_println!(
                    "[AUDIOD] device init failed stage={} code={}",
                    err.as_str(),
                    err as u8
                );
            }
        }
        Self {
            volume,
            device,
            kind,
            vendor_id,
            device_id,
            queue: PcmQueue::new(),
            queue_owner: None,
            stream_progress: StreamProgressTracker::new(),
            system_settings,
            system_queue: SystemSoundQueue::new(),
            active_system_sound: None,
            bad_asset_mask: 0,
            tone_frames_left: 0,
            tone_phase: 0,
            tone_hz: DEFAULT_TONE_HZ,
            last_state: AudioDeviceState::Unavailable,
            last_dma_log_frames: 0,
            persist_dirty: false,
            status_diag_count: 0,
            pcm_diag_count: 0,
        }
    }

    fn state(&self) -> AudioDeviceState {
        match self.device.as_ref() {
            None => {
                if self.kind == OutputDeviceKind::None {
                    AudioDeviceState::Unavailable
                } else {
                    AudioDeviceState::Failed
                }
            }
            Some(dev) => {
                if self.tone_frames_left > 0
                    || self.active_system_sound.is_some()
                    || !self.system_queue.is_empty()
                    || !self.queue.is_empty()
                {
                    AudioDeviceState::Playing
                } else if dev.underruns() > 0 && !dev.state().is_usable() {
                    AudioDeviceState::Underrun
                } else {
                    dev.state()
                }
            }
        }
    }

    fn status(&self) -> AudioStatus {
        let played = self
            .device
            .as_ref()
            .map(|d| d.frames_played() as u32)
            .unwrap_or(0);
        let underruns = self.device.as_ref().map(|d| d.underruns()).unwrap_or(0);
        let usable = self.device.is_some();
        AudioStatus {
            state: self.state() as u8,
            volume: self.volume.volume(),
            muted: self.volume.muted(),
            last_nonzero: self.volume.last_nonzero(),
            sample_rate_hz: if usable { NATIVE_RATE_HZ } else { 0 },
            channels: if usable { 2 } else { 0 },
            bits: if usable { 16 } else { 0 },
            underruns,
            frames_played: played,
            system_sounds_enabled: self.system_settings.enabled,
            system_sounds_volume: self.system_settings.volume,
            system_sound_queue_len: self.system_queue.len().min(u8::MAX as usize) as u8,
        }
    }

    fn start_tone(&mut self, freq_hz: u32, duration_ms: u32) -> bool {
        if self.device.is_none() {
            return false;
        }
        self.tone_hz = freq_hz;
        self.tone_phase = 0;
        self.tone_frames_left = ((NATIVE_RATE_HZ as u64 * duration_ms as u64) / 1000) as u32;
        if self.tone_frames_left == 0 {
            self.tone_frames_left = TONE_DEFAULT_FRAMES;
        }
        serial_println!("[AUDIOD] test-tone started {}Hz {}ms", freq_hz, duration_ms);
        true
    }

    fn pump(&mut self) {
        if self.tone_frames_left == 0 && self.active_system_sound.is_none() {
            self.activate_next_system_sound();
        }
        let Some(dev) = self.device.as_mut() else {
            return;
        };
        let vol = self.volume.effective();
        let mut tmp = [0u8; ENGINE_PERIOD_BYTES];
        if self.tone_frames_left > 0 {
            match dev.fill_sine(&mut self.tone_phase, self.tone_hz, vol) {
                Ok(true) => {
                    self.stream_progress.clear_period(dev.last_submitted_period());
                    self.tone_frames_left = self
                        .tone_frames_left
                        .saturating_sub(PERIOD_FRAME_COUNT as u32);
                    if self.tone_frames_left == 0 {
                        serial_println!("[AUDIOD] test-tone done");
                    }
                }
                Ok(false) => {}
                Err(_) => {
                    serial_println!("[AUDIOD] tone submit failed");
                    self.tone_frames_left = 0;
                }
            }
        } else if let Some(active) = self.active_system_sound.as_mut() {
            let end = active
                .offset
                .saturating_add(ENGINE_PERIOD_BYTES)
                .min(active.pcm.len());
            let gain = effective_system_gain(vol, self.system_settings.volume);
            match dev.fill_pcm(&active.pcm[active.offset..end], gain) {
                Ok(true) => {
                    self.stream_progress.clear_period(dev.last_submitted_period());
                    if !active.submitted {
                        serial_println!(
                            "[AUDIOD] system-sound pcm submitted id={} gain={}",
                            active.request.sound as u16,
                            gain
                        );
                        active.submitted = true;
                    }
                    active.offset = end;
                    if active.offset >= active.pcm.len() {
                        let sound = active.request.sound;
                        serial_println!(
                            "[AUDIOD] system-sound complete id={} name={}",
                            sound as u16,
                            sound.label()
                        );
                        self.active_system_sound = None;
                    }
                }
                Ok(false) => {}
                Err(_) => {
                    serial_println!("[AUDIOD] system-sound submit failed");
                    self.active_system_sound = None;
                }
            }
        } else if !self.queue.is_empty() {
            // Do not remove producer data until a hardware period is free.
            // `fill_pcm` can legitimately return false while the ring is full;
            // popping first used to discard that entire PCM period.
            if !dev.can_submit_period() {
                let progress = dev.poll_dma_progress_report();
                self.stream_progress.observe_dma(
                    progress.first_completed_period as usize,
                    progress.completed_periods as usize,
                    progress.current_period as usize,
                    progress.current_period_frames,
                );
                return;
            }
            let n = self.queue.peek_into(&mut tmp);
            if n == 0 {
                return;
            }
            match dev.fill_pcm(&tmp[..n], vol) {
                Ok(true) => {
                    let period = dev.last_submitted_period();
                    if let Some(owner) = self.queue_owner {
                        self.stream_progress.tag_period(period, owner, n / 4);
                    }
                    self.queue.consume(n);
                }
                Ok(false) => {}
                Err(_) => self.queue.clear(),
            }
        } else {
            let filled = dev.fill_silence_ready().unwrap_or(0);
            for offset in 0..filled as usize {
                let period = (dev.last_submitted_period() + 4 - offset) % 4;
                self.stream_progress.clear_period(period);
            }
        }
        let progress = dev.poll_dma_progress_report();
        self.stream_progress.observe_dma(
            progress.first_completed_period as usize,
            progress.completed_periods as usize,
            progress.current_period as usize,
            progress.current_period_frames,
        );
        let played = progress.total_frames;
        let log_interval = if self.last_dma_log_frames == 0 {
            DMA_FIRST_LOG_FRAMES
        } else {
            DMA_STEADY_LOG_FRAMES
        };
        if played.saturating_sub(self.last_dma_log_frames) >= log_interval {
            serial_println!(
                "[AUDIOD] dma-progress frames={} lpib={}",
                played,
                dev.position_frames()
            );
            self.last_dma_log_frames = played;
        }
    }

    fn persist_if_dirty(&mut self) {
        if !self.persist_dirty {
            return;
        }
        let mut persisted = self.volume.snapshot();
        persisted.system_sounds_enabled = self.system_settings.enabled;
        persisted.system_sounds_volume = self.system_settings.volume;
        if save_config(persisted) {
            self.persist_dirty = false;
        }
    }

    fn activate_next_system_sound(&mut self) {
        while let Some(request) = self.system_queue.pop() {
            match system_assets::resolve(request.sound) {
                Ok(wav) => {
                    serial_println!(
                        "[AUDIOD] system-sound started id={} name={} theme={} frames={}",
                        request.sound as u16,
                        request.sound.label(),
                        system_assets::THEME_NAME,
                        wav.pcm.len() / 4
                    );
                    self.active_system_sound = Some(ActiveSystemSound {
                        request,
                        pcm: wav.pcm,
                        offset: 0,
                        submitted: false,
                    });
                    return;
                }
                Err(_) => {
                    let bit = 1u16 << request.sound.index();
                    if self.bad_asset_mask & bit == 0 {
                        let asset = system_assets::asset_for(request.sound);
                        serial_println!(
                            "[AUDIOD] system-sound asset invalid id={} name={}",
                            request.sound as u16,
                            asset.canonical_name
                        );
                        self.bad_asset_mask |= bit;
                    }
                }
            }
        }
    }

    fn stop_automatic_system_sounds(&mut self) {
        self.system_queue.remove_automatic();
        if self
            .active_system_sound
            .as_ref()
            .map(|active| active.request.mode == SystemSoundMode::Automatic)
            .unwrap_or(false)
        {
            self.active_system_sound = None;
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[AUDIOD] PANIC: {}", info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[AUDIOD] sunlight-audiod starting");
    let ep = endpoint_create();
    nameserver_register("audiod", ep);
    debug_log("[AUDIOD] registered as 'audiod'");

    let mut state = ServiceState::new();
    serial_println!(
        "[AUDIOD] ready volume={} muted={} device={}",
        state.volume.volume(),
        if state.volume.muted() { 1 } else { 0 },
        state.kind.as_str()
    );
    serial_println!(
        "[AUDIOD] system-sounds theme={} enabled={} volume={} queue_cap={}",
        system_assets::THEME_NAME,
        state.system_settings.enabled as u8,
        state.system_settings.volume,
        sunlight_audiod::SYSTEM_SOUND_QUEUE_CAPACITY
    );
    state.last_state = state.state();
    if option_env!("SUNLIGHT_INJECT_AUDIO_TEST") == Some("1") {
        let _ = state.start_tone(DEFAULT_TONE_HZ, DEFAULT_TONE_MS);
    }

    loop {
        state.pump();
        match ipc_recv_timeout(ep, ENGINE_WAIT_MS) {
            Some(msg) => {
                let status_diag = msg.label == AudiodMsg::GET_STREAM_STATUS
                    && state.status_diag_count < 16;
                let pcm_diag = msg.label == AudiodMsg::SUBMIT_PCM && state.pcm_diag_count < 4;
                if status_diag {
                    serial_println!(
                        "[AUDIOD][status-request] sender_pid={} endpoint={} opcode={:#x} request_words={} status_request_bytes={}",
                        msg.badge,
                        ep.0,
                        msg.label,
                        msg.word_count,
                        msg.word_count.saturating_mul(8)
                    );
                    state.status_diag_count += 1;
                }
                if pcm_diag {
                    serial_println!(
                        "[AUDIOD][pcm-request] sender_pid={} endpoint={} opcode={:#x} request_words={} request_caps={} pcm_bytes={}",
                        msg.badge,
                        ep.0,
                        msg.label,
                        msg.word_count,
                        msg.cap_count,
                        msg.words[0]
                    );
                    state.pcm_diag_count += 1;
                }
                let reply = handle_msg(&mut state, &msg);
                let reply_result = ipc_reply_result(reply);
                if status_diag {
                    serial_println!(
                        "[AUDIOD][status-reply] sender_pid={} result={:?} reply_tag={:#x} status_reply_bytes={} expected_status_reply_bytes=32 decode_shape={}",
                        msg.badge,
                        reply_result,
                        reply.label,
                        reply.word_count.saturating_mul(8),
                        if reply.label == AudiodMsg::REPLY && reply.word_count == 4 { "ok" } else { "invalid" }
                    );
                }
                if pcm_diag {
                    serial_println!(
                        "[AUDIOD][pcm-reply] sender_pid={} result={:?} reply_tag={:#x} reply_bytes={} expected_reply_bytes=32",
                        msg.badge,
                        reply_result,
                        reply.label,
                        reply.word_count.saturating_mul(8)
                    );
                }
                state.persist_if_dirty();
            }
            None => {}
        }
        let now_state = state.state();
        if now_state != state.last_state {
            serial_println!("[AUDIOD] state {}", now_state.as_str());
            state.last_state = now_state;
        }
    }
}

fn handle_msg(state: &mut ServiceState, msg: &IpcMsg) -> IpcMsg {
    match msg.label {
        AudiodMsg::GET_STATUS | AudiodMsg::GET_VOLUME => pack_audio_status(state.status()),
        AudiodMsg::GET_SYSTEM_SOUNDS => pack_audio_status(state.status()),
        AudiodMsg::GET_STREAM_STATUS => {
            let underruns = state.device.as_ref().map(|dev| dev.underruns()).unwrap_or(0);
            pack_audio_stream_status(state.stream_progress.status(msg.badge, underruns))
        }
        AudiodMsg::GET_DEVICE => IpcMsg::with_label(AudiodMsg::REPLY)
            .word(0, state.kind as u64)
            .word(1, state.vendor_id as u64 | ((state.device_id as u64) << 16))
            .word(2, state.state() as u64),
        AudiodMsg::SET_VOLUME => {
            if msg.words[0] > 100 {
                return err(AudiodMsg::ERR_BAD_REQUEST);
            }
            state.volume.set_volume(msg.words[0] as u8);
            state.persist_dirty = true;
            pack_audio_status(state.status())
        }
        AudiodMsg::SET_MUTE => {
            if msg.words[0] > 1 {
                return err(AudiodMsg::ERR_BAD_REQUEST);
            }
            state.volume.set_muted(msg.words[0] != 0);
            state.persist_dirty = true;
            pack_audio_status(state.status())
        }
        AudiodMsg::SET_SYSTEM_SOUNDS_ENABLED => {
            if msg.word_count != 1 || msg.words[0] > 1 {
                return err(AudiodMsg::ERR_BAD_REQUEST);
            }
            state.system_settings.enabled = msg.words[0] != 0;
            if !state.system_settings.enabled {
                state.stop_automatic_system_sounds();
            }
            state.persist_dirty = true;
            pack_audio_status(state.status())
        }
        AudiodMsg::SET_SYSTEM_SOUNDS_VOLUME => {
            if msg.word_count != 1 || msg.words[0] > 100 {
                return err(AudiodMsg::ERR_BAD_REQUEST);
            }
            state.system_settings.volume = msg.words[0] as u8;
            state.persist_dirty = true;
            pack_audio_status(state.status())
        }
        AudiodMsg::PLAY_TONE => {
            if state.device.is_none() {
                return err(AudiodMsg::ERR_UNAVAILABLE);
            }
            let freq = if msg.words[0] == 0 {
                DEFAULT_TONE_HZ
            } else if msg.words[0] > 8_000 {
                return err(AudiodMsg::ERR_BAD_REQUEST);
            } else {
                msg.words[0] as u32
            };
            let ms = if msg.words[1] == 0 {
                DEFAULT_TONE_MS
            } else if msg.words[1] > 5_000 {
                return err(AudiodMsg::ERR_BAD_REQUEST);
            } else {
                msg.words[1] as u32
            };
            let _ = state.start_tone(freq, ms);
            pack_audio_status(state.status())
        }
        AudiodMsg::SUBMIT_PCM => submit_pcm(state, msg),
        AudiodMsg::PLAY_SYSTEM_SOUND => {
            if state.device.is_none() {
                return err(AudiodMsg::ERR_UNAVAILABLE);
            }
            let request = match decode_system_sound_request(msg) {
                Ok(request) => request,
                Err(_) => return err(AudiodMsg::ERR_BAD_REQUEST),
            };
            let outcome = state.system_queue.enqueue(
                request,
                state
                    .active_system_sound
                    .as_ref()
                    .map(|active| active.request),
                monotonic_millis(),
                state.system_settings,
                state.volume.effective(),
            );
            if outcome == SystemSoundEnqueue::QueueFull {
                err(AudiodMsg::ERR_OVERFLOW)
            } else {
                pack_audio_status(state.status())
            }
        }
        AudiodMsg::STOP => {
            if let Some(device) = state.device.as_mut() {
                let progress = match device.flush_with_progress() {
                    Ok(progress) => progress,
                    Err(_) => return err(AudiodMsg::ERR_DEVICE_FAILED),
                };
                state.stream_progress.observe_dma(
                    progress.first_completed_period as usize,
                    progress.completed_periods as usize,
                    progress.current_period as usize,
                    progress.current_period_frames,
                );
            }
            let underruns = state.device.as_ref().map(|dev| dev.underruns()).unwrap_or(0);
            let stopped = state.stream_progress.status(msg.badge, underruns);
            state.tone_frames_left = 0;
            state.queue.clear();
            state.queue_owner = None;
            state.system_queue.clear();
            state.active_system_sound = None;
            state.stream_progress.reset();
            pack_audio_stream_status(stopped)
        }
        _ => err(AudiodMsg::ERR_BAD_REQUEST),
    }
}

fn submit_pcm(state: &mut ServiceState, msg: &IpcMsg) -> IpcMsg {
    if state.device.is_none() {
        return err(AudiodMsg::ERR_UNAVAILABLE);
    }
    let len = msg.words[0] as usize;
    if len == 0 || len > MAX_PCM_BYTES {
        return err(AudiodMsg::ERR_OVERFLOW);
    }
    if msg.cap_count == 0 {
        return err(AudiodMsg::ERR_BAD_REQUEST);
    }
    if state.queue_owner.is_some() && state.queue_owner != Some(msg.badge) {
        return err(AudiodMsg::ERR_OVERFLOW);
    }
    let Ok(ptr) = shm_map(msg.caps[0]) else {
        return err(AudiodMsg::ERR_BAD_REQUEST);
    };
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len.min(4096)) };
    let validation = validate_pcm(AudioBuffer {
        bytes,
        format: AudioFormat::NATIVE,
    });
    match validation {
        sunlight_audio::PcmValidation::Err(AudioError::UnsupportedFormat) => {
            let _ = shm_free(msg.caps[0]);
            return err(AudiodMsg::ERR_INVALID_FORMAT);
        }
        sunlight_audio::PcmValidation::Err(_) => {
            let _ = shm_free(msg.caps[0]);
            return err(AudiodMsg::ERR_OVERFLOW);
        }
        sunlight_audio::PcmValidation::Ok { .. } => {}
    }
    // PcmQueue owns a copy. Drop audiod's received mapping now so every
    // submission does not permanently consume a shared-memory mapping.
    let queued = state.queue.push(bytes);
    let _ = shm_free(msg.caps[0]);
    match queued {
        Ok(()) => {
            state.queue_owner = Some(msg.badge);
            if !state.stream_progress.begin_submission(msg.badge, len as u64 / 4) {
                state.queue.clear();
                state.queue_owner = None;
                return err(AudiodMsg::ERR_OVERFLOW);
            }
            let underruns = state.device.as_ref().map(|dev| dev.underruns()).unwrap_or(0);
            pack_audio_stream_status(state.stream_progress.status(msg.badge, underruns))
        }
        Err(_) => err(AudiodMsg::ERR_OVERFLOW),
    }
}

fn err(code: u64) -> IpcMsg {
    IpcMsg::with_label(AudiodMsg::ERROR).word(0, code)
}

fn load_config_text() -> Option<heapless::String<256>> {
    let fd = libc::open(CONFIG_PATH.as_bytes()).ok()?;
    let mut raw = [0u8; 256];
    let n = match libc::read(fd, &mut raw) {
        Ok(n) => n,
        Err(_) => {
            let _ = libc::close(fd);
            return None;
        }
    };
    let _ = libc::close(fd);
    let text = core::str::from_utf8(&raw[..n]).ok()?;
    let mut out = heapless::String::<256>::new();
    let _ = out.push_str(text);
    Some(out)
}

fn save_config(cfg: PersistedAudio) -> bool {
    if libc::mkdir_recursive(b"/root/.config/sunlight").is_err() {
        return false;
    }
    let rendered = render_persisted_buf(cfg);
    let fd = match libc::open_with_flags(
        CONFIG_TMP_PATH.as_bytes(),
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
    ) {
        Ok(fd) => fd,
        Err(_) => return false,
    };
    let ok = libc::write_all(fd, rendered.as_bytes()).is_ok();
    let _ = libc::close(fd);
    if !ok {
        return false;
    }
    libc::rename(CONFIG_TMP_PATH.as_bytes(), CONFIG_PATH.as_bytes()).is_ok()
}
