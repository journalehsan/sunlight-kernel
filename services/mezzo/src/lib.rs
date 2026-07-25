#![no_std]

use sunlight_ipc::{LockState, LOCK_SESSION_USERNAME_MAX};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockSession {
    pub state: LockState,
    pub generation: u64,
    pub presenter_pid: u64,
    pub presenter_generation: u64,
    pub last_presenter_failure: u64,
    pub recovery_attempts: u32,
    pub safe_mode: bool,
    pub session_uid: u32,
    pub session_gid: u32,
    pub session_username: [u8; LOCK_SESSION_USERNAME_MAX],
    pub session_username_len: usize,
    pub transition_deadline_ms: u64,
}

impl LockSession {
    pub const fn new() -> Self {
        Self {
            state: LockState::Unlocked,
            generation: 0,
            presenter_pid: 0,
            presenter_generation: 0,
            last_presenter_failure: 0,
            recovery_attempts: 0,
            safe_mode: false,
            session_uid: 0,
            session_gid: 0,
            session_username: [0; LOCK_SESSION_USERNAME_MAX],
            session_username_len: 0,
            transition_deadline_ms: 0,
        }
    }

    pub fn establish_session(&mut self, uid: u32, gid: u32, username: &[u8]) -> bool {
        if self.state != LockState::Unlocked
            || username.is_empty()
            || username.len() > self.session_username.len()
        {
            return false;
        }
        self.session_username.fill(0);
        self.session_username[..username.len()].copy_from_slice(username);
        self.session_username_len = username.len();
        self.session_uid = uid;
        self.session_gid = gid;
        true
    }

    pub fn enter(&mut self, now_ms: u64, timeout_ms: u64) -> Option<u64> {
        if self.state != LockState::Unlocked || self.session_username_len == 0 {
            return None;
        }
        self.generation = self.generation.checked_add(1)?;
        self.state = LockState::EnteringLock;
        self.presenter_pid = 0;
        self.presenter_generation = 0;
        self.last_presenter_failure = 0;
        self.recovery_attempts = 0;
        self.safe_mode = false;
        self.transition_deadline_ms = now_ms.saturating_add(timeout_ms);
        Some(self.generation)
    }

    pub fn begin_recovery(&mut self, safe_mode: bool, now_ms: u64, timeout_ms: u64) -> bool {
        if self.state == LockState::Unlocked || self.state == LockState::LeavingLock {
            return false;
        }
        self.state = LockState::RecoveringPresenter;
        self.presenter_pid = 0;
        self.presenter_generation = 0;
        self.recovery_attempts = self.recovery_attempts.saturating_add(1);
        self.safe_mode = safe_mode;
        self.transition_deadline_ms = now_ms.saturating_add(timeout_ms);
        true
    }

    pub fn register_presenter(&mut self, generation: u64, pid: u64) -> bool {
        if generation != self.generation
            || pid == 0
            || !matches!(
                self.state,
                LockState::EnteringLock
                    | LockState::LockedFallback
                    | LockState::RecoveringPresenter
            )
        {
            return false;
        }
        self.presenter_pid = pid;
        self.presenter_generation = generation;
        true
    }

    pub fn presenter_ready(&mut self, generation: u64, pid: u64) -> bool {
        if generation != self.generation
            || self.presenter_generation != generation
            || self.presenter_pid != pid
        {
            return false;
        }
        self.state = LockState::LockedWithPresenter;
        self.transition_deadline_ms = 0;
        true
    }

    pub fn presenter_failed(&mut self, pid: u64) -> bool {
        if pid == 0 || pid != self.presenter_pid || self.state == LockState::Unlocked {
            return false;
        }
        self.fallback(pid);
        true
    }

    pub fn fallback(&mut self, failed_pid: u64) {
        self.last_presenter_failure = failed_pid;
        self.presenter_pid = 0;
        self.presenter_generation = 0;
        self.state = LockState::LockedFallback;
        self.transition_deadline_ms = 0;
    }

    pub fn transition_expired(&mut self, now_ms: u64, failed_pid: u64) -> bool {
        if !matches!(
            self.state,
            LockState::EnteringLock | LockState::RecoveringPresenter
        ) || self.transition_deadline_ms == 0
            || now_ms < self.transition_deadline_ms
        {
            return false;
        }
        self.fallback(failed_pid);
        true
    }

    pub fn begin_authentication(&mut self, generation: u64, pid: u64) -> bool {
        if self.state != LockState::LockedWithPresenter
            || generation != self.generation
            || pid != self.presenter_pid
        {
            return false;
        }
        self.state = LockState::Authenticating;
        true
    }

    pub const fn authentication_identity_matches(&self, uid: u32, gid: u32) -> bool {
        uid == self.session_uid && gid == self.session_gid
    }

    pub fn authentication_failed(&mut self, generation: u64, pid: u64) -> bool {
        if self.state != LockState::Authenticating
            || generation != self.generation
            || pid != self.presenter_pid
        {
            return false;
        }
        self.state = LockState::LockedWithPresenter;
        self.transition_deadline_ms = 0;
        true
    }

    pub fn leave(&mut self, generation: u64, pid: u64) -> bool {
        if self.state != LockState::Authenticating
            || generation != self.generation
            || pid != self.presenter_pid
        {
            return false;
        }
        self.state = LockState::LeavingLock;
        true
    }

    pub fn finish_leave(&mut self) {
        self.state = LockState::Unlocked;
        self.presenter_pid = 0;
        self.presenter_generation = 0;
        self.safe_mode = false;
        self.transition_deadline_ms = 0;
    }
}

impl Default for LockSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_presenters_and_authentication_are_rejected() {
        let mut session = LockSession::new();
        assert!(session.establish_session(1000, 1000, b"sunlight"));
        let generation = session.enter(10, 1_000).unwrap();
        assert!(!session.register_presenter(generation + 1, 10));
        assert!(session.register_presenter(generation, 10));
        assert!(session.presenter_ready(generation, 10));
        assert!(!session.begin_authentication(generation - 1, 10));
        assert!(session.begin_authentication(generation, 10));
        assert!(!session.leave(generation, 11));
        assert!(session.leave(generation, 10));
    }

    #[test]
    fn presenter_failure_never_unlocks() {
        let mut session = LockSession::new();
        assert!(session.establish_session(1000, 1000, b"sunlight"));
        let generation = session.enter(10, 1_000).unwrap();
        assert!(session.register_presenter(generation, 20));
        assert!(session.presenter_ready(generation, 20));
        assert!(session.presenter_failed(20));
        assert_eq!(session.state, LockState::LockedFallback);
        assert_ne!(session.state, LockState::Unlocked);
    }

    #[test]
    fn transition_timeout_fails_closed() {
        let mut session = LockSession::new();
        assert!(session.establish_session(1000, 1000, b"sunlight"));
        session.enter(100, 500).unwrap();
        assert!(!session.transition_expired(599, 42));
        assert!(session.transition_expired(600, 42));
        assert_eq!(session.state, LockState::LockedFallback);
        assert_eq!(session.last_presenter_failure, 42);
    }

    #[test]
    fn generation_never_wraps_or_reuses_a_stale_epoch() {
        let mut session = LockSession::new();
        assert!(session.establish_session(1000, 1000, b"sunlight"));
        session.generation = u64::MAX;
        assert!(session.enter(0, 100).is_none());
        assert_eq!(session.generation, u64::MAX);
        assert_eq!(session.state, LockState::Unlocked);
    }

    #[test]
    fn only_the_authenticated_session_identity_may_unlock() {
        let mut session = LockSession::new();
        assert!(session.establish_session(1000, 100, b"sunlight"));
        assert!(session.authentication_identity_matches(1000, 100));
        assert!(!session.authentication_identity_matches(0, 0));
        assert!(!session.authentication_identity_matches(1001, 100));
    }
}
