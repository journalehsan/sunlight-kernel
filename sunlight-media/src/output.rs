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
    origin_frames: u32,
    submitted_frames: u64,
    timeline_frames: u64,
    volume: u8,
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
            origin_frames: snapshot.frames_played,
            submitted_frames: 0,
            timeline_frames: 0,
            volume: volume.min(100),
        })
    }

    fn consumed_from(&self, absolute: u32) -> u64 {
        absolute.wrapping_sub(self.origin_frames) as u64
    }

    fn consumed_session_frames(&self) -> Result<u64, MediaError> {
        self.client
            .snapshot()
            .map(|snapshot| {
                self.consumed_from(snapshot.frames_played)
                    .min(self.submitted_frames)
            })
            .map_err(|_| MediaError::new(MediaErrorKind::AudioOutput, 3))
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
        loop {
            match self.client.submit_pcm(&gained[..pcm.len()]) {
                Ok(_) => break,
                Err(sunlight_audiod::AudioClientError::Overflow) => sunlight_ipc::process_yield(),
                Err(_) => return Err(MediaError::new(MediaErrorKind::AudioOutput, 5)),
            }
        }
        self.submitted_frames = self.submitted_frames.saturating_add((pcm.len() / 4) as u64);

        // Keep at most the HDA ring plus one producer period outstanding.
        // This bounds latency without starving the four-period hardware ring.
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
                return Ok(self.timeline_frames.saturating_add(consumed));
            }
            sunlight_ipc::process_yield();
        }
    }

    fn flush(&mut self) -> Result<(), MediaError> {
        let snapshot = self
            .client
            .stop()
            .map_err(|_| MediaError::new(MediaErrorKind::AudioOutput, 6))?;
        let consumed = self
            .consumed_from(snapshot.frames_played)
            .min(self.submitted_frames);
        self.timeline_frames = self.timeline_frames.saturating_add(consumed);
        self.origin_frames = snapshot.frames_played;
        self.submitted_frames = 0;
        Ok(())
    }

    fn set_position_frames(&mut self, frames: u64) {
        self.timeline_frames = frames;
        self.submitted_frames = 0;
    }

    fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
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
