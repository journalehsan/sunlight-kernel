//! Execute the production scope state/scan against a deterministic scheduler
//! fixture. Native gates separately exercise real scheduling and cleanup.
#![allow(dead_code)]
#[path = "../kernel/src/process/foreground.rs"]
mod foreground;

mod ipc {
    pub fn with_shard(_: u32, f: impl FnOnce(&mut ())) {
        f(&mut ())
    }
    pub fn handle_ipc_cancel(
        pid: usize,
        s: &mut crate::sched::Scheduler,
        _: &mut (),
    ) -> Result<(), ()> {
        s.cancelled_calls.push(pid);
        Ok(())
    }
}
mod sched {
    pub struct Call {
        pub endpoint_id: u32,
    }
    impl Copy for Call {}
    impl Clone for Call {
        fn clone(&self) -> Self {
            *self
        }
    }
    pub struct Process {
        pub pid: usize,
        pub ppid: usize,
        pub foreground: crate::foreground::Foreground,
        pub pending_call: Option<Call>,
        pub finished: bool,
    }
    impl Process {
        pub fn is_exiting(&self) -> bool {
            self.finished
        }
    }
    #[derive(Default)]
    pub struct Scheduler {
        pub processes: Vec<Process>,
        pub wakes: Vec<usize>,
        pub cancelled_calls: Vec<usize>,
    }
    impl Scheduler {
        pub fn wake_for_signal(&mut self, pid: usize) {
            self.wakes.push(pid);
        }
        pub fn live_owner_core(&self, _: usize) -> Option<usize> {
            None
        }
    }
    pub fn request_reschedule_on(_: usize) {}
}

fn process(pid: usize, member: u64) -> sched::Process {
    sched::Process {
        pid,
        ppid: 0,
        foreground: foreground::Foreground {
            member,
            ..Default::default()
        },
        pending_call: None,
        finished: false,
    }
}

#[test]
fn stale_process_slots_and_duplicate_cancel_cannot_target_new_work() {
    let mut owner = process(10, 0);
    let first = owner.foreground.begin().unwrap();
    owner.pending_call = Some(sched::Call { endpoint_id: 4 });
    let mut s = sched::Scheduler {
        processes: vec![owner, process(11, first), process(12, 0)],
        ..Default::default()
    };
    s.cancel_foreground(0, 2);
    assert_eq!(s.processes[1].foreground.forced_exit, Some(130));
    assert_eq!(s.processes[2].foreground.forced_exit, None);
    assert_eq!(s.cancelled_calls, [10]);
    let wakes = s.wakes.clone();
    s.cancel_foreground(0, 1);
    assert_eq!(s.wakes, wakes);
    assert_eq!(s.processes[0].foreground.cancel, 2);
    // Reuse the numeric PID/slot with fresh lifecycle state.
    s.processes[1] = process(11, 0);
    s.processes[0].foreground = Default::default();
    let second = s.processes[0].foreground.begin().unwrap();
    assert_ne!(first, second);
    s.cancel_foreground(0, 2);
    assert_eq!(s.processes[1].foreground.forced_exit, None);
}

#[test]
fn exit_race_and_descendants_preserve_already_recorded_status() {
    let mut owner = process(1, 0);
    let id = owner.foreground.begin().unwrap();
    let mut exited = process(2, id);
    exited.finished = true;
    let mut s = sched::Scheduler {
        processes: vec![owner, exited, process(3, id), process(4, id)],
        ..Default::default()
    };
    s.cancel_foreground(0, 1);
    assert_eq!(s.processes[1].foreground.forced_exit, None);
    assert_eq!(s.processes[2].foreground.forced_exit, Some(129));
    assert_eq!(s.processes[3].foreground.forced_exit, Some(129));
    assert_eq!(s.wakes, [1, 3, 4]);
}

#[test]
fn members_cannot_detach_and_scope_storage_is_bounded() {
    let mut member = process(3, 42);
    assert_eq!(member.foreground.execution(), 42);
    assert!(member.foreground.begin().is_none());
    let mut root = foreground::Foreground::default();
    let first = root.begin().unwrap();
    assert!(root.begin().is_none());
    root.owned = 0;
    assert_ne!(root.begin().unwrap(), first);
    assert!(std::mem::size_of::<foreground::Foreground>() <= 80);
}

#[test]
fn concurrent_scope_creation_never_reuses_identity() {
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                (0..128)
                    .map(|_| foreground::Foreground::default().begin().unwrap())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut ids: Vec<_> = threads
        .into_iter()
        .flat_map(|t| t.join().unwrap())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 1024);
}

#[test]
fn numeric_signals_respect_scope_and_parent_incarnation() {
    let mut owner = process(10, 0);
    owner.foreground.incarnation = 100;
    let scope = owner.foreground.begin().unwrap();
    let mut child = process(11, scope);
    child.ppid = 10;
    child.foreground.parent_generation = 100;
    let mut other = process(12, 0);
    other.foreground.incarnation = 200;
    let mut s = sched::Scheduler {
        processes: vec![owner, child, other],
        ..Default::default()
    };
    assert!(s.can_signal_foreground(0, 1));
    assert!(!s.can_signal_foreground(2, 1));
    assert!(!s.can_signal_foreground(1, 0));
    s.processes[0] = process(10, 0);
    s.processes[0].foreground.incarnation = 101;
    assert!(!s.can_signal_foreground(0, 1));
    s.processes[0].foreground.session_owner = true;
    assert!(!s.can_signal_foreground(2, 0));
}

#[test]
fn session_identity_is_qualified_by_broker_publication() {
    let f = foreground::Foreground {
        session_broker: 100,
        session: 1,
        generation: 7,
        ..Default::default()
    };
    assert!(f.matches_session(100, 1, 7));
    assert!(!f.matches_session(101, 1, 7));
    assert!(!f.matches_session(100, 1, 8));
    assert!(!f.matches_session(0, 1, 7));
    assert!(!f.matches_session(100, 1, 0));
}

#[test]
fn full_session_teardown_reaches_idle_and_reparented_members() {
    let mut owner = process(10, 0);
    owner.foreground.session_owner = true;
    owner.foreground.session_broker = 77;
    owner.foreground.session = 4;
    owner.foreground.generation = 9;
    let mut idle = process(11, 0);
    idle.foreground.session_broker = 77;
    idle.foreground.session = 4;
    idle.foreground.generation = 9;
    idle.ppid = 1; // the original shell parent is gone
    let mut unrelated = process(12, 0);
    unrelated.foreground.session_broker = 77;
    unrelated.foreground.session = 5;
    unrelated.foreground.generation = 9;
    let mut s = sched::Scheduler {
        processes: vec![owner, idle, unrelated],
        ..Default::default()
    };

    s.terminate_session(77, 4, 9, 1);
    assert_eq!(s.processes[0].foreground.forced_exit, Some(129));
    assert_eq!(s.processes[1].foreground.forced_exit, Some(129));
    assert_eq!(s.processes[2].foreground.forced_exit, None);
    assert_eq!(s.wakes, [10, 11]);
}
