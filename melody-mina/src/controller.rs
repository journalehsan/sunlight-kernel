//! Thin Melody Mina client adapter for the reusable Sunlight media API.

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

    pub fn open(&mut self, path: &str) -> Result<(), MediaError> {
        self.player.open(path)?;
        let snapshot = self.player.snapshot();
        self.source_generation = snapshot.generation;
        self.view.apply_backend(snapshot, false);
        self.interaction = InteractionState::default();
        self.visualization = VisualizationFrame::empty();
        Ok(())
    }

    pub fn play_pause(&mut self) -> Result<(), MediaError> {
        match self.view.playback_state {
            PlaybackState::Playing => self.player.pause(),
            _ if self.view.controls().play_pause => self.player.play(),
            _ => Err(MediaError::new(MediaErrorKind::InvalidState, 1)),
        }
    }

    pub fn stop(&mut self) -> Result<(), MediaError> {
        if self.view.controls().stop {
            self.player.stop()
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
        self.player.seek(MediaTime::from_millis(target))?;
        self.interaction.seek_commit_pending = true;
        self.interaction.seek_target_ms = target;
        self.view.position_ms = target;
        Ok(())
    }

    pub fn cancel_seek(&mut self) {
        self.interaction.seek_drag_active = false;
    }

    pub fn set_volume(&mut self, value: u32) -> Result<(), MediaError> {
        self.player.set_volume(value.min(100) as u8)
    }

    pub fn refresh(&mut self) -> bool {
        let snapshot = self.player.snapshot();
        self.apply_snapshot(snapshot)
    }

    fn apply_snapshot(&mut self, snapshot: MediaSnapshot) -> bool {
        if snapshot.generation != self.source_generation {
            return false;
        }
        let old = self.view;
        if self.interaction.seek_commit_pending {
            let distance = snapshot
                .position
                .as_millis()
                .abs_diff(self.interaction.seek_target_ms);
            if distance <= 25 || snapshot.error.is_some() {
                self.interaction.seek_commit_pending = false;
            }
        }
        self.view.apply_backend(
            snapshot,
            self.interaction.seek_drag_active || self.interaction.seek_commit_pending,
        );
        if snapshot.state == sunlight_media::PlaybackState::Playing {
            self.visualization = snapshot.visualization;
        }
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
        assert!(controller.apply_snapshot(seek_snapshot(4, 2_000)));
        assert_eq!(controller.view.position_ms, 7_500);
        assert!(controller.interaction.seek_commit_pending);
        let _ = controller.apply_snapshot(seek_snapshot(4, 7_500));
        assert_eq!(controller.view.position_ms, 7_500);
        assert!(!controller.interaction.seek_commit_pending);
    }
}
