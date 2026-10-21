//! Serialized specification model, not a kernel API or an authority broker.
//! The owner must authenticate identities and serialize *all* operations with
//! scheduler blocked/runnable transitions. No operation performs a wake itself.
#![no_std]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub pid: u64,
    pub incarnation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub slot: u64,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminal {
    Closed,
    ProducerExit,
    ConsumerExit,
    Revoked,
    Exhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Stale,
    Unauthorized,
    Invalid,
    Capacity,
    Terminal(Terminal),
    AlreadyWaiting,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    Changed(u64),
    Registered,
}
/// A wake obligation: consume under the same serialization as registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    pub wake: Option<Identity>,
    pub terminal: Option<Terminal>,
}
#[derive(Debug)]
pub struct Doorbell {
    channel: Channel,
    owner: Identity,
    producer: Identity,
    consumer: Identity,
    sequence: u64,
    terminal: Option<Terminal>,
    waiter: Option<Identity>,
}
impl Doorbell {
    /// Reservation is supplied by a bounded trusted owner; failure changes nothing.
    pub fn new(
        channel: Channel,
        owner: Identity,
        producer: Identity,
        consumer: Identity,
        reserved: bool,
    ) -> Result<Self, Error> {
        if !reserved {
            return Err(Error::Capacity);
        }
        if channel.generation == 0
            || [owner, producer, consumer]
                .iter()
                .any(|i| i.incarnation == 0)
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            channel,
            owner,
            producer,
            consumer,
            sequence: 0,
            terminal: None,
            waiter: None,
        })
    }
    fn check(&self, channel: Channel, caller: Identity, role: Identity) -> Result<(), Error> {
        if channel != self.channel {
            return Err(Error::Stale);
        }
        if caller != role {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
    fn open(&self) -> Result<(), Error> {
        self.terminal.map_or(Ok(()), |t| Err(Error::Terminal(t)))
    }
    pub fn snapshot(&self, channel: Channel, caller: Identity) -> Result<u64, Error> {
        self.check(channel, caller, self.consumer)?;
        self.open()?;
        Ok(self.sequence)
    }
    pub fn wait(
        &mut self,
        channel: Channel,
        caller: Identity,
        observed: u64,
    ) -> Result<Wait, Error> {
        self.check(channel, caller, self.consumer)?;
        self.open()?;
        if observed != self.sequence {
            return Ok(Wait::Changed(self.sequence));
        }
        if self.waiter.is_some() {
            return Err(Error::AlreadyWaiting);
        }
        self.waiter = Some(caller);
        Ok(Wait::Registered)
    }
    pub fn notify(&mut self, channel: Channel, caller: Identity) -> Result<Effect, Error> {
        self.check(channel, caller, self.producer)?;
        self.open()?;
        match self.sequence.checked_add(1) {
            Some(next) => {
                self.sequence = next;
                Ok(Effect {
                    wake: self.waiter.take(),
                    terminal: None,
                })
            }
            None => Ok(self.finish(Terminal::Exhausted)),
        }
    }
    fn finish(&mut self, reason: Terminal) -> Effect {
        let terminal = *self.terminal.get_or_insert(reason);
        Effect {
            wake: self.waiter.take(),
            terminal: Some(terminal),
        }
    }
    pub fn close(&mut self, channel: Channel, caller: Identity) -> Result<Effect, Error> {
        self.check(channel, caller, self.owner)?;
        Ok(self.finish(Terminal::Closed))
    }
    pub fn revoke(&mut self, channel: Channel, caller: Identity) -> Result<Effect, Error> {
        self.check(channel, caller, self.owner)?;
        Ok(self.finish(Terminal::Revoked))
    }
    /// Trusted lifecycle event, not callable by an untrusted peer.
    pub fn peer_exit(&mut self, identity: Identity) -> Effect {
        if identity == self.consumer {
            self.waiter = None; // dead task is never made runnable
            self.finish(Terminal::ConsumerExit)
        } else if identity == self.producer {
            self.finish(Terminal::ProducerExit)
        } else {
            Effect {
                wake: None,
                terminal: self.terminal,
            }
        }
    }
    /// Serialized cancellation/spurious scheduler wake removes the registration.
    pub fn cancel(&mut self, channel: Channel, caller: Identity) -> Result<bool, Error> {
        self.check(channel, caller, self.consumer)?;
        Ok(self.waiter.take().is_some())
    }
    pub fn has_waiter(&self) -> bool {
        self.waiter.is_some()
    }

    #[cfg(all(debug_assertions, feature = "fixture"))]
    pub fn fixture_sequence(&mut self, sequence: u64) {
        assert!(!self.has_waiter());
        assert!(self.terminal.is_none());
        self.sequence = sequence;
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
