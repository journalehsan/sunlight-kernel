//! Optional sound boundary. audio.v1 has one global application stream and no
//! effect mixer. Keep cues nonblocking and silent until per-client mixing exists.
#[derive(Clone, Copy)]
pub enum Cue {
    Selection,
    Started,
    Completed,
    Invalid,
}
pub fn cue(_: Cue) {}
