//! Phase 1 foundations for `uac-service`: session bookkeeping and a runas skeleton.
//!
//! This is a mock-first implementation intended to model a 30-minute prompt cache.
//! `UserSession` tracks cache lifetime and an active flag; session creation can be
//! extended later with password/PAM prompts.

use heapless::{FnvIndexMap, String};

/// Cache TTL for elevated sessions in seconds (30 minutes).
pub const SESSION_TTL_SECS: u64 = 1_800;

/// Generic timestamp used by this demo layer.
pub type Timestamp = u64;

/// Minimal error reporting for user/session operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// The operation is rejected because the request or identity is malformed.
    InvalidRequest,
    /// The session cache table cannot accept more entries.
    SessionStoreFull,
    /// No session exists for the requested uid.
    SessionMissing,
}

/// Result type for `uac-service` helpers.
pub type AuthResult<T> = Result<T, AuthError>;

/// Represents a cached authentication session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserSession {
    /// Session owner uid.
    pub uid: u32,
    /// Session creation time (seconds since an implementation-defined epoch).
    pub created_at: Timestamp,
    /// Expiry time (seconds since the same epoch).
    pub expires_at: Timestamp,
    /// Marked false when explicitly revoked.
    pub active: bool,
}

impl UserSession {
    /// Create a new 30-minute session for `uid`.
    pub fn new(uid: u32, now: Timestamp) -> Self {
        Self {
            uid,
            created_at: now,
            expires_at: now.saturating_add(SESSION_TTL_SECS),
            active: true,
        }
    }

    /// Return true when the session is still active.
    pub fn is_active(&self, now: Timestamp) -> bool {
        self.active && now < self.expires_at
    }

    /// Invalidate a session (mock admin/daemon revocation path).
    pub fn invalidate(&mut self) {
        self.active = false;
    }
}

/// Bounded session table keyed by uid.
pub struct SessionStore<const N: usize> {
    sessions: FnvIndexMap<u32, UserSession, N>,
}

impl<const N: usize> SessionStore<N> {
    /// Empty session table.
    pub const fn new() -> Self {
        Self {
            sessions: FnvIndexMap::new(),
        }
    }

    /// Fetch a session by uid.
    pub fn get(&self, uid: u32) -> Option<&UserSession> {
        self.sessions.get(&uid)
    }

    /// Refresh or create a session for `uid` and return the resulting state.
    pub fn ensure_session(&mut self, uid: u32, now: Timestamp) -> AuthResult<UserSession> {
        if let Some(session) = self.sessions.get_mut(&uid) {
            *session = UserSession::new(uid, now);
            return Ok(*session);
        }

        self.sessions
            .insert(uid, UserSession::new(uid, now))
            .map_err(|_| AuthError::SessionStoreFull)?;
        self.sessions
            .get(&uid)
            .copied()
            .ok_or(AuthError::SessionMissing)
    }

    /// Validate and fetch cached session state.
    pub fn get_active(&self, uid: u32, now: Timestamp) -> Option<&UserSession> {
        self.sessions.get(&uid).filter(|s| s.is_active(now))
    }
}

/// Request form used by `runas`.
#[derive(Debug)]
pub struct RunasRequest {
    /// Calling user requesting elevation.
    pub caller_uid: u32,
    /// Target uid that the caller wants to execute with.
    pub target_uid: u32,
    /// Optional short command label used for audit/logging.
    pub command: String<64>,
}

impl RunasRequest {
    pub fn new(caller_uid: u32, target_uid: u32, command: &str) -> Self {
        let mut cmd = String::<64>::new();
        let _ = cmd.push_str(command);
        Self {
            caller_uid,
            target_uid,
            command: cmd,
        }
    }
}

/// Response shape for a mocked runas decision.
#[derive(Debug, PartialEq, Eq)]
pub enum RunasOutcome {
    /// Existing prompt cache accepted this run; no re-prompt needed.
    CacheUsed,
    /// Cache was missing/expired; request must be re-prompted (mock always grants once prompted).
    Prompted,
}

/// Skeleton handler that validates/creates cached session and returns authorization.
///
/// This does not perform real credential checks. It demonstrates cache policy and
/// the control flow for later integration with prompts/biometrics/IPC auth.
pub fn runas<const N: usize>(
    req: &RunasRequest,
    now: Timestamp,
    store: &mut SessionStore<N>,
) -> AuthResult<RunasOutcome> {
    if req.command.is_empty() {
        return Err(AuthError::InvalidRequest);
    }

    if let Some(session) = store.get_active(req.caller_uid, now) {
        if session.uid != req.caller_uid {
            return Err(AuthError::InvalidRequest);
        }

        return Ok(RunasOutcome::CacheUsed);
    }

    // Mock behavior: create/refresh cache immediately and mark this as a prompted flow.
    let _ = store.ensure_session(req.caller_uid, now)?;
    let _ = req.target_uid;
    Ok(RunasOutcome::Prompted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_session_expiry() {
        let now: Timestamp = 10_000;
        let session = UserSession::new(1000, now);
        assert!(session.is_active(now + 1));
        assert!(!session.is_active(now + SESSION_TTL_SECS + 1));
    }

    #[test]
    fn runas_cache_flow() {
        let mut store: SessionStore<4> = SessionStore::new();
        let now: Timestamp = 5_000;
        let req = RunasRequest::new(1000, 0, "/bin/sh");
        let first = runas(&req, now, &mut store).unwrap();
        assert_eq!(first, RunasOutcome::Prompted);
        let second = runas(&req, now + 120, &mut store).unwrap();
        assert_eq!(second, RunasOutcome::CacheUsed);
    }
}
