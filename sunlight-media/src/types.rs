#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MediaTime(u64);

impl MediaTime {
    pub const ZERO: Self = Self(0);

    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }

    pub const fn from_frames(frames: u64, rate_hz: u32) -> Self {
        if rate_hz == 0 {
            Self::ZERO
        } else {
            Self(frames.saturating_mul(1000) / rate_hz as u64)
        }
    }

    pub const fn frames_at(self, rate_hz: u32) -> u64 {
        self.0.saturating_mul(rate_hz as u64) / 1000
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmFormat {
    Signed16LeInterleaved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioStreamInfo {
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub sample_format: PcmFormat,
    pub duration: Option<MediaTime>,
    pub seekable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum PlaybackState {
    #[default]
    Idle = 0,
    Loading = 1,
    Ready = 2,
    Playing = 3,
    Paused = 4,
    Ended = 5,
    Error = 6,
}

impl PlaybackState {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Loading,
            2 => Self::Ready,
            3 => Self::Playing,
            4 => Self::Paused,
            5 => Self::Ended,
            6 => Self::Error,
            _ => Self::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_time_round_trips_native_frames_without_gui_time() {
        let time = MediaTime::from_frames(72_000, 48_000);
        assert_eq!(time.as_millis(), 1_500);
        assert_eq!(time.frames_at(48_000), 72_000);
    }
}
