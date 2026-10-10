//! Deterministic session transitions. Commands stay occupied until acknowledged.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Idle,
    Recording,
    Paused,
    Finalizing,
    Recorded,
    Playing,
    PlaybackPaused,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Record,
    Pause,
    Resume,
    Stop,
    Finalized { frames: u64 },
    Play,
    PausePlayback,
    ResumePlayback,
    PlaybackEnded,
    Save,
    New,
    Fail,
}

pub fn transition(state: State, action: Action) -> Result<State, &'static str> {
    use Action::*;
    use State::*;
    Ok(match (state, action) {
        (Idle, Record) => Recording,
        (Recording, Pause) => Paused,
        (Paused, Resume) => Recording,
        (Recording | Paused, Stop) => Finalizing,
        (Finalizing, Finalized { frames: 0 }) => Idle,
        (Finalizing, Finalized { .. }) => Recorded,
        (Recorded, Play) | (PlaybackPaused, ResumePlayback) => Playing,
        (Playing, PausePlayback) => PlaybackPaused,
        (Playing | PlaybackPaused, PlaybackEnded | Stop) => Recorded,
        (Recorded, Save) => Recorded,
        (Idle | Recorded | Error, New) => Idle,
        (_, Fail) => Error,
        _ => return Err("This action is unavailable in the current recorder state"),
    })
}

impl State {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Idle,
            1 => Self::Recording,
            2 => Self::Paused,
            3 => Self::Finalizing,
            4 => Self::Recorded,
            5 => Self::Playing,
            6 => Self::PlaybackPaused,
            _ => Self::Error,
        }
    }
}

pub fn duration_millis(frames: u64, rate: u32) -> u64 {
    if rate == 0 {
        return 0;
    }
    (frames / u64::from(rate))
        .saturating_mul(1000)
        .saturating_add((frames % u64::from(rate)) * 1000 / u64::from(rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_and_playback_transitions() {
        let mut s = State::Idle;
        for (a, expected) in [
            (Action::Record, State::Recording),
            (Action::Pause, State::Paused),
            (Action::Resume, State::Recording),
            (Action::Stop, State::Finalizing),
            (Action::Finalized { frames: 48000 }, State::Recorded),
            (Action::Save, State::Recorded),
            (Action::Play, State::Playing),
            (Action::PausePlayback, State::PlaybackPaused),
            (Action::ResumePlayback, State::Playing),
            (Action::PlaybackEnded, State::Recorded),
            (Action::New, State::Idle),
        ] {
            s = transition(s, a).unwrap();
            assert_eq!(s, expected);
        }
    }
    #[test]
    fn rejects_duplicates_and_actions_without_a_recording() {
        assert!(transition(State::Idle, Action::Save).is_err());
        assert!(transition(State::Idle, Action::Play).is_err());
        assert!(transition(State::Recording, Action::Record).is_err());
        assert!(transition(State::Recording, Action::Play).is_err());
        assert!(transition(State::Recording, Action::Save).is_err());
        assert!(transition(State::Finalizing, Action::Stop).is_err());
        assert!(transition(State::Recorded, Action::Stop).is_err());
        assert!(transition(State::Paused, Action::Pause).is_err());
    }
    #[test]
    fn empty_recording_returns_idle() {
        assert_eq!(
            transition(State::Finalizing, Action::Finalized { frames: 0 }),
            Ok(State::Idle)
        );
    }
    #[test]
    fn frame_clock_and_paused_time() {
        let mut s = transition(State::Idle, Action::Record).unwrap();
        let frames = 48000 + 1024;
        let before = duration_millis(frames, 48000);
        s = transition(s, Action::Pause).unwrap();
        assert_eq!(duration_millis(frames, 48000), before);
        s = transition(s, Action::Resume).unwrap();
        assert_eq!(s, State::Recording);
        assert_eq!(duration_millis(frames + 48000, 48000), 2021);
        assert_eq!(duration_millis(0, 48000), 0);
        assert_eq!(duration_millis(180 * 48000, 48000), 180000);
        assert_eq!(duration_millis(10, 0), 0);
    }
}
