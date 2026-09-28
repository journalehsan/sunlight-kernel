//! Core Linux eventfd counter semantics, shared with the kernel and host tests.
use alloc::vec::Vec;
use spin::Mutex;

pub const EFD_SEMAPHORE: u32 = 1;
pub const EFD_NONBLOCK: u32 = 0x800;
pub const EFD_CLOEXEC: u32 = 0x80000;
pub const MAX_COUNTER: u64 = u64::MAX - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventError {
    Invalid,
    WouldBlock,
}

pub struct EventFd {
    counter: u64,
    semaphore: bool,
    refs: usize,
}

impl EventFd {
    pub fn new(initval: u32, flags: u32) -> Result<Self, EventError> {
        if flags & !(EFD_SEMAPHORE | EFD_NONBLOCK | EFD_CLOEXEC) != 0 {
            return Err(EventError::Invalid);
        }
        Ok(Self {
            counter: initval as u64,
            semaphore: flags & EFD_SEMAPHORE != 0,
            refs: 0,
        })
    }

    pub fn read(&mut self) -> Result<u64, EventError> {
        if self.counter == 0 {
            return Err(EventError::WouldBlock);
        }
        let value = if self.semaphore { 1 } else { self.counter };
        self.counter -= value;
        Ok(value)
    }

    pub fn write(&mut self, value: u64) -> Result<(), EventError> {
        if value == u64::MAX {
            return Err(EventError::Invalid);
        }
        self.counter = self
            .counter
            .checked_add(value)
            .filter(|&n| n <= MAX_COUNTER)
            .ok_or(EventError::WouldBlock)?;
        Ok(())
    }

    pub fn readable(&self) -> bool {
        self.counter > 0
    }
    pub fn writable(&self) -> bool {
        self.counter < MAX_COUNTER
    }
    pub fn can_write(&self, value: u64) -> bool {
        value < u64::MAX
            && self
                .counter
                .checked_add(value)
                .is_some_and(|total| total <= MAX_COUNTER)
    }

    pub fn retain(&mut self) {
        self.refs = self
            .refs
            .checked_add(1)
            .expect("eventfd reference count overflow");
    }

    pub fn release(&mut self) -> bool {
        debug_assert!(self.refs > 0);
        self.refs -= 1;
        self.refs == 0
    }

    pub fn unused(&self) -> bool {
        self.refs == 0
    }
}

static EVENTS: Mutex<Vec<Option<EventFd>>> = Mutex::new(Vec::new());

pub fn create(initval: u32, flags: u32) -> Result<u32, EventError> {
    let event = EventFd::new(initval, flags)?;
    let mut pool = EVENTS.lock();
    if let Some((idx, slot)) = pool.iter_mut().enumerate().find(|(_, slot)| slot.is_none()) {
        *slot = Some(event);
        return Ok(idx as u32);
    }
    let idx = pool.len();
    if idx >= 0x0100_0000 {
        return Err(EventError::Invalid);
    }
    pool.push(Some(event));
    Ok(idx as u32)
}

pub fn retain(idx: u32) {
    if let Some(Some(event)) = EVENTS.lock().get_mut(idx as usize) {
        event.retain();
    }
}

pub fn release(idx: u32) {
    let mut pool = EVENTS.lock();
    if let Some(slot @ Some(_)) = pool.get_mut(idx as usize) {
        let event = slot.as_mut().unwrap();
        if event.unused() || event.release() {
            *slot = None;
        }
    }
}

pub fn read(idx: u32) -> Result<u64, EventError> {
    EVENTS
        .lock()
        .get_mut(idx as usize)
        .and_then(Option::as_mut)
        .ok_or(EventError::Invalid)?
        .read()
}

pub fn write(idx: u32, value: u64) -> Result<(), EventError> {
    EVENTS
        .lock()
        .get_mut(idx as usize)
        .and_then(Option::as_mut)
        .ok_or(EventError::Invalid)?
        .write(value)
}

pub fn readiness(idx: u32) -> (bool, bool) {
    EVENTS
        .lock()
        .get(idx as usize)
        .and_then(Option::as_ref)
        .map(|event| (event.readable(), event.writable()))
        .unwrap_or((false, false))
}

pub fn can_write(idx: u32, value: u64) -> bool {
    EVENTS
        .lock()
        .get(idx as usize)
        .and_then(Option::as_ref)
        .is_some_and(|event| event.can_write(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eventfd_create_zero_and_initial_value() {
        let empty = EventFd::new(0, 0).unwrap();
        assert!(!empty.readable());
        assert!(empty.writable());
        assert_eq!(EventFd::new(19, 0).unwrap().read(), Ok(19));
    }

    #[test]
    fn eventfd_read_resets_and_write_accumulates() {
        let mut event = EventFd::new(0, 0).unwrap();
        event.write(4).unwrap();
        event.write(3).unwrap();
        assert!(event.readable());
        assert_eq!(event.read(), Ok(7));
        assert!(!event.readable());
    }

    #[test]
    fn eventfd_semaphore_read_one() {
        let mut event = EventFd::new(2, EFD_SEMAPHORE).unwrap();
        assert_eq!(event.read(), Ok(1));
        assert_eq!(event.read(), Ok(1));
        assert_eq!(event.read(), Err(EventError::WouldBlock));
    }

    #[test]
    fn eventfd_invalid_flags_and_write_value() {
        assert!(matches!(EventFd::new(0, 0x1000), Err(EventError::Invalid)));
        let mut event = EventFd::new(0, EFD_NONBLOCK).unwrap();
        assert_eq!(event.read(), Err(EventError::WouldBlock));
        assert_eq!(event.write(u64::MAX), Err(EventError::Invalid));
    }

    #[test]
    fn eventfd_overflow_and_writable_readiness() {
        let mut event = EventFd::new(0, 0).unwrap();
        event.write(MAX_COUNTER).unwrap();
        assert!(!event.writable());
        assert!(!event.can_write(1));
        assert_eq!(event.write(1), Err(EventError::WouldBlock));
        event.read().unwrap();
        assert!(event.writable());
    }

    #[test]
    fn eventfd_duplicate_shared_state_and_close_lifetime() {
        let mut shared = EventFd::new(0, EFD_NONBLOCK).unwrap();
        shared.retain(); // first descriptor
        shared.retain(); // dup
        shared.write(3).unwrap();
        assert!(!shared.release()); // close first descriptor
        assert_eq!(shared.read(), Ok(3)); // duplicate sees the same counter
        assert!(shared.release()); // final close frees the object
    }

    #[test]
    fn pooled_eventfd_duplicate_survives_first_close() {
        let idx = create(0, EFD_NONBLOCK | EFD_CLOEXEC).unwrap();
        retain(idx);
        retain(idx); // duplicate descriptor uses the same index
        write(idx, 7).unwrap();
        assert_eq!(readiness(idx), (true, true));
        release(idx);
        assert_eq!(read(idx), Ok(7));
        release(idx);
        assert_eq!(readiness(idx), (false, false));
        assert_eq!(read(idx), Err(EventError::Invalid));
    }
}
