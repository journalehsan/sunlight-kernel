//! Thin Melody Mina client adapter for the reusable Sunlight media API.

extern crate alloc;

use alloc::string::{String, ToString};
use sunlight_media::{
    MediaError, MediaErrorKind, MediaPlayer, MediaSnapshot, MediaTime, VisualizationFrame,
};

use crate::model::{seek_target_ms, InteractionState, NowPlayingViewModel, PlaybackState};

pub struct MelodyMediaController {
    player: MediaPlayer,
    view: NowPlayingViewModel,
    interaction: InteractionState,
    source_generation: u64,
    visualization: VisualizationFrame,
    current_path: Option<String>,
    pending_open: Option<String>,
    auto_play: bool,
    desired_playing: Option<bool>,
    transport_issued: bool,
    pending_stop: bool,
    pending_seek_ms: Option<u64>,
    seek_issued: bool,
    seek_epoch_at_issue: u64,
}

impl MelodyMediaController {
    pub fn new() -> Self {
        let player = MediaPlayer::new();
        let mut view = NowPlayingViewModel::default();
        let initial = player.snapshot();
        view.apply_backend(initial, false);
        Self {
            player,
            view,
            interaction: InteractionState::default(),
            source_generation: initial.generation,
            visualization: initial.visualization,
            current_path: None,
            pending_open: None,
            auto_play: false,
            desired_playing: None,
            transport_issued: false,
            pending_stop: false,
            pending_seek_ms: None,
            seek_issued: false,
            seek_epoch_at_issue: initial.seek_epoch,
        }
    }

    pub const fn view(&self) -> NowPlayingViewModel {
        self.view
    }

    pub const fn interaction(&self) -> InteractionState {
        self.interaction
    }

    pub const fn visualization(&self) -> VisualizationFrame {
        self.visualization
    }

    pub fn seek_enabled(&self) -> bool {
        self.view.controls().seek && !self.interaction.seek_commit_pending
    }

    pub fn shows_pause(&self) -> bool {
        self.desired_playing
            .unwrap_or(self.view.playback_state == PlaybackState::Playing)
    }

    pub fn has_pending_action(&self) -> bool {
        self.pending_open.is_some()
            || self.auto_play
            || self.desired_playing.is_some()
            || self.pending_stop
            || self.pending_seek_ms.is_some()
    }

    pub fn open(&mut self, path: &str) -> Result<(), MediaError> {
        self.open_with_autoplay(path, true)
    }

    pub fn open_paused(&mut self, path: &str) -> Result<(), MediaError> {
        self.open_with_autoplay(path, false)
    }

    fn open_with_autoplay(&mut self, path: &str, auto_play: bool) -> Result<(), MediaError> {
        if path.is_empty() || path.len() >= sunlight_libc::MAX_PATH || path.as_bytes().contains(&0)
        {
            return Err(MediaError::new(MediaErrorKind::FileOpen, 1));
        }
        if self.view.playback_state == PlaybackState::Loading
            && self.current_path.as_deref() == Some(path)
        {
            self.auto_play |= auto_play;
            return Ok(());
        }
        self.current_path = Some(path.to_string());
        self.pending_open = Some(path.to_string());
        self.auto_play = auto_play;
        self.desired_playing = None;
        self.transport_issued = false;
        self.pending_stop = false;
        self.pending_seek_ms = None;
        self.seek_issued = false;
        self.seek_epoch_at_issue = self.player.snapshot().seek_epoch;
        self.interaction = InteractionState::default();
        self.visualization = VisualizationFrame::empty();
        self.view.playback_state = PlaybackState::Loading;
        self.view.position_ms = 0;
        self.view.duration_ms = None;
        self.view.seekable = false;
        self.view.error = None;
        if let Err(error) = self.try_open() {
            self.pending_open = None;
            self.auto_play = false;
            self.view.playback_state = PlaybackState::Error;
            self.view.error = Some(error);
            return Err(error);
        }
        Ok(())
    }

    pub fn play_pause(&mut self) -> Result<(), MediaError> {
        if !self.view.controls().play_pause {
            return Err(MediaError::new(MediaErrorKind::InvalidState, 1));
        }
        self.auto_play = false;
        self.desired_playing = Some(!self.shows_pause());
        self.transport_issued = false;
        self.pump_controls(self.player.snapshot().state);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), MediaError> {
        if self.view.controls().stop {
            self.auto_play = false;
            self.desired_playing = None;
            self.transport_issued = false;
            self.pending_seek_ms = None;
            self.interaction.seek_commit_pending = false;
            self.pending_stop = true;
            self.pump_controls(self.player.snapshot().state);
            Ok(())
        } else {
            Err(MediaError::new(MediaErrorKind::InvalidState, 2))
        }
    }

    pub fn begin_seek(&mut self, percent: u32) -> bool {
        if !self.seek_enabled() {
            return false;
        }
        self.interaction.seek_drag_active = true;
        self.interaction.seek_preview_percent = percent.min(100);
        true
    }

    pub fn preview_seek(&mut self, percent: u32) {
        if self.interaction.seek_drag_active {
            self.interaction.seek_preview_percent = percent.min(100);
        }
    }

    pub fn commit_seek(&mut self, percent: u32) -> Result<(), MediaError> {
        self.interaction.seek_drag_active = false;
        self.interaction.seek_preview_percent = percent.min(100);
        let target = seek_target_ms(&self.view, percent)
            .ok_or_else(|| MediaError::new(MediaErrorKind::Seek, 1))?;
        self.pending_seek_ms = Some(target);
        self.seek_issued = false;
        self.interaction.seek_commit_pending = true;
        self.interaction.seek_target_ms = target;
        self.view.position_ms = target;
        self.pump_controls(self.player.snapshot().state);
        Ok(())
    }

    pub fn cancel_seek(&mut self) {
        self.interaction.seek_drag_active = false;
    }

    pub fn set_volume(&mut self, value: u32) -> Result<(), MediaError> {
        self.player.set_volume(value.min(100) as u8)
    }

    pub fn refresh(&mut self) -> bool {
        if self.pending_open.is_some() {
            if let Err(error) = self.try_open() {
                self.pending_open = None;
                self.auto_play = false;
                self.view.playback_state = PlaybackState::Error;
                self.view.error = Some(error);
                return true;
            }
            if self.pending_open.is_some() {
                return false;
            }
        }
        let snapshot = self.player.snapshot();
        self.apply_snapshot(snapshot)
    }

    fn try_open(&mut self) -> Result<(), MediaError> {
        let Some(path) = self.pending_open.as_deref() else {
            return Ok(());
        };
        match self.player.open(path) {
            Ok(()) => {
                self.pending_open = None;
                let snapshot = self.player.snapshot();
                self.source_generation = snapshot.generation;
                self.view.apply_backend(snapshot, false);
                Ok(())
            }
            Err(error) if error.kind == MediaErrorKind::Busy => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn pump_controls(&mut self, state: sunlight_media::PlaybackState) {
        if self.pending_open.is_some() || state == sunlight_media::PlaybackState::Loading {
            return;
        }
        if self.player.command_pending() {
            return;
        }
        if self.pending_stop {
            if matches!(
                state,
                sunlight_media::PlaybackState::Ready | sunlight_media::PlaybackState::Idle
            ) {
                self.pending_stop = false;
            } else if self.player.stop().is_ok() {
                return;
            }
            if self.pending_stop {
                return;
            }
        }
        if let Some(target) = self.pending_seek_ms {
            if !self.seek_issued {
                let epoch = self.player.snapshot().seek_epoch;
                if self.player.seek(MediaTime::from_millis(target)).is_ok() {
                    self.seek_issued = true;
                    self.seek_epoch_at_issue = epoch;
                }
                return;
            }
        }
        if self.auto_play
            && state == sunlight_media::PlaybackState::Ready
            && self.desired_playing.is_none()
        {
            self.desired_playing = Some(true);
            self.transport_issued = false;
        } else if state == sunlight_media::PlaybackState::Playing {
            self.auto_play = false;
        }
        if let Some(playing) = self.desired_playing {
            let satisfied = if playing {
                state == sunlight_media::PlaybackState::Playing
            } else {
                state == sunlight_media::PlaybackState::Paused
            };
            if satisfied
                || (playing
                    && self.transport_issued
                    && state == sunlight_media::PlaybackState::Ended)
            {
                self.desired_playing = None;
                self.auto_play = false;
                self.transport_issued = false;
            } else if !self.transport_issued {
                let result = if playing {
                    self.player.play()
                } else if state == sunlight_media::PlaybackState::Playing {
                    self.player.pause()
                } else {
                    return;
                };
                self.transport_issued = result.is_ok();
            }
        }
    }

    fn apply_snapshot(&mut self, snapshot: MediaSnapshot) -> bool {
        if snapshot.generation != self.source_generation {
            return false;
        }
        let old = self.view;
        if snapshot.state == sunlight_media::PlaybackState::Error {
            self.auto_play = false;
            self.desired_playing = None;
            self.transport_issued = false;
            self.pending_stop = false;
            self.pending_seek_ms = None;
            self.seek_issued = false;
            self.interaction.seek_commit_pending = false;
        }
        if self.interaction.seek_commit_pending && self.seek_issued {
            if snapshot.seek_epoch != self.seek_epoch_at_issue || snapshot.error.is_some() {
                self.interaction.seek_commit_pending = false;
                self.pending_seek_ms = None;
                self.seek_issued = false;
            }
        }
        self.view.apply_backend(
            snapshot,
            self.interaction.seek_drag_active || self.interaction.seek_commit_pending,
        );
        if snapshot.state == sunlight_media::PlaybackState::Playing {
            self.visualization = snapshot.visualization;
        }
        self.pump_controls(snapshot.state);
        old != self.view || snapshot.state == sunlight_media::PlaybackState::Playing
    }
}

impl Default for MelodyMediaController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_media::{
        AudioStreamInfo, PcmFormat, PlaybackState as BackendState, VisualizationFrame,
    };

    fn seek_snapshot(generation: u64, position_ms: u64) -> MediaSnapshot {
        MediaSnapshot {
            generation,
            state: BackendState::Paused,
            position: MediaTime::from_millis(position_ms),
            seek_epoch: 0,
            stream: Some(AudioStreamInfo {
                sample_rate_hz: 48_000,
                channels: 2,
                sample_format: PcmFormat::Signed16LeInterleaved,
                duration: Some(MediaTime::from_millis(10_000)),
                seekable: true,
            }),
            volume: 68,
            error: None,
            visualization: VisualizationFrame::empty(),
        }
    }

    #[test]
    fn stale_source_state_and_visualization_are_ignored() {
        let mut controller = MelodyMediaController::new();
        controller.source_generation = 9;
        let stale = MediaSnapshot {
            generation: 8,
            state: BackendState::Ended,
            position: MediaTime::from_millis(1_000),
            seek_epoch: 0,
            stream: None,
            volume: 12,
            error: None,
            visualization: {
                let mut frame = VisualizationFrame::empty();
                frame.len = 1;
                frame.bins[0] = 100;
                frame
            },
        };
        assert!(!controller.apply_snapshot(stale));
        assert_eq!(controller.view.playback_state, PlaybackState::Idle);
        assert_eq!(controller.view.volume, 68);
        assert!(controller.visualization.bins().is_empty());
    }

    #[test]
    fn volume_commands_clamp_to_backend_range() {
        let mut controller = MelodyMediaController::new();
        assert!(controller.set_volume(900).is_ok());
        assert_eq!(controller.player.snapshot().volume, 100);
    }

    #[test]
    fn committed_seek_holds_preview_until_backend_resynchronizes() {
        let mut controller = MelodyMediaController::new();
        controller.source_generation = 4;
        controller.view.position_ms = 7_500;
        controller.interaction.seek_commit_pending = true;
        controller.interaction.seek_target_ms = 7_500;
        controller.seek_issued = true;
        assert!(controller.apply_snapshot(seek_snapshot(4, 2_000)));
        assert_eq!(controller.view.position_ms, 7_500);
        assert!(controller.interaction.seek_commit_pending);
        let mut acknowledged = seek_snapshot(4, 7_500);
        acknowledged.seek_epoch = 1;
        let _ = controller.apply_snapshot(acknowledged);
        assert_eq!(controller.view.position_ms, 7_500);
        assert!(!controller.interaction.seek_commit_pending);
    }

    #[test]
    fn seek_ack_uses_worker_epoch_and_ignores_stale_eof() {
        let mut controller = MelodyMediaController::new();
        controller.source_generation = 4;
        controller.view.position_ms = 7_500;
        controller.interaction.seek_commit_pending = true;
        controller.interaction.seek_target_ms = 7_500;
        controller.seek_issued = true;
        let mut acknowledged = seek_snapshot(4, 7_575);
        acknowledged.seek_epoch = 1;
        let _ = controller.apply_snapshot(acknowledged);
        assert!(!controller.interaction.seek_commit_pending);
        assert_eq!(controller.view.position_ms, 7_575);

        controller.interaction.seek_commit_pending = true;
        controller.interaction.seek_target_ms = 2_500;
        controller.seek_issued = true;
        controller.seek_epoch_at_issue = 1;
        let mut ended = seek_snapshot(4, 10_000);
        ended.state = BackendState::Ended;
        ended.seek_epoch = 1;
        let _ = controller.apply_snapshot(ended);
        assert!(controller.interaction.seek_commit_pending);
        assert_eq!(controller.view.position_ms, 7_575);
        let mut acknowledged = seek_snapshot(4, 2_500);
        acknowledged.seek_epoch = 2;
        let _ = controller.apply_snapshot(acknowledged);
        assert!(!controller.interaction.seek_commit_pending);
        assert_eq!(controller.view.position_ms, 2_500);
    }

    #[test]
    fn track_change_waits_for_pending_worker_command() {
        let mut controller = MelodyMediaController::new();
        controller.player.play().unwrap();
        controller.open("/music/next.wav").unwrap();
        assert_eq!(controller.pending_open.as_deref(), Some("/music/next.wav"));
        assert_eq!(controller.view.playback_state, PlaybackState::Loading);
        assert!(!controller.refresh());
        assert_eq!(controller.pending_open.as_deref(), Some("/music/next.wav"));
    }

    #[test]
    fn repeated_click_during_loading_keeps_one_open_request() {
        let mut controller = MelodyMediaController::new();
        controller.open_paused("/music/one.wav").unwrap();
        let generation = controller.player.snapshot().generation;
        controller.open("/music/one.wav").unwrap();
        assert_eq!(controller.player.snapshot().generation, generation);
        assert!(controller.auto_play);
        assert!(controller.pending_open.is_none());
    }

    #[test]
    fn seek_waits_for_a_busy_command_and_preserves_preview() {
        let mut controller = MelodyMediaController::new();
        controller
            .view
            .apply_backend(seek_snapshot(0, 2_000), false);
        controller.player.play().unwrap();
        assert!(controller.begin_seek(75));
        controller.commit_seek(75).unwrap();
        assert_eq!(controller.pending_seek_ms, Some(7_500));
        assert!(!controller.seek_issued);
        assert_eq!(controller.view.position_ms, 7_500);
    }
}
