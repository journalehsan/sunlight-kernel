//! PCM output boundary. Decoder code never imports audiod or hardware types.

use crate::error::{MediaError, MediaErrorKind};

pub const PERIOD_FRAMES: usize = sunlight_audio::hda::PERIOD_FRAME_COUNT;
pub const STARTUP_PERIODS: usize = 5;

/// Collect decoder output into hardware-sized writes. A short write is only
/// allowed at end of stream, where audiod pads the final DMA period.
pub struct PcmPacketizer {
    bytes: [u8; sunlight_ipc::SHM_PAGE],
    frames: usize,
}

impl PcmPacketizer {
    pub const fn new() -> Self {
        Self {
            bytes: [0; sunlight_ipc::SHM_PAGE],
            frames: 0,
        }
    }

    pub fn clear(&mut self) {
        self.frames = 0;
    }

    pub fn push(
        &mut self,
        frame: [i16; 2],
        sink: &mut impl AudioSink,
    ) -> Result<Option<u64>, MediaError> {
        let offset = self.frames * 4;
        self.bytes[offset..offset + 2].copy_from_slice(&frame[0].to_le_bytes());
        self.bytes[offset + 2..offset + 4].copy_from_slice(&frame[1].to_le_bytes());
        self.frames += 1;
        if self.frames == PERIOD_FRAMES {
            self.frames = 0;
            return sink.write(&self.bytes).map(Some);
        }
        Ok(None)
    }

    pub fn finish(&mut self, sink: &mut impl AudioSink) -> Result<Option<u64>, MediaError> {
        if self.frames == 0 {
            return Ok(None);
        }
        let len = self.frames * 4;
        self.frames = 0;
        sink.write(&self.bytes[..len]).map(Some)
    }
}

pub trait AudioSink {
    /// Submit PCM and return the number of session frames consumed by the
    /// hardware clock so far.
    fn write(&mut self, pcm_s16le_stereo: &[u8]) -> Result<u64, MediaError>;
    fn position_frames(&mut self) -> Result<u64, MediaError>;
    fn drain(&mut self) -> Result<u64, MediaError>;
    fn flush(&mut self) -> Result<(), MediaError>;
    /// Rebase the media clock after an explicit decoder seek or Stop.
    fn set_position_frames(&mut self, frames: u64);
    fn set_volume(&mut self, volume: u8);
}

pub struct SunlightAudioSink {
    client: sunlight_audiod::AudioClient,
    startup: [[u8; sunlight_ipc::SHM_PAGE]; STARTUP_PERIODS],
    startup_lengths: [usize; STARTUP_PERIODS],
    startup_count: usize,
    started: bool,
    submitted_frames: u64,
    timeline_frames: u64,
    volume: u8,
    next_progress_log_frames: u64,
    playback_started_ms: Option<u64>,
    last_submit_finished_ms: Option<u64>,
    submit_max_ms: u64,
    producer_gap_max_ms: u64,
}

impl SunlightAudioSink {
    pub fn open(volume: u8) -> Result<Self, MediaError> {
        let client = sunlight_audiod::AudioClient::new();
        let snapshot = client
            .snapshot()
            .map_err(|_| MediaError::new(MediaErrorKind::AudioOutput, 1))?;
        if !snapshot.available()
            || snapshot.sample_rate_hz != sunlight_audio::NATIVE_RATE_HZ
            || snapshot.channels != 2
            || snapshot.bits != 16
        {
            return Err(MediaError::new(MediaErrorKind::AudioOutput, 2));
        }
        Ok(Self {
            client,
            startup: [[0; sunlight_ipc::SHM_PAGE]; STARTUP_PERIODS],
            startup_lengths: [0; STARTUP_PERIODS],
            startup_count: 0,
            started: false,
            submitted_frames: 0,
            timeline_frames: 0,
            volume: volume.min(100),
            next_progress_log_frames: 48_000,
            playback_started_ms: None,
            last_submit_finished_ms: None,
            submit_max_ms: 0,
            producer_gap_max_ms: 0,
        })
    }

    fn consumed_session_frames(&self) -> Result<u64, MediaError> {
        for _ in 0..8 {
            match self.client.stream_status() {
                Ok(status) => {
                    return Ok(status.consumed_frames.min(self.submitted_frames));
                }
                Err(sunlight_audiod::AudioClientError::Overflow)
                | Err(sunlight_audiod::AudioClientError::Timeout) => {
                    sunlight_ipc::process_yield();
                }
                Err(error) => {
                    return Err(MediaError::new(
                        MediaErrorKind::AudioOutput,
                        status_error_detail(error),
                    ));
                }
            }
        }
        Err(MediaError::new(MediaErrorKind::AudioOutput, 3))
    }

    fn snapshot_position(&self) -> Result<u64, MediaError> {
        self.consumed_session_frames()
            .map(|frames| self.timeline_frames.saturating_add(frames))
    }

    fn submit_now(&mut self, pcm: &[u8]) -> Result<u64, MediaError> {
        let started_ms = sunlight_ipc::monotonic_millis();
        let playback_started_ms = *self.playback_started_ms.get_or_insert(started_ms);
        if let Some(previous) = self.last_submit_finished_ms {
            self.producer_gap_max_ms = self
                .producer_gap_max_ms
                .max(started_ms.saturating_sub(previous));
        }
        let mut gained = [0u8; sunlight_ipc::SHM_PAGE];
        gained[..pcm.len()].copy_from_slice(pcm);
        sunlight_audio::pcm::apply_gain_s16le(&mut gained[..pcm.len()], self.volume);
        let status = loop {
            match self.client.submit_pcm_chunk(&gained[..pcm.len()]) {
                Ok(status) => break status,
                Err(sunlight_audiod::AudioClientError::Overflow) => sunlight_ipc::process_yield(),
                Err(error) => {
                    return Err(MediaError::new(
                        MediaErrorKind::AudioOutput,
                        submit_error_detail(error),
                    ))
                }
            }
        };
        let finished_ms = sunlight_ipc::monotonic_millis();
        self.last_submit_finished_ms = Some(finished_ms);
        self.submit_max_ms = self
            .submit_max_ms
            .max(finished_ms.saturating_sub(started_ms));
        self.submitted_frames = self.submitted_frames.saturating_add((pcm.len() / 4) as u64);
        if self.submitted_frames >= self.next_progress_log_frames {
            log_playback_progress(
                self.submitted_frames,
                status.consumed_frames,
                status.buffered_frames,
                status.underruns,
                self.timeline_frames.saturating_add(status.consumed_frames),
                pcm.len() / 4,
                finished_ms.saturating_sub(playback_started_ms),
                self.submit_max_ms,
                self.producer_gap_max_ms,
            );
            self.submit_max_ms = 0;
            self.producer_gap_max_ms = 0;
            self.next_progress_log_frames = self
                .next_progress_log_frames
                .saturating_add(sunlight_audio::NATIVE_RATE_HZ as u64);
        }

        // Keep roughly 170 ms outstanding across audiod's queue and the DMA
        // ring. This absorbs transient decode and scheduling delays in QEMU.
        let consumed = status.consumed_frames.min(self.submitted_frames);
        if self.submitted_frames.saturating_sub(consumed) <= 8 * PERIOD_FRAMES as u64 {
            return Ok(self.timeline_frames.saturating_add(consumed));
        }
        loop {
            let consumed = self.consumed_session_frames()?;
            if self.submitted_frames.saturating_sub(consumed) <= 8 * PERIOD_FRAMES as u64 {
                return Ok(self.timeline_frames.saturating_add(consumed));
            }
            sunlight_ipc::process_yield();
        }
    }

    fn submit_startup(&mut self) -> Result<u64, MediaError> {
        let mut consumed = self.timeline_frames;
        for index in 0..self.startup_count {
            let pcm = self.startup[index];
            consumed = self.submit_now(&pcm[..self.startup_lengths[index]])?;
        }
        self.startup_count = 0;
        self.started = true;
        Ok(consumed)
    }

    fn reset_timing(&mut self) {
        self.playback_started_ms = None;
        self.last_submit_finished_ms = None;
        self.submit_max_ms = 0;
        self.producer_gap_max_ms = 0;
    }
}

impl AudioSink for SunlightAudioSink {
    fn write(&mut self, pcm: &[u8]) -> Result<u64, MediaError> {
        if pcm.is_empty() || pcm.len() > sunlight_ipc::SHM_PAGE || pcm.len() % 4 != 0 {
            return Err(MediaError::new(MediaErrorKind::AudioOutput, 4));
        }
        if !self.started {
            let index = self.startup_count;
            self.startup[index][..pcm.len()].copy_from_slice(pcm);
            self.startup_lengths[index] = pcm.len();
            self.startup_count += 1;
            if self.startup_count == STARTUP_PERIODS {
                return self.submit_startup();
            }
            return Ok(self.timeline_frames);
        }
        self.submit_now(pcm)
    }

    fn position_frames(&mut self) -> Result<u64, MediaError> {
        self.snapshot_position()
    }

    fn drain(&mut self) -> Result<u64, MediaError> {
        if !self.started {
            self.submit_startup()?;
        }
        loop {
            let consumed = self.consumed_session_frames()?;
            if consumed >= self.submitted_frames {
                let status = self.client.stop_stream().map_err(|error| {
                    MediaError::new(MediaErrorKind::AudioOutput, stop_error_detail(error))
                })?;
                let consumed = status.consumed_frames.min(self.submitted_frames);
                self.timeline_frames = self.timeline_frames.saturating_add(consumed);
                self.submitted_frames = 0;
                self.next_progress_log_frames = sunlight_audio::NATIVE_RATE_HZ as u64;
                self.started = false;
                self.reset_timing();
                return Ok(self.timeline_frames);
            }
            sunlight_ipc::process_yield();
        }
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        self.startup_count = 0;
        self.started = false;
        let status = self.client.stop_stream().map_err(|error| {
            MediaError::new(MediaErrorKind::AudioOutput, stop_error_detail(error))
        })?;
        let consumed = status.consumed_frames.min(self.submitted_frames);
        self.timeline_frames = self.timeline_frames.saturating_add(consumed);
        self.submitted_frames = 0;
        self.next_progress_log_frames = sunlight_audio::NATIVE_RATE_HZ as u64;
        self.reset_timing();
        Ok(())
    }

    fn set_position_frames(&mut self, frames: u64) {
        self.startup_count = 0;
        self.started = false;
        self.timeline_frames = frames;
        self.submitted_frames = 0;
        self.next_progress_log_frames = sunlight_audio::NATIVE_RATE_HZ as u64;
        self.reset_timing();
    }

    fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
    }
}

#[cfg(target_os = "none")]
fn log_playback_progress(
    submitted_frames: u64,
    consumed_frames: u64,
    buffered_frames: u32,
    underruns: u32,
    position_frames: u64,
    write_frames: usize,
    elapsed_ms: u64,
    submit_max_ms: u64,
    producer_gap_max_ms: u64,
) {
    use core::fmt::Write;

    struct LogLine {
        bytes: [u8; 256],
        len: usize,
    }

    impl Write for LogLine {
        fn write_str(&mut self, value: &str) -> core::fmt::Result {
            let end = self.len + value.len();
            if end > self.bytes.len() {
                return Err(core::fmt::Error);
            }
            self.bytes[self.len..end].copy_from_slice(value.as_bytes());
            self.len = end;
            Ok(())
        }
    }

    let mut line = LogLine {
        bytes: [0; 256],
        len: 0,
    };
    let _ = write!(
        line,
        "[MEDIA][playback] submitted_frames={} consumed_frames={} buffered_frames={} buffer_ms={} write_frames={} underruns={} position_ms={}",
        submitted_frames,
        consumed_frames,
        buffered_frames,
        buffered_frames as u64 * 1000 / sunlight_audio::NATIVE_RATE_HZ as u64,
        write_frames,
        underruns,
        position_frames.saturating_mul(1000) / sunlight_audio::NATIVE_RATE_HZ as u64,
    );
    if let Ok(message) = core::str::from_utf8(&line.bytes[..line.len]) {
        sunlight_ipc::debug_log(message);
    }
    // DebugLog accepts at most 256 bytes; keep timing in a separate bounded
    // line so large counters cannot truncate the diagnostic fields.
    line.len = 0;
    let _ = write!(
        line,
        "[MEDIA][timing] elapsed_ms={} submit_max_ms={} producer_gap_max_ms={}",
        elapsed_ms, submit_max_ms, producer_gap_max_ms,
    );
    if let Ok(message) = core::str::from_utf8(&line.bytes[..line.len]) {
        sunlight_ipc::debug_log(message);
    }
}

#[cfg(not(target_os = "none"))]
fn log_playback_progress(_: u64, _: u64, _: u32, _: u32, _: u64, _: usize, _: u64, _: u64, _: u64) {
}

fn status_error_detail(error: sunlight_audiod::AudioClientError) -> u32 {
    match error {
        sunlight_audiod::AudioClientError::ServiceUnavailable
        | sunlight_audiod::AudioClientError::Unavailable => 1,
        sunlight_audiod::AudioClientError::InvalidFormat => 4,
        sunlight_audiod::AudioClientError::DeviceFailed => 6,
        sunlight_audiod::AudioClientError::Timeout
        | sunlight_audiod::AudioClientError::Transport => 3,
        sunlight_audiod::AudioClientError::BadRequest
        | sunlight_audiod::AudioClientError::Overflow => 5,
    }
}

fn submit_error_detail(error: sunlight_audiod::AudioClientError) -> u32 {
    match error {
        sunlight_audiod::AudioClientError::ServiceUnavailable
        | sunlight_audiod::AudioClientError::Unavailable => 1,
        sunlight_audiod::AudioClientError::Timeout => 9,
        sunlight_audiod::AudioClientError::Transport => 4,
        sunlight_audiod::AudioClientError::InvalidFormat => 5,
        sunlight_audiod::AudioClientError::DeviceFailed => 6,
        sunlight_audiod::AudioClientError::BadRequest => 7,
        sunlight_audiod::AudioClientError::Overflow => 8,
    }
}

fn stop_error_detail(error: sunlight_audiod::AudioClientError) -> u32 {
    match error {
        sunlight_audiod::AudioClientError::ServiceUnavailable
        | sunlight_audiod::AudioClientError::Unavailable => 1,
        sunlight_audiod::AudioClientError::DeviceFailed => 6,
        sunlight_audiod::AudioClientError::Timeout
        | sunlight_audiod::AudioClientError::Transport => 6,
        sunlight_audiod::AudioClientError::InvalidFormat
        | sunlight_audiod::AudioClientError::BadRequest
        | sunlight_audiod::AudioClientError::Overflow => 6,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn submit_timeout_is_distinct_from_status_failure() {
        let timeout = sunlight_audiod::AudioClientError::Timeout;
        let submit = MediaError::new(MediaErrorKind::AudioOutput, submit_error_detail(timeout));
        let status = MediaError::new(MediaErrorKind::AudioOutput, status_error_detail(timeout));
        assert_ne!(submit.detail, status.detail);
        assert_eq!(submit.user_message(), "Audio PCM submission timed out");
        assert_eq!(
            status.user_message(),
            "Audio output status could not be read"
        );
    }

    pub struct FakeSink {
        pub written: Vec<u8>,
        pub consumed: u64,
        pub fail: bool,
        pub volume: u8,
    }

    impl FakeSink {
        pub fn new() -> Self {
            Self {
                written: Vec::new(),
                consumed: 0,
                fail: false,
                volume: 100,
            }
        }
    }

    impl AudioSink for FakeSink {
        fn write(&mut self, pcm: &[u8]) -> Result<u64, MediaError> {
            if self.fail {
                return Err(MediaError::new(MediaErrorKind::AudioOutput, 99));
            }
            self.written.extend_from_slice(pcm);
            self.consumed += (pcm.len() / 4) as u64;
            Ok(self.consumed)
        }

        fn position_frames(&mut self) -> Result<u64, MediaError> {
            Ok(self.consumed)
        }

        fn drain(&mut self) -> Result<u64, MediaError> {
            Ok(self.consumed)
        }

        fn flush(&mut self) -> Result<(), MediaError> {
            self.written.clear();
            self.consumed = 0;
            Ok(())
        }

        fn set_position_frames(&mut self, frames: u64) {
            self.consumed = frames;
        }

        fn set_volume(&mut self, volume: u8) {
            self.volume = volume.min(100);
        }
    }

    #[test]
    fn fake_sink_is_deterministic_flushable_and_reports_errors() {
        let mut sink = FakeSink::new();
        assert_eq!(sink.write(&[1; 16]).unwrap(), 4);
        assert_eq!(sink.position_frames().unwrap(), 4);
        sink.set_volume(250);
        assert_eq!(sink.volume, 100);
        sink.flush().unwrap();
        assert!(sink.written.is_empty());
        assert_eq!(sink.consumed, 0);
        sink.set_position_frames(12);
        assert_eq!(sink.position_frames().unwrap(), 12);
        sink.fail = true;
        assert_eq!(
            sink.write(&[0; 4]).unwrap_err().kind,
            MediaErrorKind::AudioOutput
        );
    }

    #[test]
    fn irregular_decoder_chunks_submit_only_complete_periods_until_eof() {
        let mut sink = FakeSink::new();
        let mut packetizer = PcmPacketizer::new();
        for _ in 0..(PERIOD_FRAMES * 2 + 17) {
            packetizer.push([123, -123], &mut sink).unwrap();
        }
        assert_eq!(sink.consumed, (PERIOD_FRAMES * 2) as u64);
        assert_eq!(sink.written.len(), PERIOD_FRAMES * 2 * 4);
        packetizer.finish(&mut sink).unwrap();
        assert_eq!(sink.consumed, (PERIOD_FRAMES * 2 + 17) as u64);
    }

    #[test]
    fn resampled_music_reaches_sink_with_native_frame_count() {
        let mut sink = FakeSink::new();
        let mut packetizer = PcmPacketizer::new();
        let mut resampler = crate::resampler::LinearResampler::new(44_100, 48_000);
        let input = [700i16; 4_410];
        for chunk in input.chunks(127) {
            resampler
                .process(chunk, 1, |frame| {
                    packetizer.push(frame, &mut sink)?;
                    Ok(())
                })
                .unwrap();
        }
        resampler
            .finish(|frame| {
                packetizer.push(frame, &mut sink)?;
                Ok(())
            })
            .unwrap();
        packetizer.finish(&mut sink).unwrap();
        assert_eq!(sink.consumed, 4_800);
        assert!(sink
            .written
            .chunks_exact(4)
            .all(|frame| frame == [188, 2, 188, 2]));
    }
}
