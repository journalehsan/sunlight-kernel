use crate::{
    error::{MediaError, MediaErrorKind},
    types::PlaybackState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaybackAction {
    BeginOpen,
    LoadReady,
    Play,
    Pause,
    Stop { loaded: bool },
    SeekComplete { resume: bool },
    End,
}

/// Pure transition policy shared by the native worker and host tests. Resource
/// side effects (decoder reset and sink flush) happen before the returned state
/// is published.
pub(crate) fn transition(
    current: PlaybackState,
    action: PlaybackAction,
) -> Result<PlaybackState, MediaError> {
    use PlaybackAction as Action;
    use PlaybackState as State;
    match action {
        Action::BeginOpen => Ok(State::Loading),
        Action::LoadReady if current == State::Loading => Ok(State::Ready),
        Action::Play if matches!(current, State::Ready | State::Paused | State::Ended) => {
            Ok(State::Playing)
        }
        Action::Play if current == State::Playing => Ok(State::Playing),
        Action::Pause if current == State::Playing => Ok(State::Paused),
        Action::Pause => Ok(current),
        Action::Stop { loaded: true } => Ok(State::Ready),
        Action::Stop { loaded: false } => Ok(State::Idle),
        Action::SeekComplete { resume: true } => Ok(State::Playing),
        Action::SeekComplete { resume: false } => Ok(State::Paused),
        Action::End if current == State::Playing => Ok(State::Ended),
        _ => Err(MediaError::new(MediaErrorKind::InvalidState, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_play_pause_resume_stop_and_eof_are_explicit() {
        let mut state = PlaybackState::Idle;
        state = transition(state, PlaybackAction::BeginOpen).unwrap();
        assert_eq!(state, PlaybackState::Loading);
        state = transition(state, PlaybackAction::LoadReady).unwrap();
        state = transition(state, PlaybackAction::Play).unwrap();
        assert_eq!(state, PlaybackState::Playing);
        state = transition(state, PlaybackAction::Pause).unwrap();
        assert_eq!(state, PlaybackState::Paused);
        state = transition(state, PlaybackAction::Play).unwrap();
        state = transition(state, PlaybackAction::End).unwrap();
        assert_eq!(state, PlaybackState::Ended);
        state = transition(state, PlaybackAction::Stop { loaded: true }).unwrap();
        assert_eq!(state, PlaybackState::Ready);
    }

    #[test]
    fn invalid_play_does_not_pretend_to_be_playing_and_seek_preserves_intent() {
        assert_eq!(
            transition(PlaybackState::Idle, PlaybackAction::Play)
                .unwrap_err()
                .kind,
            MediaErrorKind::InvalidState
        );
        assert_eq!(
            transition(
                PlaybackState::Playing,
                PlaybackAction::SeekComplete { resume: true }
            )
            .unwrap(),
            PlaybackState::Playing
        );
        assert_eq!(
            transition(
                PlaybackState::Paused,
                PlaybackAction::SeekComplete { resume: false }
            )
            .unwrap(),
            PlaybackState::Paused
        );
    }
}
