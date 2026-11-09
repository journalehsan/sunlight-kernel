//! Owner-bound launch authorization in the existing Account registry.
use super::{CapabilityBroker, Error, RecipientIdentity, SessionRef, State};

/// Opaque, move-only authorization for one owner-requested launch. It is not
/// serializable and cannot be manufactured from a SessionRef by callers.
pub(crate) struct LaunchAttachment {
    reference: SessionRef,
    owner: RecipientIdentity,
    revision: u64,
    bootstrap: bool,
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    test_failure: u8,
}

impl LaunchAttachment {
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    pub(crate) fn with_test_failure(mut self, stage: u8) -> Self {
        self.test_failure = stage;
        self
    }
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    pub(crate) fn fails_at(&self, stage: u8) -> bool {
        self.test_failure == stage
    }
}

impl CapabilityBroker {
    /// Caller must hold SCHEDULER and authenticate the trusted SessionD role.
    /// Exact owner identity includes process incarnation and VM generation.
    pub(crate) fn authorize_session_launch(
        &self,
        owner: RecipientIdentity,
        reference: SessionRef,
        now: u64,
    ) -> Result<(LaunchAttachment, u32, u32), Error> {
        let session = self
            .sessions
            .rows
            .iter()
            .find(|s| s.reference == reference)
            .ok_or(Error::SessionChanged)?;
        if session.owner != owner || session.state != State::Active || now >= session.deadline {
            return Err(Error::Denied);
        }
        Ok((
            LaunchAttachment {
                reference,
                owner,
                revision: session.launch_revision,
                bootstrap: false,
                #[cfg(all(debug_assertions, feature = "session_launch_test"))]
                test_failure: 0,
            },
            session.uid,
            session.gid,
        ))
    }

    /// Only kernel callers may supply this reference, read from the current
    /// parent's Process record under SCHEDULER. Never accept it from IPC data.
    pub(crate) fn authorize_inherited_session_launch(
        &self,
        reference: SessionRef,
        now: u64,
    ) -> Result<LaunchAttachment, Error> {
        let owner = self
            .sessions
            .rows
            .iter()
            .find(|s| s.reference == reference)
            .ok_or(Error::SessionChanged)?
            .owner;
        self.authorize_session_launch(owner, reference, now)
            .map(|(attachment, _, _)| attachment)
    }

    /// The registered owner may bootstrap exactly once from a freshly consumed
    /// local-auth proof. An explicit suspension permanently closes this path.
    pub(crate) fn authorize_initial_session_launch(
        &self,
        owner: RecipientIdentity,
        reference: SessionRef,
        now: u64,
    ) -> Result<(LaunchAttachment, u32, u32), Error> {
        let session = self
            .sessions
            .rows
            .iter()
            .find(|s| s.reference == reference)
            .ok_or(Error::SessionChanged)?;
        if session.owner != owner
            || !session.bootstrap_available
            || session.state != State::Suspended
            || now >= session.deadline
        {
            return Err(Error::Denied);
        }
        Ok((
            LaunchAttachment {
                reference,
                owner,
                revision: session.launch_revision,
                bootstrap: true,
                #[cfg(all(debug_assertions, feature = "session_launch_test"))]
                test_failure: 0,
            },
            session.uid,
            session.gid,
        ))
    }

    /// Commit and publication share SCHEDULER -> PMM -> CAP_BROKER guards.
    /// Consume bootstrap eligibility only after all fallible construction.
    pub(crate) fn commit_session_launch(
        &mut self,
        attachment: LaunchAttachment,
        now: u64,
    ) -> Result<SessionRef, Error> {
        if attachment.bootstrap {
            self.authorize_initial_session_launch(attachment.owner, attachment.reference, now)?;
        } else {
            self.authorize_session_launch(attachment.owner, attachment.reference, now)?;
        }
        let session = self
            .sessions
            .rows
            .iter_mut()
            .find(|s| s.reference == attachment.reference)
            .ok_or(Error::SessionChanged)?;
        if session.launch_revision != attachment.revision {
            return Err(Error::Denied);
        }
        if attachment.bootstrap {
            session.bootstrap_available = false;
            session.state = State::Active;
        }
        Ok(attachment.reference)
    }

    pub(crate) fn session_allocation_live(&self, reference: SessionRef, now: u64) -> bool {
        self.sessions
            .rows
            .iter()
            .any(|s| s.reference == reference && s.state == State::Active && now < s.deadline)
    }
}
