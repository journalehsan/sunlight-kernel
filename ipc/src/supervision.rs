//! Bounded ownership mechanism, used only under the kernel process-table lock.
//! It deliberately contains no service graph, lease policy or adoption operation.
pub const CAPACITY: usize = 64;
pub const MAX_LAUNCHES: u64 = 4;
pub const ACTIVE: u64 = 0;
pub const EXITING: u64 = 1;
pub const REAPED: u64 = 2;
pub const BLOCKED: u64 = 3;
pub const STATUS: u64 = 0xD360_0001;
pub const RECORD: u64 = 0xD360_0002;
pub const DETAIL: u64 = 0xD360_0003;
pub const ACCOUNT: u64 = 0xD360_0004;
pub const PATH: u64 = 0xD360_0005;
pub const RPC_STATUS: u64 = 0xD360;
pub const RPC_RECORD: u64 = 0xD361;
pub const RPC_DETAIL: u64 = 0xD362;
pub const RPC_ACCOUNT: u64 = 0xD363;
pub const RPC_PATH: u64 = 0xD364;
pub const TEST_ARM: u64 = 0xD36D;
pub const TEST_PROBE: u64 = 0xD36E;
pub const TEST_CRASH: u64 = 0xD36F;
pub const ERROR_BLOCKED: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub pid: u64,
    pub generation: u64,
}
impl Identity {
    pub const fn valid(self) -> bool {
        self.pid != 0 && self.generation != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: u64,
    pub process: Identity,
    pub owner_epoch: u64,
    pub owner: Identity,
    pub path: [u64; 4],
    pub tracked: bool,
    /// Private frames, page tables, PMM free frames at exact reaping.
    pub reap: Option<[u64; 3]>,
}
pub struct Supervisor {
    bootstrap: Option<Identity>,
    manager: Option<Identity>,
    reaped: bool,
    epoch: u64,
    attempts: u64,
    next_record: u64,
    records: [Option<Record>; CAPACITY],
}
impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}
impl Supervisor {
    pub const fn new() -> Self {
        Self {
            bootstrap: None,
            manager: None,
            reaped: false,
            epoch: 0,
            attempts: 0,
            next_record: 0,
            records: [None; CAPACITY],
        }
    }
    pub fn install_bootstrap(&mut self, identity: Identity) -> bool {
        if self.bootstrap.is_some() || !identity.valid() {
            return false;
        }
        self.bootstrap = Some(identity);
        true
    }
    pub fn is_bootstrap(&self, identity: Identity) -> bool {
        self.bootstrap == Some(identity)
    }
    pub fn manager(&self) -> Option<Identity> {
        self.manager
    }
    pub fn current(&self, identity: Identity) -> bool {
        self.manager == Some(identity) && !self.reaped
    }
    pub fn active(&self, identity: Identity) -> bool {
        self.current(identity) && self.epoch == 1
    }
    pub fn readable(&self, identity: Identity) -> bool {
        self.is_bootstrap(identity) || self.current(identity)
    }
    /// Called before the spawn attempt; failed loads consume budget too.
    pub fn attempt(&mut self, caller: Identity) -> bool {
        if !self.is_bootstrap(caller)
            || (self.manager.is_some() && !self.reaped)
            || self.attempts >= MAX_LAUNCHES
        {
            return false;
        }
        self.attempts += 1;
        true
    }
    /// Must follow a successful attempt under the same existing kernel lock.
    pub fn grant(&mut self, identity: Identity) {
        assert!(identity.valid() && self.attempts > self.epoch);
        assert!(self.manager.is_none() || self.reaped);
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("supervisor epoch exhausted");
        self.manager = Some(identity);
        self.reaped = false;
    }
    pub fn manager_reaped(&mut self, identity: Identity) {
        if self.manager == Some(identity) {
            self.reaped = true;
        }
    }
    pub fn status(&self) -> [u64; 4] {
        [
            if self.manager.is_none() || self.reaped {
                REAPED
            } else if self.epoch == 1 {
                ACTIVE
            } else {
                BLOCKED
            },
            self.epoch,
            self.records.iter().flatten().count() as u64,
            self.attempts,
        ]
    }
    pub fn can_spawn(&self, caller: Identity) -> bool {
        self.active(caller)
            && self.next_record < u64::MAX
            && self.records.iter().any(Option::is_none)
    }
    pub fn spawned(&mut self, caller: Identity, process: Identity, path: [u64; 4], tracked: bool) {
        assert!(self.can_spawn(caller) && process.valid());
        self.next_record += 1;
        *self.records.iter_mut().find(|r| r.is_none()).unwrap() = Some(Record {
            id: self.next_record,
            process,
            owner_epoch: self.epoch,
            owner: caller,
            path,
            tracked,
            reap: None,
        });
    }
    pub fn record(&self, index: usize) -> Option<Record> {
        self.records.get(index).copied().flatten()
    }
    pub fn by_id(&self, id: u64) -> Option<Record> {
        self.records.iter().flatten().find(|r| r.id == id).copied()
    }
    pub fn reaped(&mut self, process: Identity, receipt: [u64; 3], owner_live: bool) {
        let active_epoch = self.epoch == 1 && !self.reaped && owner_live;
        for slot in &mut self.records {
            if let Some(r) = slot.as_mut().filter(|r| r.process == process) {
                if active_epoch && r.owner_epoch == self.epoch && !r.tracked {
                    *slot = None;
                } else {
                    r.reap = Some(receipt);
                }
            }
        }
    }
    pub fn release(&mut self, caller: Identity, process: Identity) -> bool {
        if !self.active(caller) {
            return false;
        }
        for slot in &mut self.records {
            if slot.is_some_and(|r| {
                r.process == process
                    && r.owner_epoch == self.epoch
                    && r.owner == caller
                    && r.reap.is_some()
            }) {
                *slot = None;
                return true;
            }
        }
        false
    }
}

/// Complete syscall surface for diagnostic successors. IPC call admission is
/// further restricted to registering their own endpoint by kernel IPC mediation.
pub const fn diagnostic_syscall_allowed(number: u64) -> bool {
    matches!(number, 1..=4 | 7 | 8 | 10..=13 | 18 | 20 | 21 | 25 | 33..=36 | 81 | 86 | 99 | 140 | 141)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootstrapAction {
    Wait,
    Launch,
    Exhausted,
    RegistrationTimeout,
    ReapTimeout,
}
#[derive(Default)]
pub struct BootstrapMonitor {
    retry_at: Option<u64>,
    registration_deadline: Option<u64>,
    reap_deadline: Option<u64>,
    terminal: bool,
}
impl BootstrapMonitor {
    pub fn step(&mut self, now: u64, status: [u64; 4], registered: bool) -> BootstrapAction {
        if self.terminal {
            return BootstrapAction::Wait;
        }
        if status[0] == EXITING {
            self.retry_at = None;
            self.registration_deadline = None;
            let deadline = *self.reap_deadline.get_or_insert(now.saturating_add(5000));
            if now >= deadline {
                self.terminal = true;
                return BootstrapAction::ReapTimeout;
            }
            return BootstrapAction::Wait;
        }
        self.reap_deadline = None;
        if status[0] == REAPED {
            self.registration_deadline = None;
            if status[3] >= MAX_LAUNCHES {
                self.terminal = true;
                return BootstrapAction::Exhausted;
            }
            let delay = match status[3] {
                0 | 1 => 250,
                2 => 1000,
                _ => 4000,
            };
            let deadline = *self.retry_at.get_or_insert(now.saturating_add(delay));
            if now >= deadline {
                self.retry_at = None;
                return BootstrapAction::Launch;
            }
        } else {
            self.retry_at = None;
            if registered {
                self.registration_deadline = None;
            } else {
                let deadline = *self
                    .registration_deadline
                    .get_or_insert(now.saturating_add(5000));
                if now >= deadline {
                    self.terminal = true;
                    return BootstrapAction::RegistrationTimeout;
                }
            }
        }
        BootstrapAction::Wait
    }
}

/// Diagnostic-only access to the current grant; no ability to transfer records.
pub fn query(operation: u64, selector: u64) -> Option<[u64; 4]> {
    let (ret, msg) = unsafe {
        crate::raw_syscall(
            crate::SunlightSyscall::SessionGetCredentials,
            selector,
            operation,
            0,
            0,
            0,
            0,
            0,
        )
    };
    (ret == 0).then_some([msg.words[0], msg.words[1], msg.words[2], msg.words[3]])
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    fn id(pid: u64, generation: u64) -> Identity {
        Identity { pid, generation }
    }
    fn setup() -> Supervisor {
        let mut s = Supervisor::new();
        assert!(s.install_bootstrap(id(1, 10)));
        assert!(s.attempt(id(1, 10)));
        s.grant(id(7, 20));
        s
    }
    #[test]
    fn bootstrap_is_generation_qualified_and_immutable() {
        let mut s = setup();
        assert!(!s.install_bootstrap(id(1, 11)));
        for i in [id(1, 11), id(7, 20), id(8, 10)] {
            assert!(!s.attempt(i));
        }
    }
    #[test]
    fn live_or_unreaped_manager_excludes_startup() {
        let mut s = setup();
        assert!(!s.attempt(id(1, 10)));
        assert_eq!(s.status()[3], 1);
        s.manager_reaped(id(7, 21));
        assert!(!s.attempt(id(1, 10)));
        s.manager_reaped(id(7, 20));
        assert!(s.attempt(id(1, 10)));
    }
    #[test]
    fn concurrent_bootstrap_attempts_have_one_winner() {
        let mut s = Supervisor::new();
        s.install_bootstrap(id(1, 10));
        let shared = Arc::new(Mutex::new(s));
        let barrier = Arc::new(Barrier::new(3));
        let threads: std::vec::Vec<_> = (0..2)
            .map(|i| {
                let s = shared.clone();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    let mut s = s.lock().unwrap();
                    if s.attempt(id(1, 10)) {
                        s.grant(id(7 + i, 20 + i));
                        true
                    } else {
                        false
                    }
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(
            threads
                .into_iter()
                .filter_map(|t| t.join().ok())
                .filter(|v| *v)
                .count(),
            1
        );
    }
    #[test]
    fn successor_has_metadata_but_no_lifecycle_authority() {
        let mut s = setup();
        s.spawned(id(7, 20), id(8, 30), [1; 4], true);
        s.manager_reaped(id(7, 20));
        assert!(s.attempt(id(1, 10)));
        s.grant(id(9, 40));
        assert!(s.readable(id(9, 40)));
        assert!(!s.active(id(9, 40)));
        assert!(!s.active(id(7, 20)));
        assert!(!s.can_spawn(id(9, 40)));
        assert!(!s.release(id(9, 40), id(8, 30)));
        assert_eq!(s.record(0).unwrap().owner_epoch, 1);
        assert_eq!(s.status()[0], BLOCKED);
    }
    #[test]
    fn exact_reaping_survives_manager_and_process_slot_reuse() {
        let mut s = setup();
        s.spawned(id(7, 20), id(8, 30), [2; 4], false);
        s.manager_reaped(id(7, 20));
        s.reaped(id(8, 31), [1, 2, 3], false);
        assert!(s.record(0).unwrap().reap.is_none());
        s.reaped(id(8, 30), [4, 5, 6], false);
        assert_eq!(s.record(0).unwrap().reap, Some([4, 5, 6]));
    }
    #[test]
    fn tracked_release_requires_reap_and_original_authority() {
        let mut s = setup();
        s.spawned(id(7, 20), id(8, 30), [0; 4], true);
        assert!(!s.release(id(7, 20), id(8, 30)));
        s.reaped(id(8, 30), [1; 3], true);
        assert!(!s.release(id(7, 21), id(8, 30)));
        assert!(s.release(id(7, 20), id(8, 30)));
        s.spawned(id(7, 20), id(8, 31), [0; 4], true);
        assert_eq!(s.record(0).unwrap().id, 2);
        assert!(s.by_id(1).is_none());
    }
    #[test]
    fn bounded_capacity_rejects_before_spawn() {
        let mut s = setup();
        for i in 0..CAPACITY {
            s.spawned(id(7, 20), id(100 + i as u64, 30), [0; 4], true);
        }
        assert!(!s.can_spawn(id(7, 20)));
        assert_eq!(s.status()[2], CAPACITY as u64);
    }
    #[test]
    fn failed_launches_consume_budget_and_never_reset() {
        let mut s = setup();
        s.manager_reaped(id(7, 20));
        for _ in 0..3 {
            assert!(s.attempt(id(1, 10)));
        }
        assert!(!s.attempt(id(1, 10)));
        assert_eq!(s.status()[3], MAX_LAUNCHES);
    }
    #[test]
    fn manager_death_before_service_reap_retains_untracked_record() {
        let mut s = setup();
        s.spawned(id(7, 20), id(8, 30), [0; 4], false);
        s.reaped(id(8, 30), [1; 3], false);
        assert!(s.record(0).is_some());
    }
    #[test]
    fn live_original_manager_releases_untracked_reap() {
        let mut s = setup();
        s.spawned(id(7, 20), id(8, 30), [0; 4], false);
        s.reaped(id(8, 30), [1; 3], true);
        assert!(s.record(0).is_none());
    }
    #[test]
    fn same_pid_different_manager_generation_never_inherits_authority() {
        let mut s = setup();
        s.spawned(id(7, 20), id(8, 30), [0; 4], true);
        s.manager_reaped(id(7, 20));
        assert!(s.attempt(id(1, 10)));
        s.grant(id(7, 21));
        assert!(!s.readable(id(7, 20)));
        assert!(s.readable(id(7, 21)));
        assert!(!s.active(id(7, 21)));
        assert_eq!(s.record(0).unwrap().owner, id(7, 20));
    }
    #[test]
    fn release_racing_manager_reap_is_atomic_and_successor_stays_blocked() {
        for _ in 0..32 {
            let mut s = setup();
            s.spawned(id(7, 20), id(8, 30), [0; 4], true);
            s.reaped(id(8, 30), [1; 3], true);
            let shared = Arc::new(Mutex::new(s));
            let b = Arc::new(Barrier::new(3));
            let r = shared.clone();
            let br = b.clone();
            let release = std::thread::spawn(move || {
                br.wait();
                r.lock().unwrap().release(id(7, 20), id(8, 30))
            });
            let f = shared.clone();
            let bf = b.clone();
            let fence = std::thread::spawn(move || {
                bf.wait();
                f.lock().unwrap().manager_reaped(id(7, 20));
            });
            b.wait();
            let released = release.join().unwrap();
            fence.join().unwrap();
            let mut s = shared.lock().unwrap();
            assert_eq!(s.record(0).is_none(), released);
            assert!(s.attempt(id(1, 10)));
            s.grant(id(9, 40));
            assert!(!s.active(id(9, 40)));
        }
    }
    #[test]
    fn bootstrap_backoff_does_not_reset_or_restart_an_unreaped_manager() {
        let mut m = BootstrapMonitor::default();
        assert_eq!(m.step(0, [REAPED, 1, 1, 1], false), BootstrapAction::Wait);
        assert_eq!(m.step(249, [REAPED, 1, 1, 1], false), BootstrapAction::Wait);
        assert_eq!(
            m.step(250, [REAPED, 1, 1, 1], false),
            BootstrapAction::Launch
        );
        assert_eq!(m.step(251, [BLOCKED, 2, 1, 2], true), BootstrapAction::Wait);
        assert_eq!(m.step(500, [REAPED, 2, 1, 2], false), BootstrapAction::Wait);
        assert_eq!(
            m.step(1499, [REAPED, 2, 1, 2], false),
            BootstrapAction::Wait
        );
        assert_eq!(
            m.step(1500, [REAPED, 2, 1, 2], false),
            BootstrapAction::Launch
        );
    }
    #[test]
    fn registration_timeout_is_terminal_and_never_kills_or_retries_live_owner() {
        let mut m = BootstrapMonitor::default();
        assert_eq!(
            m.step(100, [BLOCKED, 2, 4, 2], false),
            BootstrapAction::Wait
        );
        assert_eq!(
            m.step(5099, [BLOCKED, 2, 4, 2], false),
            BootstrapAction::Wait
        );
        assert_eq!(
            m.step(5100, [BLOCKED, 2, 4, 2], false),
            BootstrapAction::RegistrationTimeout
        );
        assert_eq!(
            m.step(9000, [REAPED, 2, 4, 2], false),
            BootstrapAction::Wait
        );
    }
    #[test]
    fn bootstrap_budget_exhaustion_is_terminal() {
        let mut m = BootstrapMonitor::default();
        assert_eq!(
            m.step(0, [REAPED, 4, 4, 4], false),
            BootstrapAction::Exhausted
        );
        assert_eq!(
            m.step(5000, [REAPED, 4, 4, 1], false),
            BootstrapAction::Wait
        );
    }
    #[test]
    fn diagnostic_successor_has_no_alternate_mutating_syscall_path() {
        for op in [
            5, 6, 14, 15, 16, 17, 22, 23, 24, 30, 31, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
            50, 51, 52, 60, 72, 85, 90, 92, 93, 94, 97, 100, 102, 105, 118, 131, 132, 138, 151,
            152, 158, 1000, 1016,
        ] {
            assert!(
                !diagnostic_syscall_allowed(op),
                "forbidden diagnostic syscall {}",
                op
            );
        }
        for op in [
            1, 2, 3, 4, 7, 8, 10, 11, 12, 13, 18, 20, 21, 33, 86, 99, 140, 141,
        ] {
            assert!(diagnostic_syscall_allowed(op));
        }
    }
    #[test]
    fn unreaped_manager_timeout_is_terminal_without_replacement() {
        let mut m = BootstrapMonitor::default();
        assert_eq!(m.step(0, [EXITING, 1, 4, 1], true), BootstrapAction::Wait);
        assert_eq!(
            m.step(4999, [EXITING, 1, 4, 1], false),
            BootstrapAction::Wait
        );
        assert_eq!(
            m.step(5000, [EXITING, 1, 4, 1], false),
            BootstrapAction::ReapTimeout
        );
        assert_eq!(
            m.step(6000, [REAPED, 1, 4, 1], false),
            BootstrapAction::Wait
        );
    }
}
