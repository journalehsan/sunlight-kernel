//! Presentation-only state consumed by the Melody Mina frontend.
//!
//! These types deliberately contain no decoder, PCM, device, or service state.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Paused,
    PlayingPresentation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NowPlayingViewModel {
    pub title: &'static str,
    pub artist: &'static str,
    pub album: &'static str,
    pub elapsed_seconds: u32,
    pub duration_seconds: u32,
    pub seekable: bool,
    pub playback_state: PlaybackState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaylistItemViewModel {
    pub title: &'static str,
    pub artist: Option<&'static str>,
    pub duration_seconds: Option<u32>,
}

pub const DEMO_NOW_PLAYING: NowPlayingViewModel = NowPlayingViewModel {
    title: "Sunlight Dreams",
    artist: "Helios Collective",
    album: "Demo Sessions / Frontend preview",
    elapsed_seconds: 42,
    duration_seconds: 207,
    seekable: true,
    playback_state: PlaybackState::Paused,
};

pub const DEMO_PLAYLIST: [PlaylistItemViewModel; 12] = [
    PlaylistItemViewModel {
        title: "Sunlight Dreams",
        artist: Some("Helios Collective"),
        duration_seconds: Some(207),
    },
    PlaylistItemViewModel {
        title: "Warm Horizon",
        artist: Some("Mina & the Solars"),
        duration_seconds: Some(234),
    },
    PlaylistItemViewModel {
        title: "A Quiet Orbit",
        artist: Some("Amber Field"),
        duration_seconds: Some(191),
    },
    PlaylistItemViewModel {
        title: "After the Rain",
        artist: Some("North Window"),
        duration_seconds: Some(268),
    },
    PlaylistItemViewModel {
        title: "Gold on Glass",
        artist: Some("Helios Collective"),
        duration_seconds: Some(222),
    },
    PlaylistItemViewModel {
        title: "Morning Circuit",
        artist: None,
        duration_seconds: Some(176),
    },
    PlaylistItemViewModel {
        title: "Long Way Through the Blue",
        artist: Some("Mina & the Solars"),
        duration_seconds: Some(244),
    },
    PlaylistItemViewModel {
        title: "Soft Machines",
        artist: Some("Amber Field"),
        duration_seconds: Some(199),
    },
    PlaylistItemViewModel {
        title: "Window Seat",
        artist: Some("North Window"),
        duration_seconds: Some(215),
    },
    PlaylistItemViewModel {
        title: "Solar Echo",
        artist: Some("Helios Collective"),
        duration_seconds: Some(188),
    },
    PlaylistItemViewModel {
        title: "Between Stations",
        artist: None,
        duration_seconds: None,
    },
    PlaylistItemViewModel {
        title: "Home in the Daylight",
        artist: Some("Mina & the Solars"),
        duration_seconds: Some(251),
    },
];

pub const fn timeline_percent(model: &NowPlayingViewModel) -> u32 {
    if !model.seekable || model.duration_seconds == 0 {
        0
    } else {
        model.elapsed_seconds.saturating_mul(100) / model.duration_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_values_are_centralized_and_bounded() {
        assert_eq!(DEMO_NOW_PLAYING.title, "Sunlight Dreams");
        assert!(DEMO_NOW_PLAYING.elapsed_seconds < DEMO_NOW_PLAYING.duration_seconds);
        assert_eq!(DEMO_PLAYLIST.len(), 12);
    }

    #[test]
    fn timeline_handles_unseekable_and_zero_duration() {
        let mut model = DEMO_NOW_PLAYING;
        model.seekable = false;
        assert_eq!(timeline_percent(&model), 0);
        model.seekable = true;
        model.duration_seconds = 0;
        assert_eq!(timeline_percent(&model), 0);
    }
}
