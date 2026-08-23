//! Presentation state consumed by Melody Mina.
//!
//! Authoritative playback values arrive from `sunlight-media`; pointer/drag
//! state stays separate so a refresh cannot fight an active seek gesture.

use sunlight_media::{MediaError, MediaSnapshot, PlaybackState as BackendState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackState {
    #[default]
    Idle,
    Loading,
    Ready,
    Playing,
    Paused,
    Ended,
    Error,
}

impl From<BackendState> for PlaybackState {
    fn from(state: BackendState) -> Self {
        match state {
            BackendState::Idle => Self::Idle,
            BackendState::Loading => Self::Loading,
            BackendState::Ready => Self::Ready,
            BackendState::Playing => Self::Playing,
            BackendState::Paused => Self::Paused,
            BackendState::Ended => Self::Ended,
            BackendState::Error => Self::Error,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlAvailability {
    pub open: bool,
    pub play_pause: bool,
    pub stop: bool,
    pub seek: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NowPlayingViewModel {
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub seekable: bool,
    pub playback_state: PlaybackState,
    pub volume: u8,
    pub error: Option<MediaError>,
}

impl Default for NowPlayingViewModel {
    fn default() -> Self {
        Self {
            position_ms: 0,
            duration_ms: None,
            seekable: false,
            playback_state: PlaybackState::Idle,
            volume: 68,
            error: None,
        }
    }
}

impl NowPlayingViewModel {
    pub fn apply_backend(&mut self, snapshot: MediaSnapshot, suppress_position_update: bool) {
        self.playback_state = snapshot.state.into();
        self.duration_ms = snapshot
            .stream
            .and_then(|stream| stream.duration)
            .map(|v| v.as_millis());
        self.seekable = snapshot
            .stream
            .map(|stream| stream.seekable)
            .unwrap_or(false);
        self.volume = snapshot.volume.min(100);
        self.error = snapshot.error;
        if !suppress_position_update {
            self.position_ms = snapshot.position.as_millis();
        }
        if snapshot.state == BackendState::Ended && !suppress_position_update {
            if let Some(duration) = self.duration_ms {
                self.position_ms = duration;
            }
        } else if matches!(snapshot.state, BackendState::Idle | BackendState::Loading) {
            self.position_ms = 0;
        }
    }

    pub const fn controls(self) -> ControlAvailability {
        let loaded = matches!(
            self.playback_state,
            PlaybackState::Ready
                | PlaybackState::Playing
                | PlaybackState::Paused
                | PlaybackState::Ended
        );
        ControlAvailability {
            open: !matches!(self.playback_state, PlaybackState::Loading),
            play_pause: loaded,
            stop: matches!(
                self.playback_state,
                PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Ended
            ),
            seek: loaded && self.seekable && matches!(self.duration_ms, Some(value) if value > 0),
        }
    }

    pub const fn shows_pause(self) -> bool {
        matches!(self.playback_state, PlaybackState::Playing)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractionState {
    pub seek_drag_active: bool,
    pub seek_preview_percent: u32,
    pub seek_commit_pending: bool,
    pub seek_target_ms: u64,
}

pub fn timeline_percent(model: &NowPlayingViewModel) -> u32 {
    match model.duration_ms {
        Some(duration) if model.seekable && duration != 0 => {
            (model.position_ms.min(duration).saturating_mul(100) / duration) as u32
        }
        _ => 0,
    }
}

pub fn seek_target_ms(model: &NowPlayingViewModel, percent: u32) -> Option<u64> {
    match model.duration_ms {
        Some(duration) if model.controls().seek => {
            Some(duration.saturating_mul(percent.min(100) as u64) / 100)
        }
        _ => None,
    }
}

pub fn next_playlist_index(current: usize, len: usize, delta: isize) -> Option<usize> {
    if len == 0 || delta == 0 {
        return (len != 0).then_some(current.min(len.saturating_sub(1)));
    }
    let current = current.min(len - 1) as isize;
    Some(if delta.is_negative() {
        let amount = delta.unsigned_abs() % len;
        (current - amount as isize).rem_euclid(len as isize) as usize
    } else {
        (current + (delta as usize % len) as isize).rem_euclid(len as isize) as usize
    })
}

/// Formats whole seconds as `m:ss` or `h:mm:ss` without allocation.
pub fn format_time(seconds: u64, output: &mut [u8; 24]) -> &str {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    let mut index = 0;
    if hours != 0 {
        index += write_decimal(hours, &mut output[index..]);
        output[index] = b':';
        index += 1;
        output[index] = b'0' + (minutes / 10) as u8;
        output[index + 1] = b'0' + (minutes % 10) as u8;
        index += 2;
    } else {
        index += write_decimal(minutes, &mut output[index..]);
    }
    output[index] = b':';
    output[index + 1] = b'0' + (secs / 10) as u8;
    output[index + 2] = b'0' + (secs % 10) as u8;
    index += 3;
    core::str::from_utf8(&output[..index]).unwrap_or("0:00")
}

fn write_decimal(mut value: u64, output: &mut [u8]) -> usize {
    let mut reversed = [0u8; 20];
    let mut len = 0;
    loop {
        reversed[len] = b'0' + (value % 10) as u8;
        len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for index in 0..len {
        output[index] = reversed[len - index - 1];
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_media::{
        AudioStreamInfo, MediaError, MediaErrorKind, MediaTime, PcmFormat, VisualizationFrame,
    };

    fn snapshot(state: BackendState, position_ms: u64) -> MediaSnapshot {
        MediaSnapshot {
            generation: 1,
            state,
            position: MediaTime::from_millis(position_ms),
            stream: Some(AudioStreamInfo {
                sample_rate_hz: 48_000,
                channels: 2,
                sample_format: PcmFormat::Signed16LeInterleaved,
                duration: Some(MediaTime::from_millis(6_000)),
                seekable: true,
            }),
            volume: 72,
            error: None,
            visualization: VisualizationFrame::empty(),
        }
    }

    #[test]
    fn maps_authoritative_states_and_errors() {
        let mut view = NowPlayingViewModel::default();
        view.apply_backend(snapshot(BackendState::Playing, 3_000), false);
        assert!(view.shows_pause());
        view.apply_backend(snapshot(BackendState::Paused, 3_000), false);
        assert!(!view.shows_pause());
        let mut failed = snapshot(BackendState::Error, 0);
        failed.error = Some(MediaError::new(MediaErrorKind::Decode, 7));
        view.apply_backend(failed, false);
        assert_eq!(
            view.error.map(|error| error.kind),
            Some(MediaErrorKind::Decode)
        );
        view.apply_backend(snapshot(BackendState::Ended, 5_999), false);
        assert_eq!(view.position_ms, 6_000);
        assert!(!view.shows_pause());
    }

    #[test]
    fn formats_minutes_hours_and_multi_hour_values() {
        let cases = [
            (0, "0:00"),
            (3, "0:03"),
            (6, "0:06"),
            (3_599, "59:59"),
            (3_600, "1:00:00"),
            (45_296, "12:34:56"),
        ];
        for (seconds, expected) in cases {
            let mut output = [0; 24];
            assert_eq!(format_time(seconds, &mut output), expected);
        }
    }

    #[test]
    fn seeking_clamps_and_drag_blocks_backend_position() {
        let mut view = NowPlayingViewModel::default();
        view.apply_backend(snapshot(BackendState::Playing, 3_000), false);
        assert_eq!(timeline_percent(&view), 50);
        assert_eq!(seek_target_ms(&view, 250), Some(6_000));
        view.apply_backend(snapshot(BackendState::Playing, 5_000), true);
        assert_eq!(view.position_ms, 3_000);
        view.seekable = false;
        assert_eq!(seek_target_ms(&view, 50), None);
    }

    #[test]
    fn six_second_timeline_normalizes_to_endpoints_and_halfway() {
        let mut view = NowPlayingViewModel::default();
        view.apply_backend(snapshot(BackendState::Ready, 0), false);
        assert_eq!(timeline_percent(&view), 0);
        view.apply_backend(snapshot(BackendState::Playing, 3_000), false);
        assert_eq!(timeline_percent(&view), 50);
        view.apply_backend(snapshot(BackendState::Ended, 6_000), false);
        assert_eq!(timeline_percent(&view), 100);
        assert_eq!(seek_target_ms(&view, 50), Some(3_000));
    }

    #[test]
    fn backend_volume_is_bounded_in_view_state() {
        let mut event = snapshot(BackendState::Ready, 0);
        event.volume = 255;
        let mut view = NowPlayingViewModel::default();
        view.apply_backend(event, false);
        assert_eq!(view.volume, 100);
    }

    #[test]
    fn playlist_navigation_wraps_and_handles_empty_lists() {
        assert_eq!(next_playlist_index(0, 3, -1), Some(2));
        assert_eq!(next_playlist_index(2, 3, 1), Some(0));
        assert_eq!(next_playlist_index(1, 3, 4), Some(2));
        assert_eq!(next_playlist_index(99, 3, 0), Some(2));
        assert_eq!(next_playlist_index(0, 0, 1), None);
    }
}
