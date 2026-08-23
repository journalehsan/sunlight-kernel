//! PCM output boundary. Decoder code never imports audiod or hardware types.

use crate::error::{MediaError, MediaErrorKind};

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
    submitted_frames: u64,
    timeline_frames: u64,
    volume: u8,
    next_progress_log_frames: u64,
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
            submitted_frames: 0,
            timeline_frames: 0,
            volume: volume.min(100),
            next_progress_log_frames: 48_000,
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
}

impl AudioSink for SunlightAudioSink {
    fn write(&mut self, pcm: &[u8]) -> Result<u64, MediaError> {
        if pcm.is_empty() || pcm.len() > sunlight_ipc::SHM_PAGE || pcm.len() % 4 != 0 {
            return Err(MediaError::new(MediaErrorKind::AudioOutput, 4));
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
        self.submitted_frames = self.submitted_frames.saturating_add((pcm.len() / 4) as u64);
        if self.submitted_frames >= self.next_progress_log_frames {
            log_playback_progress(
                self.submitted_frames,
                status.consumed_frames,
                status.buffered_frames,
                status.underruns,
                self.timeline_frames.saturating_add(status.consumed_frames),
            );
            self.next_progress_log_frames = self
                .next_progress_log_frames
                .saturating_add(sunlight_audio::NATIVE_RATE_HZ as u64);
        }

        // Keep at most the HDA ring plus one producer period outstanding.
        // This bounds latency without starving the four-period hardware ring.
        let consumed = status.consumed_frames.min(self.submitted_frames);
        if self.submitted_frames.saturating_sub(consumed) <= 5 * 1024 {
            return Ok(self.timeline_frames.saturating_add(consumed));
        }
        loop {
            let consumed = self.consumed_session_frames()?;
            if self.submitted_frames.saturating_sub(consumed) <= 5 * 1024 {
                return Ok(self.timeline_frames.saturating_add(consumed));
            }
            sunlight_ipc::process_yield();
        }
    }

    fn position_frames(&mut self) -> Result<u64, MediaError> {
        self.snapshot_position()
    }

    fn drain(&mut self) -> Result<u64, MediaError> {
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
                return Ok(self.timeline_frames);
            }
            sunlight_ipc::process_yield();
        }
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        let status = self.client.stop_stream().map_err(|error| {
            MediaError::new(MediaErrorKind::AudioOutput, stop_error_detail(error))
        })?;
        let consumed = status.consumed_frames.min(self.submitted_frames);
        self.timeline_frames = self.timeline_frames.saturating_add(consumed);
        self.submitted_frames = 0;
        self.next_progress_log_frames = sunlight_audio::NATIVE_RATE_HZ as u64;
        Ok(())
    }

    fn set_position_frames(&mut self, frames: u64) {
        self.timeline_frames = frames;
        self.submitted_frames = 0;
        self.next_progress_log_frames = sunlight_audio::NATIVE_RATE_HZ as u64;
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
) {
    sunlight_ipc::debug_log("[MEDIA][playback] submitted_frames=");
    log_u64(submitted_frames);
    sunlight_ipc::debug_log(" consumed_frames=");
    log_u64(consumed_frames);
    sunlight_ipc::debug_log(" buffered_frames=");
    log_u64(buffered_frames as u64);
    sunlight_ipc::debug_log(" underruns=");
    log_u64(underruns as u64);
    sunlight_ipc::debug_log(" position_ms=");
    log_u64(position_frames.saturating_mul(1000) / sunlight_audio::NATIVE_RATE_HZ as u64);
    sunlight_ipc::debug_log("\n");
}

#[cfg(not(target_os = "none"))]
fn log_playback_progress(_: u64, _: u64, _: u32, _: u32, _: u64) {}

#[cfg(target_os = "none")]
fn log_u64(mut value: u64) {
    if value == 0 {
        sunlight_ipc::debug_log("0");
        return;
    }
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    while value != 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    digits[..len].reverse();
    if let Ok(text) = core::str::from_utf8(&digits[..len]) {
        sunlight_ipc::debug_log(text);
    }
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
        sunlight_audiod::AudioClientError::Timeout => 3,
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
}
