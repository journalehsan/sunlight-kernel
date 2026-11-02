//! Notification state belongs to the existing broker channel, never a bearer.
use super::{
    channel::{Error, Handle, State},
    delivery::RecipientIdentity,
    CapabilityBroker,
};
use sunlight_fastlane_doorbell::{Channel, Doorbell, Identity, Wait};

pub(super) fn identity(who: RecipientIdentity) -> Identity {
    Identity {
        pid: who.pid as u64,
        incarnation: who.incarnation,
    }
}
pub(super) fn key(h: Handle) -> Channel {
    // Full token AND generation are checked by row_mut before model access.
    Channel {
        slot: h.token.0,
        generation: h.generation,
    }
}
pub(super) fn error(e: sunlight_fastlane_doorbell::Error) -> Error {
    use sunlight_fastlane_doorbell::{Error as E, Terminal};
    match e {
        E::Stale => Error::Stale,
        E::Unauthorized | E::Invalid => Error::Denied,
        E::Capacity => Error::Capacity,
        E::AlreadyWaiting => Error::Duplicate,
        E::Terminal(Terminal::Exhausted) => Error::Exhausted,
        E::Terminal(_) => Error::Terminal,
    }
}
impl CapabilityBroker {
    /// op: 10 notify (producer), 11 observe / 12 wait (consumer),
    /// 14 cancel current wait (producer). Close uses the existing revoke op 4.
    pub(super) fn channel_bell(
        &mut self,
        h: Handle,
        who: RecipientIdentity,
        op: u64,
        observed: u64,
    ) -> Result<(u64, Option<Identity>), Error> {
        let row = self.channels.row_mut(h)?;
        let allowed = match op {
            10 | 14 => who == row.creator,
            11 | 12 => row.consumer == Some(who),
            _ => false,
        };
        if !allowed {
            return Err(Error::Denied);
        }
        if !row.live() {
            return Err(Error::Terminal);
        }
        if row.state != State::Attached {
            return Err(Error::Incomplete);
        }
        let consumer = identity(row.consumer.ok_or(Error::Incomplete)?);
        if row.bell.is_none() {
            row.bell = Some(
                Doorbell::new(
                    key(h),
                    identity(row.creator),
                    identity(row.creator),
                    consumer,
                    true,
                )
                .map_err(error)?,
            );
        }
        let bell = row.bell.as_mut().unwrap();
        #[cfg(all(debug_assertions, feature = "fastlane_doorbell_trace"))]
        crate::sched::bell_trace::event("operation", h.generation, who.pid,
            who.incarnation, who.vm_generation, op, observed);
        match op {
            10 => {
                let effect = bell.notify(key(h), identity(who)).map_err(error)?;
                let value = bell.snapshot(key(h), consumer).map_err(error);
                #[cfg(all(debug_assertions, feature = "fastlane_doorbell_trace"))]
                crate::sched::bell_trace::event("publish-remove", h.generation, who.pid,
                    who.incarnation, who.vm_generation, value.unwrap_or(u64::MAX), u64::from(effect.wake.is_some()));
                // Exhaustion is terminal and MUST still deliver its wake.
                Ok((value.unwrap_or(u64::MAX), effect.wake))
            }
            11 => bell
                .snapshot(key(h), identity(who))
                .map(|s| (s, None))
                .map_err(error),
            12 => match bell.wait(key(h), identity(who), observed).map_err(error)? {
                Wait::Changed(_) => Err(Error::Changed),
                Wait::Registered => Ok((0, None)),
            },
            14 => {
                // This is explicit channel-scoped cancellation by its producer,
                // not a transferred per-registration cancellation capability.
                bell.snapshot(key(h), consumer).map_err(error)?;
                let cancelled = bell.cancel(key(h), consumer).map_err(error)?;
                Ok((0, cancelled.then_some(consumer)))
            }
            _ => Err(Error::Denied),
        }
    }
}
