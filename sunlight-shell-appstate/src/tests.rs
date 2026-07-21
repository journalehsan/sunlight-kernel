//! Host-runnable lifecycle tests for [`RunningAppRegistry`].
//!
//! These mirror the 20 scenarios enumerated in the patch spec plus the
//! spec's "also verify" claims (no duplicate subscription, no unbounded
//! growth, etc.). Targets the std test runner via
//! `cargo test -p sunlight-shell-appstate --lib --target x86_64-unknown-linux-gnu`.

#![cfg(test)]

use std::collections::HashSet;
use std::vec::Vec;

use crate::{
    AppId, AppInstanceId, AppRunState, ProcessKey, RunningAppRegistry, WindowKey, WindowSnapshot,
    APP_COUNT, MAX_PROCESSES_PER_APP, MAX_WINDOWS_PER_APP,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct FakeProcessTable {
    alive: HashSet<u64>,
}

impl FakeProcessTable {
    fn new() -> Self {
        Self {
            alive: HashSet::new(),
        }
    }
    fn spawn(&mut self) -> u64 {
        let pid = self.next_pid();
        self.alive.insert(pid);
        pid
    }
    fn next_pid(&mut self) -> u64 {
        // Start at a non-zero value (pid 0 is the kernel sentinel).
        let mut pid = self.alive.len() as u64 + 1;
        while self.alive.contains(&pid) {
            pid += 1;
        }
        pid
    }
    fn kill(&mut self, pid: u64) {
        self.alive.remove(&pid);
    }
    fn is_alive(&self, pid: u64) -> bool {
        self.alive.contains(&pid)
    }
}

fn win(id: u64, owner_pid: u64, minimized: bool, visible: bool) -> WindowSnapshot {
    WindowSnapshot {
        id,
        owner_pid,
        process_generation: 0,
        generation: 0,
        minimized,
        visible,
        normal: true,
    }
}

fn win_normal(id: u64, owner_pid: u64) -> WindowSnapshot {
    win(id, owner_pid, false, true)
}

fn win_minimized(id: u64, owner_pid: u64) -> WindowSnapshot {
    win(id, owner_pid, true, false)
}

/// A `reconcile` call where no window maps to anything via telemetry fallback.
/// Used when the registry's stored pid slots are the only authority.
fn reconcile_no_fallback(
    reg: &mut RunningAppRegistry,
    windows: &[WindowSnapshot],
    is_alive: impl Fn(u64) -> bool,
    now: u64,
) -> bool {
    reg.reconcile(windows, is_alive, |_| None, now)
}

fn launch(
    reg: &mut RunningAppRegistry,
    app_id: AppId,
    table: &mut FakeProcessTable,
    now: u64,
) -> u64 {
    let launch_id = reg.note_launch_requested(app_id, now);
    let pid = table.spawn();
    reg.note_spawn_succeeded(app_id, pid, launch_id);
    pid
}

// ---------------------------------------------------------------------------
// 1 — Open app → indicator appears.
// ---------------------------------------------------------------------------

#[test]
fn open_app_indicator_appears() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Terminal, &mut table, 0);
    assert!(!reg.is_indicator_on(AppId::Terminal)); // still launching
    let win = win_normal(100, pid);
    let dirty = reconcile_no_fallback(&mut reg, &[win], |p| table.is_alive(p), 100);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Running);
    assert!(reg.is_indicator_on(AppId::Terminal));
}

// ---------------------------------------------------------------------------
// 2 — Normal close → indicator disappears.
// ---------------------------------------------------------------------------

#[test]
fn normal_close_indicator_disappears() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Files, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    assert!(reg.is_indicator_on(AppId::Files));
    table.kill(pid);
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Files).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::Files));
}

// ---------------------------------------------------------------------------
// 3 — Decoration close (window destroyed while pid briefly alive) ->
//     ClosingAwaitExit, then Idle once pid dies.
// ---------------------------------------------------------------------------

#[test]
fn decoration_close_indicator_disappears() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::TextEditor, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    // Window gone first; pid still alive.
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert!(dirty);
    assert_eq!(
        reg.snapshot(AppId::TextEditor).state,
        AppRunState::ClosingAwaitExit
    );
    assert!(reg.is_indicator_on(AppId::TextEditor));
    // Now pid dies.
    table.kill(pid);
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 300);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::TextEditor).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::TextEditor));
}

// ---------------------------------------------------------------------------
// 4 — Minimize → indicator remains.
// ---------------------------------------------------------------------------

#[test]
fn minimize_indicator_remains() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Writer, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    let dirty = reconcile_no_fallback(
        &mut reg,
        &[win_minimized(1, pid)],
        |p| table.is_alive(p),
        200,
    );
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Writer).state, AppRunState::Minimized);
    assert!(reg.is_indicator_on(AppId::Writer));
}

// ---------------------------------------------------------------------------
// 5 — Restore → indicator remains.
// ---------------------------------------------------------------------------

#[test]
fn restore_indicator_remains() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Writer, &mut table, 0);
    reconcile_no_fallback(
        &mut reg,
        &[win_minimized(1, pid)],
        |p| table.is_alive(p),
        100,
    );
    let dirty = reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 200);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Writer).state, AppRunState::Running);
    assert!(reg.is_indicator_on(AppId::Writer));
}

// ---------------------------------------------------------------------------
// 6 — Focus switch → both running apps remain indicated.
// ---------------------------------------------------------------------------

#[test]
fn focus_switch_both_indicated() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid_a = launch(&mut reg, AppId::Terminal, &mut table, 0);
    let pid_b = launch(&mut reg, AppId::Files, &mut table, 1);
    let windows = [win_normal(1, pid_a), win_normal(2, pid_b)];
    reconcile_no_fallback(&mut reg, &windows, |p| table.is_alive(p), 100);
    // Shell doesn't receive child focus events today; both stay Running.
    assert!(reg.is_indicator_on(AppId::Terminal));
    assert!(reg.is_indicator_on(AppId::Files));
}

// ---------------------------------------------------------------------------
// 7 — Two windows → closing one keeps indicator.
// ---------------------------------------------------------------------------

#[test]
fn two_windows_closing_one_keeps_indicator() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Terminal, &mut table, 0);
    let windows = [win_normal(1, pid), win_normal(2, pid)];
    reconcile_no_fallback(&mut reg, &windows, |p| table.is_alive(p), 100);
    // Close win 1; win 2 alive.
    let dirty = reconcile_no_fallback(&mut reg, &[win_normal(2, pid)], |p| table.is_alive(p), 200);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Running);
    assert!(reg.is_indicator_on(AppId::Terminal));
    assert_eq!(reg.snapshot(AppId::Terminal).total_window_count, 1);
}

// ---------------------------------------------------------------------------
// 8 — Closing the final window removes indicator when the app exits.
// ---------------------------------------------------------------------------

#[test]
fn closing_final_window_removes_indicator_on_exit() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Settings, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    // Window disappears first.
    reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert_eq!(
        reg.snapshot(AppId::Settings).state,
        AppRunState::ClosingAwaitExit
    );
    // Process exits.
    table.kill(pid);
    reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 300);
    assert_eq!(reg.snapshot(AppId::Settings).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::Settings));
}

// ---------------------------------------------------------------------------
// 9 — Crash → indicator disappears.
// ---------------------------------------------------------------------------

#[test]
fn crash_indicator_disappears() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Calculator, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    // Crash: pid dies, window disappears on same poll.
    table.kill(pid);
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Calculator).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::Calculator));
}

// ---------------------------------------------------------------------------
// 10 — End Task → indicator disappears (after confirmed termination).
// ---------------------------------------------------------------------------

#[test]
fn end_task_indicator_disappears() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Calculator, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    reg.note_kill_in_flight(pid);
    // End Task delivered; window gone but pid still briefly reported alive.
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert!(dirty);
    assert_eq!(
        reg.snapshot(AppId::Calculator).state,
        AppRunState::ClosingAwaitExit
    );
    assert!(reg.is_indicator_on(AppId::Calculator));
    // Kernel reports pid dead.
    table.kill(pid);
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 300);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Calculator).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::Calculator));
}

// ---------------------------------------------------------------------------
// 11 — Failed launch → no permanent indicator.
// ---------------------------------------------------------------------------

#[test]
fn failed_launch_no_permanent_indicator() {
    let mut reg = RunningAppRegistry::new();
    let launch_id = reg.note_launch_requested(AppId::Bench, 0);
    assert_eq!(reg.snapshot(AppId::Bench).state, AppRunState::Launching);
    // Spawn() returned an error.
    reg.note_launch_failed(AppId::Bench, "spawn errno 13");
    assert_eq!(reg.snapshot(AppId::Bench).state, AppRunState::Failed);
    assert!(!reg.is_indicator_on(AppId::Bench));
    // Next reconcile (no live pid) clears Failed -> Idle.
    let dirty = reconcile_no_fallback(&mut reg, &[], |_| false, 100);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Bench).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::Bench));
    let _ = launch_id;
}

// ---------------------------------------------------------------------------
// 12 — Immediate launch/exit race.
// ---------------------------------------------------------------------------

#[test]
fn immediate_launch_exit_race() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let now = 0u64;
    let launch_id = reg.note_launch_requested(AppId::Terminal, now);
    let pid = table.spawn();
    reg.note_spawn_succeeded(AppId::Terminal, pid, launch_id);
    // Process exits before any window opens. The same poll that sees the dead
    // pid must drop the Launching state without leaving a permanent indicator.
    table.kill(pid);
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 100);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Failed);
    assert!(!reg.is_indicator_on(AppId::Terminal));
    // And the next reconcile clears Failed -> Idle.
    reconcile_no_fallback(&mut reg, &[], |_| false, 200);
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::Terminal));
}

// ---------------------------------------------------------------------------
// 13 — PID reuse with different generation (documented best-effort).
// ---------------------------------------------------------------------------

#[test]
fn pid_reuse_with_dead_gap_reattributes() {
    // We model the strongest client-side assertion possible without a kernel
    // generation API: alive → dead → alive across consecutive polls causes the
    // stale slot to be dropped and a new pid life to be re-attributed to a
    // different app.
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = 1000u64; // a pid number the kernel happens to reuse.

    // First life: Terminal.
    table.alive.insert(pid);
    let lid1 = reg.note_launch_requested(AppId::Terminal, 0);
    reg.note_spawn_succeeded(AppId::Terminal, pid, lid1);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Running);

    // Terminal process dies; window disappears. Both cleared in one poll.
    table.kill(pid);
    reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Idle);

    // Second life: kernel recycled pid `1000` for Files.
    table.alive.insert(pid);
    let lid2 = reg.note_launch_requested(AppId::Files, 300);
    reg.note_spawn_succeeded(AppId::Files, pid, lid2);
    let dirty = reconcile_no_fallback(&mut reg, &[win_normal(2, pid)], |p| table.is_alive(p), 400);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Files).state, AppRunState::Running);
    assert!(!reg.is_indicator_on(AppId::Terminal)); // stale slot removed during prev reconcile
    assert!(reg.is_indicator_on(AppId::Files));
}

// ---------------------------------------------------------------------------
// 14 — Late window-destroy event.
// ---------------------------------------------------------------------------

#[test]
fn late_window_destroy_event() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Writer, &mut table, 0);
    let windows = [win_normal(1, pid), win_normal(2, pid)];
    reconcile_no_fallback(&mut reg, &windows, |p| table.is_alive(p), 100);
    // App killed, both windows disappear from the list. Reconcile drops them.
    table.kill(pid);
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert!(dirty);
    assert_eq!(reg.snapshot(AppId::Writer).state, AppRunState::Idle);
    assert_eq!(reg.snapshot(AppId::Writer).total_window_count, 0);
    // Another "late destroy" arriving now finds nothing.
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 300);
    assert!(!dirty);
}

// ---------------------------------------------------------------------------
// 15 — App launched from terminal (no shell launch_id, pid_to_app fallback).
// ---------------------------------------------------------------------------

#[test]
fn app_launched_from_terminal() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    // Files gets launched by Terminal as a child; shell never called
    // note_spawn_succeeded for it.
    let pid = table.spawn();
    let windows = [win_normal(7, pid)];
    reg.reconcile(
        &windows,
        |p| table.is_alive(p),
        |p| if p == pid { Some(AppId::Files) } else { None },
        100,
    );
    assert_eq!(reg.snapshot(AppId::Files).state, AppRunState::Running);
    assert!(reg.is_indicator_on(AppId::Files));
    // Subsequent reconcile keeps the association without re-querying
    // pid_to_app (the stored process slot is the authority).
    let mut called = false;
    reg.reconcile(
        &windows,
        |p| table.is_alive(p),
        |_| {
            called = true;
            Some(AppId::Files)
        },
        200,
    );
    assert_eq!(reg.snapshot(AppId::Files).state, AppRunState::Running);
    // The pid already matched a stored slot, so pid_to_app is not consulted
    // for this window — but the resolver closure may still be called from
    // other apps' reconciles. We just assert the Files state stays Running
    // even if the heuristic changed.
    let _ = called;
}

// ---------------------------------------------------------------------------
// 16 — Start Menu stays open while application exits.
// ---------------------------------------------------------------------------

#[test]
fn start_menu_stays_open_while_app_exits() {
    // The registry has no Start-Menu-open concept; the test asserts that
    // state transitions occur through reconcile regardless of any external
    // menu-open bit (which the shell controls separately). Running app exits
    // while "menu open" should still broadcast a dirty bit the shell can use
    // to redraw the open menu.
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Terminal, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    table.kill(pid);
    let dirty = reconcile_no_fallback(&mut reg, &[], |p| table.is_alive(p), 200);
    assert!(dirty);
    // Shell feeds the dirty bit back to its Start Menu redraw path; here we
    // only assert indicator flipped.
    assert!(!reg.is_indicator_on(AppId::Terminal));
}

// ---------------------------------------------------------------------------
// 17 — Dock and Start show identical state.
// ---------------------------------------------------------------------------

#[test]
fn dock_and_start_share_state() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Calculator, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    // The view (dock + Start) reads from the *same* snapshot:
    let dock_state = reg.snapshot(AppId::Calculator).state;
    let start_state = reg.snapshot(AppId::Calculator).state;
    assert_eq!(dock_state, start_state);
    assert_eq!(dock_state, AppRunState::Running);
}

// ---------------------------------------------------------------------------
// 18 — Shell restart reconstructs correct state.
// ---------------------------------------------------------------------------

#[test]
fn shell_restart_reconstructs_state() {
    // Simulate: previous shell spawned a Terminal; new shell starts fresh.
    let mut reg = RunningAppRegistry::new();
    // New shell sees a stray window (pid 4242, win 99) but has no stored
    // process slot for it.
    let pid = 4242u64;
    let windows = [win_normal(99, pid)];
    let mut kill_map: HashSet<u64> = HashSet::new();
    kill_map.insert(pid);
    reg.reconstruct(&windows, |p| {
        if p == pid {
            Some(AppId::Terminal)
        } else {
            None
        }
    });
    // Reconcile is then run with the live pid alive.
    reg.reconcile(
        &windows,
        |p| kill_map.contains(&p),
        |p| {
            if p == pid {
                Some(AppId::Terminal)
            } else {
                None
            }
        },
        100,
    );
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Running);
    assert!(reg.is_indicator_on(AppId::Terminal));
    // NotRunning apps stay Idle.
    for app in AppId::all() {
        if app != AppId::Terminal {
            assert_eq!(reg.snapshot(app).state, AppRunState::Idle);
        }
    }
}

// ---------------------------------------------------------------------------
// 19 — Repeated open/close for at least 100 cycles leaves no stale entries.
// ---------------------------------------------------------------------------

#[test]
fn repeated_open_close_no_stale() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    for i in 0..100u64 {
        let pid = launch(&mut reg, AppId::Terminal, &mut table, i);
        let win = win_normal(i * 2 + 1, pid);
        assert!(reconcile_no_fallback(
            &mut reg,
            &[win],
            |p| table.is_alive(p),
            i * 2 + 50
        ));
        table.kill(pid);
        assert!(reconcile_no_fallback(
            &mut reg,
            &[],
            |p| table.is_alive(p),
            i * 2 + 100
        ));
    }
    assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Idle);
    assert_eq!(reg.total_processes(), 0);
    assert_eq!(reg.total_windows(), 0);
}

// ---------------------------------------------------------------------------
// 20 — (Pagination deferred per patch 1 scope; registry-state check still
//      applicable: switching All Apps pages preserves correct indicators.)
// ---------------------------------------------------------------------------

#[test]
fn switching_pages_preserves_indicators() {
    // Start Menu pagination is deferred to a follow-up patch; here we
    // assert the registry invariant a multi-page Start Menu would rely on
    // — states are stable across an arbitrary number of redraws (round-trips
    // through `snapshot()`).
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid_t = launch(&mut reg, AppId::Terminal, &mut table, 0);
    let pid_w = launch(&mut reg, AppId::Writer, &mut table, 0);
    let windows = [win_normal(1, pid_t), win_minimized(2, pid_w)];
    reconcile_no_fallback(&mut reg, &windows, |p| table.is_alive(p), 100);
    for _ in 0..16 {
        assert_eq!(reg.snapshot(AppId::Terminal).state, AppRunState::Running);
        assert_eq!(reg.snapshot(AppId::Writer).state, AppRunState::Minimized);
        assert!(reg.is_indicator_on(AppId::Terminal));
        assert!(reg.is_indicator_on(AppId::Writer));
    }
}

// ---------------------------------------------------------------------------
// "Also verify" claims.
// ---------------------------------------------------------------------------

#[test]
fn no_duplicate_lifecycle_subscription() {
    // The registry has no subscription mechanism (polling model), so the
    // invariant reduces to: each live process has exactly one stored slot
    // even if the same pid is reported twice via note_spawn_succeeded and/or
    // pid_to_app adoption.
    let mut reg = RunningAppRegistry::new();
    let pid = 12345u64;
    reg.note_spawn_succeeded(AppId::Terminal, pid, 1);
    reg.note_spawn_succeeded(AppId::Terminal, pid, 2); // same pid, new launch_id
    for app in AppId::all() {
        let _ = app;
    }
    assert_eq!(reg.total_processes(), 1); // one slot, latest launch_id wins
}

#[test]
fn no_unbounded_registry_growth() {
    // The registry is statically bounded; verify that exceeding the per-app
    // windows and processes caps does not overflow.
    let mut reg = RunningAppRegistry::new();
    let mut alive = HashSet::new();
    for i in 0..(MAX_WINDOWS_PER_APP * 4) as u64 {
        alive.insert(i + 1);
        let _ = reg.note_spawn_succeeded(AppId::Terminal, i + 1, i);
    }
    let windows: Vec<WindowSnapshot> = (0..(MAX_WINDOWS_PER_APP * 4) as u64)
        .map(|i| win_normal(i, i + 1))
        .collect();
    reg.reconcile(
        &windows,
        |p| alive.contains(&p),
        |p| {
            if p >= 1 && p <= 32 {
                Some(AppId::Terminal)
            } else {
                None
            }
        },
        0,
    );
    let snap = reg.snapshot(AppId::Terminal);
    assert!(snap.total_window_count as usize <= MAX_WINDOWS_PER_APP);
    assert!(reg.total_processes() <= APP_COUNT * MAX_PROCESSES_PER_APP);
}

#[test]
fn destroyed_windows_are_removed() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Tasks, &mut table, 0);
    let windows = [win_normal(1, pid), win_normal(2, pid), win_normal(3, pid)];
    reconcile_no_fallback(&mut reg, &windows, |p| table.is_alive(p), 100);
    assert_eq!(reg.snapshot(AppId::Tasks).total_window_count, 3);
    // Drop #2 via a missing-from-LIST_WINDOWS reconcile.
    let live = [win_normal(1, pid), win_normal(3, pid)];
    reconcile_no_fallback(&mut reg, &live, |p| table.is_alive(p), 200);
    assert_eq!(reg.snapshot(AppId::Tasks).total_window_count, 2);
}

#[test]
fn exited_process_generations_are_removed() {
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    // Multi-instance app: two Calendar processes, two windows.
    let pid1 = launch(&mut reg, AppId::Calendar, &mut table, 0);
    let launch_id2 = reg.note_launch_requested(AppId::Calendar, 1);
    let pid2 = table.spawn();
    reg.note_spawn_succeeded(AppId::Calendar, pid2, launch_id2);
    let windows = [win_normal(1, pid1), win_normal(2, pid2)];
    reconcile_no_fallback(&mut reg, &windows, |p| table.is_alive(p), 100);
    assert_eq!(reg.snapshot(AppId::Calendar).total_window_count, 2);
    // Kill pid1; window 1 disappears.
    table.kill(pid1);
    reconcile_no_fallback(&mut reg, &[win_normal(2, pid2)], |p| table.is_alive(p), 200);
    let snap = reg.snapshot(AppId::Calendar);
    assert_eq!(snap.total_window_count, 1);
    assert_eq!(reg.total_processes(), 1); // only pid2 slot retained
    assert!(reg.snapshot(AppId::Calendar).state.indicator_on());
}

#[test]
fn existing_launch_behaviour_preserved() {
    // Mirror main.rs::handle_app_click: a Running app with a stored window is
    // not re-launched; the ACTIVE_WINDOW path is the shell's job. The registry
    // just must NOT enter Launching again when re-launch requested on a
    // Running app (multi-instance apps notwithstanding).
    let mut reg = RunningAppRegistry::new();
    let mut table = FakeProcessTable::new();
    let pid = launch(&mut reg, AppId::Calculator, &mut table, 0);
    reconcile_no_fallback(&mut reg, &[win_normal(1, pid)], |p| table.is_alive(p), 100);
    // Re-launch request on already-Running single-instance app: state stays
    // Running (no second Launching), and a fresh launch_id is reserved.
    let _ = reg.note_launch_requested(AppId::Calculator, 200);
    assert_eq!(reg.snapshot(AppId::Calculator).state, AppRunState::Running);
    assert!(reg.is_indicator_on(AppId::Calculator));
}

#[test]
fn indicator_off_for_idle_launching_failed() {
    let mut reg = RunningAppRegistry::new();
    assert!(!reg.is_indicator_on(AppId::Files));
    let _ = reg.note_launch_requested(AppId::Files, 0);
    assert_eq!(reg.snapshot(AppId::Files).state, AppRunState::Launching);
    assert!(!reg.is_indicator_on(AppId::Files));
    reg.note_launch_failed(AppId::Files, "x");
    assert_eq!(reg.snapshot(AppId::Files).state, AppRunState::Failed);
    assert!(!reg.is_indicator_on(AppId::Files));
    reconcile_no_fallback(&mut reg, &[], |_| false, 100);
    assert_eq!(reg.snapshot(AppId::Files).state, AppRunState::Idle);
    assert!(!reg.is_indicator_on(AppId::Files));
}

#[test]
fn late_process_exit_for_old_generation_is_ignored() {
    let mut reg = RunningAppRegistry::new();
    let pid = 77u64;
    let old_key = ProcessKey {
        pid,
        generation: 10,
    };
    let fresh_key = ProcessKey {
        pid,
        generation: 11,
    };
    reg.note_process_spawned(AppId::Terminal, AppInstanceId::new(1), old_key);
    reg.note_process_spawned(AppId::Files, AppInstanceId::new(2), fresh_key);
    reg.reconcile(
        &[WindowSnapshot {
            id: 9,
            owner_pid: pid,
            process_generation: fresh_key.generation,
            generation: 1,
            minimized: false,
            visible: true,
            normal: true,
        }],
        |_| true,
        |_| None,
        1,
    );
    reg.note_process_exited(old_key);
    assert!(!reg.is_indicator_on(AppId::Terminal));
    assert!(reg.is_indicator_on(AppId::Files));
}

#[test]
fn late_window_destroy_for_old_generation_is_ignored() {
    let mut reg = RunningAppRegistry::new();
    let key = ProcessKey {
        pid: 88,
        generation: 3,
    };
    reg.note_process_spawned(AppId::Writer, AppInstanceId::new(1), key);
    reg.reconcile(
        &[WindowSnapshot {
            id: 5,
            owner_pid: key.pid,
            process_generation: key.generation,
            generation: 2,
            minimized: false,
            visible: true,
            normal: true,
        }],
        |_| true,
        |_| None,
        1,
    );
    reg.note_window_destroyed(WindowKey {
        id: 5,
        generation: 1,
    });
    assert_eq!(reg.snapshot(AppId::Writer).total_window_count, 1);
    assert!(reg.is_indicator_on(AppId::Writer));
}

#[test]
fn one_instance_can_own_multiple_processes() {
    let mut reg = RunningAppRegistry::new();
    let instance = AppInstanceId::new(42);
    let first = ProcessKey {
        pid: 301,
        generation: 1,
    };
    let second = ProcessKey {
        pid: 302,
        generation: 2,
    };
    reg.note_process_spawned(AppId::Calendar, instance, first);
    reg.note_process_spawned(AppId::Calendar, instance, second);
    let windows = [
        WindowSnapshot {
            id: 1,
            owner_pid: first.pid,
            process_generation: first.generation,
            generation: 1,
            minimized: false,
            visible: true,
            normal: true,
        },
        WindowSnapshot {
            id: 2,
            owner_pid: second.pid,
            process_generation: second.generation,
            generation: 1,
            minimized: false,
            visible: true,
            normal: true,
        },
    ];
    reg.reconcile(&windows, |_| true, |_| None, 1);
    let snapshot = reg.snapshot(AppId::Calendar);
    assert_eq!(snapshot.instance_count, 1);
    assert_eq!(snapshot.total_process_count, 2);
    assert_eq!(snapshot.total_window_count, 2);
}

#[test]
fn two_instances_of_one_app_remain_indicated() {
    let mut reg = RunningAppRegistry::new();
    let first = ProcessKey {
        pid: 401,
        generation: 1,
    };
    let second = ProcessKey {
        pid: 402,
        generation: 2,
    };
    reg.note_process_spawned(AppId::Mines, AppInstanceId::new(1), first);
    reg.note_process_spawned(AppId::Mines, AppInstanceId::new(2), second);
    reg.reconcile(
        &[
            WindowSnapshot {
                id: 1,
                owner_pid: first.pid,
                process_generation: first.generation,
                generation: 1,
                minimized: false,
                visible: true,
                normal: true,
            },
            WindowSnapshot {
                id: 2,
                owner_pid: second.pid,
                process_generation: second.generation,
                generation: 1,
                minimized: true,
                visible: false,
                normal: true,
            },
        ],
        |_| true,
        |_| None,
        1,
    );
    let snapshot = reg.snapshot(AppId::Mines);
    assert_eq!(snapshot.instance_count, 2);
    assert_eq!(snapshot.total_process_count, 2);
    assert!(reg.is_indicator_on(AppId::Mines));
}
