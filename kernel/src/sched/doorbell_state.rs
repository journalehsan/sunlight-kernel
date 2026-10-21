use sunlight_fastlane_doorbell::{Channel, Doorbell, Error, Identity, Terminal, Wait};

pub const CAPACITY: usize = 4;
const KERNEL: Identity = Identity {
    pid: 0,
    incarnation: 1,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(Channel);

impl Handle {
    pub fn channel(self) -> Channel {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub handle: Handle,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Notified(u64),
    Terminal(Terminal),
    Cancelled,
    Spurious,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    pub token: Token,
    pub outcome: Option<Outcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionalWait {
    Changed(u64),
    Registered(Token),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub identity: Identity,
    pub token: Token,
    pub outcome: Outcome,
}

struct Entry {
    model: Doorbell,
    consumer: Identity,
    token: Option<Token>,
}

impl Entry {
    fn complete(&mut self, wake: Option<Identity>, outcome: Outcome) -> Option<Completion> {
        let token = self.token.take();
        assert_eq!(wake.is_some(), token.is_some());
        wake.map(|identity| Completion {
            identity,
            token: token.unwrap(),
            outcome,
        })
    }
}

pub struct Table {
    entries: [Option<Entry>; CAPACITY],
    generation: u64,
    epoch: u64,
    pub created: u64,
    pub retired: u64,
    pub notifications: u64,
    pub completions: u64,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; CAPACITY],
            generation: 0,
            epoch: 0,
            created: 0,
            retired: 0,
            notifications: 0,
            completions: 0,
        }
    }

    fn entry(&self, handle: Handle) -> Result<&Entry, Error> {
        let entry = self
            .entries
            .get(handle.0.slot as usize)
            .and_then(Option::as_ref)
            .ok_or(Error::Stale)?;
        if entry.model.snapshot(handle.0, entry.consumer) == Err(Error::Stale) {
            return Err(Error::Stale);
        }
        Ok(entry)
    }

    fn entry_mut(&mut self, handle: Handle) -> Result<&mut Entry, Error> {
        self.entry(handle)?;
        Ok(self.entries[handle.0.slot as usize].as_mut().unwrap())
    }

    pub fn create(&mut self, consumer: Identity) -> Result<Handle, Error> {
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let generation = self.generation.checked_add(1).ok_or(Error::Capacity)?;
        let handle = Handle(Channel {
            slot: slot as u64,
            generation,
        });
        let model = Doorbell::new(handle.0, KERNEL, KERNEL, consumer, true)?;
        self.entries[slot] = Some(Entry {
            model,
            consumer,
            token: None,
        });
        self.generation = generation;
        self.created += 1;
        Ok(handle)
    }

    pub fn snapshot(&self, handle: Handle, caller: Identity) -> Result<u64, Error> {
        self.entry(handle)?.model.snapshot(handle.0, caller)
    }

    pub fn wait(
        &mut self,
        handle: Handle,
        caller: Identity,
        observed: u64,
    ) -> Result<ConditionalWait, Error> {
        let epoch = self.epoch.checked_add(1).ok_or(Error::Capacity)?;
        let entry = self.entry_mut(handle)?;
        match entry.model.wait(handle.0, caller, observed)? {
            Wait::Changed(sequence) => Ok(ConditionalWait::Changed(sequence)),
            Wait::Registered => {
                let token = Token { handle, epoch };
                entry.token = Some(token);
                self.epoch = epoch;
                Ok(ConditionalWait::Registered(token))
            }
        }
    }

    pub fn notify(&mut self, handle: Handle) -> Result<Option<Completion>, Error> {
        let entry = self.entry_mut(handle)?;
        let effect = entry.model.notify(handle.0, KERNEL)?;
        let outcome = effect.terminal.map_or_else(
            || Outcome::Notified(entry.model.snapshot(handle.0, entry.consumer).unwrap()),
            Outcome::Terminal,
        );
        let completion = entry.complete(effect.wake, outcome);
        self.notifications = self.notifications.saturating_add(1);
        self.completions = self
            .completions
            .saturating_add(u64::from(completion.is_some()));
        Ok(completion)
    }

    pub fn close(&mut self, handle: Handle) -> Result<Option<Completion>, Error> {
        let entry = self.entry_mut(handle)?;
        let effect = entry.model.close(handle.0, KERNEL)?;
        let completion = entry.complete(effect.wake, Outcome::Terminal(effect.terminal.unwrap()));
        self.completions = self
            .completions
            .saturating_add(u64::from(completion.is_some()));
        Ok(completion)
    }

    pub fn cancel(
        &mut self,
        token: Token,
        caller: Identity,
        outcome: Outcome,
    ) -> Result<Option<Completion>, Error> {
        assert!(matches!(outcome, Outcome::Cancelled | Outcome::Spurious));
        let entry = self.entry_mut(token.handle)?;
        if caller != entry.consumer {
            return Err(Error::Unauthorized);
        }
        if entry.token != Some(token) {
            return Ok(None);
        }
        assert!(entry.model.cancel(token.handle.0, caller)?);
        let completion = entry.complete(Some(caller), outcome);
        self.completions = self.completions.saturating_add(1);
        Ok(completion)
    }

    pub fn peer_exit(&mut self, identity: Identity) -> [Option<Completion>; CAPACITY] {
        let mut completions = [None; CAPACITY];
        for (slot, entry) in self.entries.iter_mut().enumerate() {
            let Some(entry) = entry else { continue };
            let effect = entry.model.peer_exit(identity);
            if identity == entry.consumer {
                entry.token = None;
                assert!(effect.wake.is_none());
            } else if effect.wake.is_some() {
                completions[slot] =
                    entry.complete(effect.wake, Outcome::Terminal(effect.terminal.unwrap()));
                self.completions = self.completions.saturating_add(1);
            }
        }
        completions
    }

    pub fn producer_exit(&mut self, handle: Handle) -> Result<Option<Completion>, Error> {
        let entry = self.entry_mut(handle)?;
        let effect = entry.model.peer_exit(KERNEL);
        let completion = entry.complete(effect.wake, Outcome::Terminal(effect.terminal.unwrap()));
        self.completions = self
            .completions
            .saturating_add(u64::from(completion.is_some()));
        Ok(completion)
    }

    pub fn retire(&mut self, handle: Handle) -> Result<(), Error> {
        let entry = self.entry(handle)?;
        if entry.token.is_some() {
            return Err(Error::AlreadyWaiting);
        }
        if !matches!(
            entry.model.snapshot(handle.0, entry.consumer),
            Err(Error::Terminal(_))
        ) {
            return Err(Error::Invalid);
        }
        self.entries[handle.0.slot as usize] = None;
        self.retired += 1;
        Ok(())
    }

    pub fn active(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub fn waiters(&self) -> usize {
        self.entries
            .iter()
            .flatten()
            .filter(|entry| entry.token.is_some())
            .count()
    }

    #[cfg(all(debug_assertions, feature = "native_doorbell_test"))]
    pub fn fixture_sequence(&mut self, handle: Handle, sequence: u64) {
        self.entry_mut(handle)
            .unwrap()
            .model
            .fixture_sequence(sequence);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CONSUMER: Identity = Identity {
        pid: 9,
        incarnation: 31,
    };

    #[test]
    fn stale_snapshot_and_exclusive_waiter() {
        let mut table = Table::new();
        let handle = table.create(CONSUMER).unwrap();
        assert_eq!(table.snapshot(handle, CONSUMER), Ok(0));
        assert_eq!(table.notify(handle), Ok(None));
        assert_eq!(
            table.wait(handle, CONSUMER, 0),
            Ok(ConditionalWait::Changed(1))
        );
        let ConditionalWait::Registered(token) = table.wait(handle, CONSUMER, 1).unwrap() else {
            panic!()
        };
        assert_eq!(table.wait(handle, CONSUMER, 1), Err(Error::AlreadyWaiting));
        assert_eq!(table.retire(handle), Err(Error::AlreadyWaiting));
        assert_eq!(
            table.notify(handle).unwrap(),
            Some(Completion {
                identity: CONSUMER,
                token,
                outcome: Outcome::Notified(2)
            })
        );
        assert_eq!(table.notify(handle), Ok(None));
        assert_eq!(table.waiters(), 0);
    }

    #[test]
    fn cancel_epoch_cannot_withdraw_a_new_registration() {
        let mut table = Table::new();
        let handle = table.create(CONSUMER).unwrap();
        let ConditionalWait::Registered(old) = table.wait(handle, CONSUMER, 0).unwrap() else {
            panic!()
        };
        assert!(table
            .cancel(old, CONSUMER, Outcome::Cancelled)
            .unwrap()
            .is_some());
        let ConditionalWait::Registered(new) = table.wait(handle, CONSUMER, 0).unwrap() else {
            panic!()
        };
        assert_ne!(old, new);
        assert_eq!(table.cancel(old, CONSUMER, Outcome::Cancelled), Ok(None));
        assert_eq!(table.waiters(), 1);
        assert_eq!(table.close(handle).unwrap().unwrap().token, new);
        assert_eq!(table.notify(handle), Err(Error::Terminal(Terminal::Closed)));
        assert_eq!(table.close(handle), Ok(None));
    }

    #[test]
    fn task_and_object_reuse_reject_stale_effects() {
        let mut table = Table::new();
        for generation in 1..=1000 {
            let consumer = Identity {
                incarnation: generation,
                ..CONSUMER
            };
            let handle = table.create(consumer).unwrap();
            assert_eq!(handle.channel().generation, generation);
            assert!(matches!(
                table.wait(handle, consumer, 0),
                Ok(ConditionalWait::Registered(_))
            ));
            let stale = Identity {
                incarnation: generation + 1,
                ..consumer
            };
            assert_eq!(table.snapshot(handle, stale), Err(Error::Unauthorized));
            assert!(table.peer_exit(stale).iter().all(Option::is_none));
            assert_eq!(table.waiters(), 1);
            assert!(table.peer_exit(consumer).iter().all(Option::is_none));
            assert_eq!(table.waiters(), 0);
            assert_eq!(
                table.notify(handle),
                Err(Error::Terminal(Terminal::ConsumerExit))
            );
            table.retire(handle).unwrap();
            assert_eq!(table.notify(handle), Err(Error::Stale));
        }
        assert_eq!(table.created, table.retired);
        assert_eq!(table.active(), 0);
    }

    #[test]
    fn capacity_failure_and_generation_exhaustion_do_not_mutate() {
        let mut table = Table::new();
        for _ in 0..CAPACITY {
            table.create(CONSUMER).unwrap();
        }
        assert_eq!(table.create(CONSUMER), Err(Error::Capacity));
        assert_eq!(table.created, CAPACITY as u64);
        let mut table = Table::new();
        table.generation = u64::MAX;
        assert_eq!(table.create(CONSUMER), Err(Error::Capacity));
        assert_eq!(table.active(), 0);
        table.generation = 0;
        let handle = table.create(CONSUMER).unwrap();
        table.epoch = u64::MAX;
        assert_eq!(table.wait(handle, CONSUMER, 0), Err(Error::Capacity));
        assert_eq!(table.waiters(), 0);
    }

    #[test]
    fn terminal_exhaustion_wakes_exactly_once() {
        let mut table = Table::new();
        let handle = table.create(CONSUMER).unwrap();
        table.fixture_sequence(handle, u64::MAX);
        assert!(matches!(
            table.wait(handle, CONSUMER, u64::MAX),
            Ok(ConditionalWait::Registered(_))
        ));
        assert_eq!(
            table.notify(handle).unwrap().unwrap().outcome,
            Outcome::Terminal(Terminal::Exhausted)
        );
        assert_eq!(
            table.notify(handle),
            Err(Error::Terminal(Terminal::Exhausted))
        );
        assert_eq!(
            table.wait(handle, CONSUMER, u64::MAX),
            Err(Error::Terminal(Terminal::Exhausted))
        );
        table.close(handle).unwrap();
        table.retire(handle).unwrap();
    }

    #[test]
    fn producer_death_terminates_a_parked_consumer() {
        let mut table = Table::new();
        let handle = table.create(CONSUMER).unwrap();
        table.wait(handle, CONSUMER, 0).unwrap();
        assert_eq!(
            table.producer_exit(handle).unwrap().unwrap().outcome,
            Outcome::Terminal(Terminal::ProducerExit)
        );
        assert_eq!(table.waiters(), 0);
        table.retire(handle).unwrap();
    }
}
