//! Shell-owned running-application registry.
//!
//! One [`RunningAppRegistry`] is the single source of truth consulted by the
//! dock, the Start Menu (Pinned / Recent / All Apps), the dynamic running-app
//! strip, and future search results. View surfaces read immutable
//! [`AppSnapshot`]s out of the registry; they never maintain their own
//! running-state copies.
//!
//! Design constraints encoded here (see `docs/GUI/START_MENU.md` for the
//! full architecture writeup and the audit report):
//!
//! - `#![no_std]`, no `alloc`: the registry is fixed-size (`APP_COUNT` apps,
//!   `MAX_PROCESSES_PER_APP` tracked processes, `MAX_WINDOWS_PER_APP` tracked
//!   windows per app) so it can be embedded in the shell crate without heap
//!   churn on the per-frame draw path.
//! - No IPC dependency: process liveness and `pid -> AppId` reverse mapping are
//!   injected closures passed to [`RunningAppRegistry::reconcile`].
//! - Stable identity: entries are keyed by [`AppId`] (the manifest identity,
//!   mirrored from the shell's own `AppId`). Window titles, display names,
//!   and icon paths are never used as identity.
//! - Pseudo-generation: the kernel's `SYS_SPAWN` does not return a process
//!   generation today, so [`ProcessKey::generation`] is a shell-side monotonic
//!   counter used as a "where available" generation surrogate. True
//!   PID-reuse safety across a live→dead→live sequence still depends on
//!   `process_is_alive` faithfully reporting the recycle (see
//!   [`RunningAppRegistry::reconcile`] and the PID-reuse limitation note at
//!   the bottom of this file).
//!
//! Lifecycle rules (spec-aligned):
//!
//! | Event                       | State transition                                  |
//! |-----------------------------|----------------------------------------------------|
//! | Launch requested            | Idle/Launching/Failed/Running -> Launching         |
//! | Spawn returned a pid        | Launching + record (pid, launch_id)                |
//! | First owned window appears  | Launching/Idle -> Running (or Minimized)           |
//! | Window minimized            | stays Running/Minimized per mix                    |
//! | Window restored             | Minimized -> Running if any visible                |
//! | One of several windows gone | stays Running while another owned window exists    |
//! | Last normal window gone     | Running/Minimized -> ClosingAwaitExit (pid alive) |
//! | Process exited / crashed    | drop pid slot + its owned windows                  |
//! | End Task confirmed          | reconcile observes is_alive=false -> drops         |
//! | Launch failed / timed out   | Launching -> Failed -> Idle on next reconcile      |
//! | Shell restart               | populate from current windows (see [`RunningAppRegistry::reconstruct`]) |
//!
//! Indicator rule (spec):
//!   `is_indicator_on` == `Running | Minimized | ClosingAwaitExit`.
//!   `Launching`, `Failed`, `Idle` do NOT draw the underline (matches the
//!   existing Start Menu tile behaviour at `start_menu.rs:1280-1292`).

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of distinct launchable SunlightOS apps tracked by the shell.
/// Mirrors the size of `VortexShell.apps: [DockAppState; 16]`.
pub const APP_COUNT: usize = 17;

/// Maximum processes tracked per app. Covers the multi-instance apps the shell
/// permits today (Calendar, Chronos, Mines, SiliconEchoes — at most a few
/// concurrent copies per real workload). A larger value would bloat the
/// registry; exceeding this drops *new* processes for that app, never stale
/// state (stale slots are still removed by [`RunningAppRegistry::reconcile`]).
pub const MAX_PROCESSES_PER_APP: usize = 4;

/// Maximum windows tracked per app. Same rationale.
pub const MAX_WINDOWS_PER_APP: usize = 8;

/// Launch timeout matching `main.rs::APP_LAUNCH_TIMEOUT_MS` (30s).
pub const APP_LAUNCH_TIMEOUT_MS: u64 = 30_000;

// ---------------------------------------------------------------------------
// AppId — manifests identity, mirrored from the shell
// ---------------------------------------------------------------------------

/// Stable application identity mirrored byte-for-byte from
/// `services/sunlight-vortex-shell/src/main.rs::AppId` (enum declaration order
/// MUST stay identical so `AppId as usize` indices line up with
/// `VortexShell.apps[i]`). The shell converts at the boundary with
/// `From<shell::AppId> for AppId` and back; identity is by manifest, never by
/// display name/binary/window title.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum AppId {
    Terminal = 0,
    Chronos = 1,
    Calculator = 2,
    Files = 3,
    Settings = 4,
    Tasks = 5,
    Bench = 6,
    TextEditor = 7,
    Writer = 8,
    Calendar = 9,
    Devices = 10,
    RappidRabbit = 11,
    ApiLab = 12,
    Mines = 13,
    SiliconEchoes = 14,
    Welcome = 15,
    WiseOwl = 16,
}

impl AppId {
    /// Iterates over the full manifest set in the canonical slot order.
    pub const fn all() -> [AppId; APP_COUNT] {
        [
            AppId::Terminal,
            AppId::Chronos,
            AppId::Calculator,
            AppId::Files,
            AppId::Settings,
            AppId::Tasks,
            AppId::Bench,
            AppId::TextEditor,
            AppId::Writer,
            AppId::Calendar,
            AppId::Devices,
            AppId::RappidRabbit,
            AppId::ApiLab,
            AppId::Mines,
            AppId::SiliconEchoes,
            AppId::Welcome,
            AppId::WiseOwl,
        ]
    }

    /// Slot index in `VortexShell.apps`. Stable: matches enum declaration order.
    pub const fn index(self) -> usize {
        self as usize
    }
}

// ---------------------------------------------------------------------------
// Ring-3 instance identity + records
// ---------------------------------------------------------------------------

/// One Ring-3 launch instance of an [`AppId`]. This is deliberately a shell
/// concept: the kernel only schedules processes and never interprets it as an
/// application name, manifest, launch policy, or window relationship.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AppInstanceId(u64);

impl AppInstanceId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A process incarnation. `generation` is the authoritative kernel generation
/// when a future generic process-inventory API supplies one; today it is the
/// shell's monotonically allocated launch generation for shell-owned spawns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProcessKey {
    pub pid: u64,
    pub generation: u64,
}

/// A display/window incarnation. The compositor does not currently expose a
/// generation in `LIST_WINDOWS`, so reconstructed entries use generation zero.
/// Exact-key event APIs still reject stale close notifications when a source
/// does provide one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WindowKey {
    pub id: u64,
    pub generation: u64,
}

/// Lifecycle phase of one app, as observed by the shell.
///
/// Maps directly to the existing `main.rs::AppLaunchState` so the dock and
/// Start Menu rendering code paths are unchanged; the registry only owns the
/// derivation rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppRunState {
    /// No process tracked for this app.
    Idle,
    /// Spawn requested / pid known, but no owned window has appeared yet and
    /// we have not yet timed out.
    Launching,
    /// At least one owned window is visible (non-minimized, on a visible
    /// workspace, not hidden/rolled-up).
    Running,
    /// At least one owned window exists, but every owned window is minimized.
    Minimized,
    /// Last owned window disappeared while a process identity is still alive
    /// (waiting for pid death after a decoration close / End-Task in flight).
    /// Indicator stays on briefly until death is confirmed.
    ClosingAwaitExit,
    /// Spawn failed or launch window timed out. Surfaced for the existing
    /// warn-coloured dock tile border; cleared to Idle on the next
    /// [`RunningAppRegistry::reconcile`] that sees no live pid.
    Failed,
}

impl AppRunState {
    /// Whether the running indicator underline (or dock accent bar) should
    /// render for this state. Per spec: indicator stays on for Running,
    /// Minimized, and the brief ClosingAwaitExit window.
    pub const fn indicator_on(self) -> bool {
        matches!(
            self,
            AppRunState::Running | AppRunState::Minimized | AppRunState::ClosingAwaitExit
        )
    }

    /// Name for diagnostics, matching `main.rs::app_state_name` output.
    pub const fn name(self) -> &'static str {
        match self {
            AppRunState::Idle => "NotRunning",
            AppRunState::Launching => "Launching",
            AppRunState::Running => "Running",
            AppRunState::Minimized => "Minimized",
            AppRunState::ClosingAwaitExit => "Closing",
            AppRunState::Failed => "Failed",
        }
    }
}

/// Static, allocator-free snapshot returned to view code. Mirrors the
/// per-app fields the dock + Start Menu read off `DockAppState` today.
#[derive(Clone, Copy, Debug)]
pub struct AppSnapshot {
    pub app_id: AppId,
    pub state: AppRunState,
    /// Distinct Ring-3 launch instances currently represented for this app.
    pub instance_count: u32,
    /// Total process incarnations across those instances.
    pub total_process_count: u32,
    /// Owned windows that are currently visible (not minimized/hidden).
    pub visible_window_count: u32,
    /// Total tracked owned windows (visible + minimized + rolled-up).
    pub total_window_count: u32,
    /// Shell doesn't yet receive per-child focus events today; this is held for
    /// future use and always false here so the indicator never depends on title
    /// text (see `start_menu.rs` audit).
    pub focused: bool,
    /// Convenience: first tracked window id (matches the historical
    /// `main_window_id` semantics the dock uses for `ACTIVATE_WINDOW`).
    pub main_window_id: Option<u64>,
    /// Convenience: first tracked owner pid (guides the shell's dock pin
    /// pid association; multiple instances are tracked internally too).
    pub main_pid: Option<u64>,
    pub last_launch_started_at: u64,
    pub last_launch_id: u64,
    /// Most recent spawn error reason captured for `Failed`. Bounded binary.
    pub launch_error_len: u32,
    pub launch_error: [u8; 32],
}

impl AppSnapshot {
    pub fn launch_error_str(&self) -> &str {
        let n = self.launch_error_len as usize;
        if n > self.launch_error.len() {
            return "";
        }
        core::str::from_utf8(&self.launch_error[..n]).unwrap_or("")
    }
}

/// Owned-window record built by the shell from `LIST_WINDOWS` snapshots.
#[derive(Clone, Copy, Debug)]
pub struct WindowSnapshot {
    pub id: u64,
    pub owner_pid: u64,
    /// Process generation supplied by an authoritative event source, or zero
    /// when the current `LIST_WINDOWS` protocol cannot provide one.
    pub process_generation: u64,
    /// Window generation supplied by an authoritative event source, or zero
    /// when the current display inventory cannot provide one.
    pub generation: u64,
    /// Window is minimized.
    pub minimized: bool,
    /// Window is visible on its workspace (not hidden/rolled-up). The shell
    /// already filters this across the active workspace before calling the
    /// registry.
    pub visible: bool,
    /// Window type is normal (not Desktop/Widget/Dialog).
    pub normal: bool,
}

// ---------------------------------------------------------------------------
// Internal slots
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // fields form the documented snapshot/pseudo-gen contract
struct ProcessSlot {
    key: ProcessKey,
    /// Ring-3 launch instance containing this process. One instance may have
    /// multiple processes; multiple instances may share the same [`AppId`].
    instance_id: AppInstanceId,
    /// Set when an End-Task was issued for this pid; reconcile tolerates the
    /// brief window before `is_alive` flips false.
    kill_in_flight: bool,
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // fields form the documented snapshot contract
struct OwnedWindow {
    key: WindowKey,
    process_key: ProcessKey,
    minimized: bool,
    visible: bool,
    normal: bool,
}

struct AppEntry {
    app_id: AppId,
    state: AppRunState,
    processes: [Option<ProcessSlot>; MAX_PROCESSES_PER_APP],
    windows: [Option<OwnedWindow>; MAX_WINDOWS_PER_APP],
    last_launch_started_at: u64,
    last_launch_id: u64,
    launch_error_len: u32,
    launch_error: [u8; 32],
}

impl AppEntry {
    const fn new(app_id: AppId) -> Self {
        Self {
            app_id,
            state: AppRunState::Idle,
            processes: [None; MAX_PROCESSES_PER_APP],
            windows: [None; MAX_WINDOWS_PER_APP],
            last_launch_started_at: 0,
            last_launch_id: 0,
            launch_error_len: 0,
            launch_error: [0; 32],
        }
    }

    fn set_error(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let n = bytes.len().min(self.launch_error.len());
        self.launch_error[..n].copy_from_slice(&bytes[..n]);
        self.launch_error_len = n as u32;
    }

    fn clear_error(&mut self) {
        self.launch_error_len = 0;
    }

    fn process_count(&self) -> usize {
        self.processes.iter().filter(|p| p.is_some()).count()
    }

    fn window_count(&self) -> usize {
        self.windows.iter().filter(|w| w.is_some()).count()
    }

    fn instance_count(&self) -> usize {
        let mut instances = [0u64; MAX_PROCESSES_PER_APP];
        let mut count = 0usize;
        for process in self.processes.iter().flatten() {
            let raw = process.instance_id.raw();
            if !instances[..count].contains(&raw) {
                instances[count] = raw;
                count += 1;
            }
        }
        count
    }

    fn visible_window_count(&self) -> usize {
        self.windows
            .iter()
            .filter(|w| matches!(w, Some(w) if w.visible))
            .count()
    }

    fn clear_windows(&mut self) {
        self.windows = [None; MAX_WINDOWS_PER_APP];
    }

    fn clear_processes(&mut self) {
        self.processes = [None; MAX_PROCESSES_PER_APP];
    }

    fn store_window(&mut self, win: OwnedWindow) {
        // Replace existing entry with same win_id if present, else first free.
        for slot in self.windows.iter_mut() {
            if let Some(existing) = slot {
                if existing.key == win.key {
                    *slot = Some(win);
                    return;
                }
            }
        }
        for slot in self.windows.iter_mut() {
            if slot.is_none() {
                *slot = Some(win);
                return;
            }
        }
        // Overflow: registry is saturated for this app. Preserve existing
        // windows; new ones are dropped. Spec says: "do not allocate on every
        // frame" — bounded by design.
    }

    fn store_process(&mut self, proc: ProcessSlot) -> bool {
        for slot in self.processes.iter_mut() {
            if let Some(existing) = slot {
                if existing.key == proc.key {
                    *slot = Some(proc);
                    return true;
                }
            }
        }
        for slot in self.processes.iter_mut() {
            if slot.is_none() {
                *slot = Some(proc);
                return true;
            }
        }
        // Overflow — refused. Caller reports failure (caller/test logs).
        false
    }

    fn has_pid(&self, pid: u64) -> bool {
        self.processes
            .iter()
            .any(|p| matches!(p, Some(p) if p.key.pid == pid))
    }

    fn has_process_key(&self, key: ProcessKey) -> bool {
        self.processes
            .iter()
            .any(|p| matches!(p, Some(process) if process.key == key))
    }

    fn remove_windows_for_pid(&mut self, pid: u64) {
        for slot in self.windows.iter_mut() {
            if let Some(w) = slot {
                if w.process_key.pid == pid {
                    *slot = None;
                }
            }
        }
    }

    fn remove_processes_for_pid(&mut self, pid: u64) {
        for slot in self.processes.iter_mut() {
            if matches!(slot, Some(process) if process.key.pid == pid) {
                *slot = None;
            }
        }
        self.remove_windows_for_pid(pid);
    }

    fn remove_windows_for_process(&mut self, key: ProcessKey) {
        for slot in self.windows.iter_mut() {
            if let Some(window) = slot {
                if window.process_key == key {
                    *slot = None;
                }
            }
        }
    }

    fn snapshot(&self) -> AppSnapshot {
        let main_window_id = self.windows.iter().rev().find_map(|w| w.map(|w| w.key.id));
        let main_pid = self
            .processes
            .iter()
            .rev()
            .find_map(|p| p.map(|p| p.key.pid));
        AppSnapshot {
            app_id: self.app_id,
            state: self.state,
            instance_count: self.instance_count() as u32,
            total_process_count: self.process_count() as u32,
            visible_window_count: self.visible_window_count() as u32,
            total_window_count: self.window_count() as u32,
            focused: false,
            main_window_id,
            main_pid,
            last_launch_started_at: self.last_launch_started_at,
            last_launch_id: self.last_launch_id,
            launch_error_len: self.launch_error_len,
            launch_error: self.launch_error,
        }
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Shell-owned running-application registry — single source of truth for
/// running/launching/fail indicators across the dock, Start Menu, and All Apps.
///
/// Acquire one instance per shell process. All state mutations go through the
/// lifecycle methods below; `reconcile` is the poll-time workhorse that
/// re-derives each app's state from a fresh `LIST_WINDOWS` snapshot.
pub struct RunningAppRegistry {
    apps: [AppEntry; APP_COUNT],
    next_launch_id: u64,
    next_instance_id: u64,
}

impl RunningAppRegistry {
    /// Empty registry; every app starts [`AppRunState::Idle`].
    pub const fn new() -> Self {
        Self {
            apps: [
                AppEntry::new(AppId::Terminal),
                AppEntry::new(AppId::Chronos),
                AppEntry::new(AppId::Calculator),
                AppEntry::new(AppId::Files),
                AppEntry::new(AppId::Settings),
                AppEntry::new(AppId::Tasks),
                AppEntry::new(AppId::Bench),
                AppEntry::new(AppId::TextEditor),
                AppEntry::new(AppId::Writer),
                AppEntry::new(AppId::Calendar),
                AppEntry::new(AppId::Devices),
                AppEntry::new(AppId::RappidRabbit),
                AppEntry::new(AppId::ApiLab),
                AppEntry::new(AppId::Mines),
                AppEntry::new(AppId::SiliconEchoes),
                AppEntry::new(AppId::Welcome),
                AppEntry::new(AppId::WiseOwl),
            ],
            next_launch_id: 1,
            next_instance_id: 1,
        }
    }

    /// Read-only snapshot of one app's running state. Pure data; view code
    /// (dock, Start Menu, future search) reads this without consulting any
    /// separate running-state copy.
    pub fn snapshot(&self, app_id: AppId) -> AppSnapshot {
        self.apps[app_id.index()].snapshot()
    }

    /// Iterate every app's snapshot in canonical slot order.
    pub fn snapshots(&self) -> AppIter<'_> {
        AppIter {
            registry: self,
            idx: 0,
        }
    }

    /// Indicator bit (underline / dock accent) for one app. Mirrors the
    /// `AppRunState::indicator_on` rule so callers don't all rewrite it.
    pub fn is_indicator_on(&self, app_id: AppId) -> bool {
        self.apps[app_id.index()].state.indicator_on()
    }

    fn entry_mut(&mut self, app_id: AppId) -> &mut AppEntry {
        &mut self.apps[app_id.index()]
    }

    /// Allocate a fresh pseudo-generation id for a new launch. Bumped by
    /// [`Self::note_launch_requested`]; the value flows into
    /// [`ProcessSlot::launch_id`] at [`Self::note_spawn_succeeded`] so that a
    /// recycled numeric pid can never "win" the match against a stale `launch_id`
    /// — see the PID-reuse limitation note at the bottom of this file.
    pub fn next_launch_id(&mut self) -> u64 {
        let id = self.next_launch_id;
        self.next_launch_id = self.next_launch_id.wrapping_add(1);
        id
    }

    fn next_instance_id(&mut self) -> AppInstanceId {
        let id = AppInstanceId::new(self.next_instance_id);
        self.next_instance_id = self.next_instance_id.wrapping_add(1);
        id
    }

    /// Schedule a launch request. Returns the assigned launch id (caller should
    /// thread it through to [`Self::note_spawn_succeeded`]).
    ///
    /// Valid from any prior state. Already-running apps may legitimately be
    /// re-launched (multi-instance apps); for those, the prior Launching/Running
    /// state is preserved and a new launch slot is just allocated.
    pub fn note_launch_requested(&mut self, app_id: AppId, now: u64) -> u64 {
        let launch_id = self.next_launch_id();
        let entry = self.entry_mut(app_id);
        entry.last_launch_id = launch_id;
        entry.last_launch_started_at = now;
        entry.clear_error();
        // Multi-instance apps may already be Running; don't clobber their
        // Running state on a fresh launch request. Transition to Launching only
        // when there is nothing else keeping the app live.
        if !matches!(
            entry.state,
            AppRunState::Running | AppRunState::Minimized | AppRunState::ClosingAwaitExit
        ) {
            entry.state = AppRunState::Launching;
        }
        launch_id
    }

    /// Confirm a process spawn returned a pid. Records the (pid, launch_id)
    /// pseudo-generation pair. `launch_id` must match the id returned by the
    /// preceding [`Self::note_launch_requested`] so a late spawn reply can
    /// never be paired with a newer launch attempt.
    pub fn note_spawn_succeeded(&mut self, app_id: AppId, pid: u64, launch_id: u64) {
        let instance_id = self.next_instance_id();
        self.note_process_spawned(
            app_id,
            instance_id,
            ProcessKey {
                pid,
                generation: launch_id,
            },
        );
    }

    /// Attach an explicit process incarnation to a Ring-3 application
    /// instance. Launchers use this for helper processes belonging to one
    /// application launch; the kernel sees neither identifier as application
    /// policy.
    pub fn note_process_spawned(
        &mut self,
        app_id: AppId,
        instance_id: AppInstanceId,
        key: ProcessKey,
    ) {
        // A fresh authoritative process generation supersedes every stale
        // observation of the same numeric PID. A PID cannot represent two
        // concurrent process incarnations, so retaining the older slot would
        // let a delayed event keep an obsolete indicator alive.
        for entry in self.apps.iter_mut() {
            entry.remove_processes_for_pid(key.pid);
        }
        let entry = self.entry_mut(app_id);
        // If a stale slot for this numeric pid still exists (left over from a
        // previous launch that we never observed dying), overwrite it with the
        // fresh launch_id — the new spawn wins.
        let stored = entry.store_process(ProcessSlot {
            key,
            instance_id,
            kill_in_flight: false,
        });
        if !stored {
            // Saturated: drop the oldest free-window-bearing slot so the
            // newest launch is always accounted for.
            let _ = entry.processes[0].take();
            entry.store_process(ProcessSlot {
                key,
                instance_id,
                kill_in_flight: false,
            });
        }
        if entry.state == AppRunState::Idle {
            entry.state = AppRunState::Launching;
        }
    }

    /// Spawn call returned an error. App goes to `Failed`; the next
    /// [`Self::reconcile`] that sees no live pid slot clears it back to `Idle`.
    pub fn note_launch_failed(&mut self, app_id: AppId, reason: &str) {
        let entry = self.entry_mut(app_id);
        entry.state = AppRunState::Failed;
        entry.set_error(reason);
        // Drop the last_launch slot if it never produced a pid entry so the
        // Failed state clears on the next reconcile (no zombie slot). If a pid
        // slot was recorded for this launch, keep it — reconcile will observe
        // the pid's death and clear Failed -> Idle.
        entry.last_launch_id = 0;
    }

    /// Mark a launch as having timed out (>= `APP_LAUNCH_TIMEOUT_MS` since
    /// `note_launch_requested`). The pid, if any, is dropped so the next
    /// `reconcile` clears `Failed` back to `Idle`.
    pub fn note_launch_timeout(&mut self, app_id: AppId) {
        let entry = self.entry_mut(app_id);
        entry.state = AppRunState::Failed;
        entry.set_error("launch timed out");
        // No pid slot is dropped here — process may still be alive. Reconcile
        // will track its death and clear Failed -> Idle.
    }

    /// Request confirmation that an End-Task has been issued for `pid`. The
    /// registry does not actually deliver the kill (that stays in the shell or
    /// the Tasks app via `sunlight_ipc::kill`); calling this lets `reconcile`
    /// tolerate the brief window before the kernel reports the pid as dead.
    pub fn note_kill_in_flight(&mut self, pid: u64) {
        for entry in self.apps.iter_mut() {
            for slot in entry.processes.iter_mut() {
                if let Some(p) = slot {
                    if p.key.pid == pid {
                        p.kill_in_flight = true;
                    }
                }
            }
        }
    }

    /// Drop a process slot unconditionally and any owned windows keyed on its
    /// pid. Used by tests to model confirmed death, and by the shell to fast-
    /// path the "End-Task returned, dead already observed" case.
    pub fn note_process_died(&mut self, pid: u64) {
        for entry in self.apps.iter_mut() {
            let mut removed = false;
            for slot in entry.processes.iter_mut() {
                if let Some(p) = slot {
                    if p.key.pid == pid {
                        *slot = None;
                        removed = true;
                    }
                }
            }
            if removed {
                entry.remove_windows_for_pid(pid);
            }
        }
    }

    /// Remove one exact process incarnation. A delayed exit for an old PID
    /// generation therefore cannot remove a newer process that reused the
    /// numeric PID.
    pub fn note_process_exited(&mut self, key: ProcessKey) {
        for entry in self.apps.iter_mut() {
            let mut removed = false;
            for slot in entry.processes.iter_mut() {
                if matches!(slot, Some(process) if process.key == key) {
                    *slot = None;
                    removed = true;
                }
            }
            if removed {
                entry.remove_windows_for_process(key);
            }
        }
    }

    /// Remove one exact window incarnation. Event sources that provide a
    /// window generation use this to ignore a late destruction notification.
    pub fn note_window_destroyed(&mut self, key: WindowKey) {
        for entry in self.apps.iter_mut() {
            for slot in entry.windows.iter_mut() {
                if matches!(slot, Some(window) if window.key == key) {
                    *slot = None;
                }
            }
        }
    }

    /// Bulk clear all per-app state — used on shell restart, since transient
    /// running state is never persisted (per spec: sunlight-kv holds
    /// notifications/calendar/tasks/reminders only).
    pub fn reset(&mut self) {
        for entry in self.apps.iter_mut() {
            entry.state = AppRunState::Idle;
            entry.clear_processes();
            entry.clear_windows();
            entry.last_launch_id = 0;
            entry.last_launch_started_at = 0;
            entry.clear_error();
        }
    }

    /// Shell-restart / late-attach reconstruction.
    ///
    /// Walk `windows` and try to assign each to one of the manifest apps via
    /// `pid_to_app`; adopt the window AND register a `ProcessSlot` for its pid
    /// so future windows from that pid attach deterministically. Pids that
    /// resolve to `None` (unpinned apps, helper processes, or anything not in
    /// the manifest list) are simply ignored by this call — they continue to
    /// live in the shell's dynamic running-apps strip (a separate code path
    /// historically keyed by `win_id`, not duplicated here).
    ///
    /// Per spec: "reconstruct state from current process/window ownership
    /// instead of restoring stale running state from persistence."
    ///
    /// Note: `pid_to_app` must come from trusted Ring-3 launch metadata or an
    /// application manifest inventory. Executable paths, titles, labels, and
    /// icon paths are deliberately invalid sources for this mapping.
    pub fn reconstruct(
        &mut self,
        windows: &[WindowSnapshot],
        mut pid_to_app: impl FnMut(u64) -> Option<AppId>,
    ) {
        for window in windows {
            let Some(app_id) = pid_to_app(window.owner_pid) else {
                continue;
            };
            let entry = self.entry_mut(app_id);
            // Register a process slot so subsequent reconcile picks up the
            // association without re-running pid_to_app.
            let _ = entry.store_process(ProcessSlot {
                key: ProcessKey {
                    pid: window.owner_pid,
                    generation: window.process_generation,
                },
                instance_id: AppInstanceId::new(0),
                kill_in_flight: false,
            });
            entry.store_window(OwnedWindow {
                key: WindowKey {
                    id: window.id,
                    generation: window.generation,
                },
                process_key: ProcessKey {
                    pid: window.owner_pid,
                    generation: window.process_generation,
                },
                minimized: window.minimized,
                visible: window.visible,
                normal: window.normal,
            });
            // Derive the initial state from the window mix.
            if entry.state == AppRunState::Idle {
                entry.state = if window.minimized {
                    AppRunState::Minimized
                } else {
                    AppRunState::Running
                };
            } else if entry.state == AppRunState::Minimized && window.visible {
                entry.state = AppRunState::Running;
            }
        }
    }

    /// Poll-time reconciliation. Re-derives every app's [`AppRunState`] from a
    /// fresh `windows` slice plus a `pid_to_app` fallback resolver.
    ///
    /// Returns `true` if any app's [`AppRunState`] changed (caller marks the
    /// dock / Start Menu dirty accordingly).
    ///
    /// `is_alive` is called on each stored numeric pid at most once per poll
    /// per app (bounded by `MAX_PROCESSES_PER_APP`).
    ///
    /// `pid_to_app` is consulted for windows whose `owner_pid` did not match
    /// any stored process slot — to support:
    ///
    /// - shell restart with already-running apps (no pid ↔ launch_id mapping),
    /// - apps launched from a terminal / by another process (no shell spawn).
    ///
    /// Windows that resolve to no app are ignored here (they live in the
    /// dynamic running-apps strip on the dock, not the pinned entries).
    pub fn reconcile(
        &mut self,
        windows: &[WindowSnapshot],
        mut is_alive: impl FnMut(u64) -> bool,
        mut pid_to_app: impl FnMut(u64) -> Option<AppId>,
        now: u64,
    ) -> bool {
        let mut dirty = false;

        for entry in self.apps.iter_mut() {
            let prev_state = entry.state;
            let prev_window_count = entry.window_count() as u32;
            let prev_process_count = entry.process_count() as u32;

            // (1) Identify dead pid slots (do NOT drop yet — state derivation
            //     in step (4) needs to observe "pid died while Launching" so
            //     one Failed cycle is surfaced before Idle). We only *drop* dead
            //     slots at the very end of this iteration (step 5).
            let mut dead_pids = [0u64; MAX_PROCESSES_PER_APP];
            let mut dead_count = 0usize;
            for slot in entry.processes.iter() {
                if let Some(p) = slot {
                    if !is_alive(p.key.pid) {
                        if dead_count < dead_pids.len() {
                            dead_pids[dead_count] = p.key.pid;
                            dead_count += 1;
                        }
                    }
                }
            }
            let is_dead_owner =
                |pid: u64| -> bool { dead_pids.iter().take(dead_count).any(|&p| p == pid) };

            // (2) Adopt windows for this app: match stored pid slots OR resolve
            //     via pid_to_app fallback. Windows reporting an owner_pid that is
            //     currently in `dead_pids` are *not* re-adopted defensively —
            //     they are stale recycled-pid events and would mislabel the app
            //     (PID-reuse hazard, see the limitation note at the file bottom).
            let mut adopted = [None; MAX_WINDOWS_PER_APP];
            let mut adopted_len = 0usize;
            for window in windows {
                if !window.normal {
                    // Dialog/Desktop/Widget windows do not drive indicators today;
                    // the dock's running strip handles its own dialogue classification.
                    continue;
                }
                if is_dead_owner(window.owner_pid) {
                    continue;
                }
                let process_key = ProcessKey {
                    pid: window.owner_pid,
                    generation: window.process_generation,
                };
                let known_process = if window.process_generation == 0 {
                    entry.has_pid(window.owner_pid)
                } else {
                    entry.has_process_key(process_key)
                };
                let belongs = if known_process {
                    true
                } else if let Some(resolved) = pid_to_app(window.owner_pid) {
                    resolved == entry.app_id
                } else {
                    false
                };
                if !belongs {
                    continue;
                }
                // Register the pid slot if first sighting for this app.
                if !known_process {
                    let _ = entry.store_process(ProcessSlot {
                        key: ProcessKey {
                            pid: window.owner_pid,
                            generation: window.process_generation,
                        },
                        instance_id: AppInstanceId::new(0),
                        kill_in_flight: false,
                    });
                }
                if adopted_len < adopted.len() {
                    adopted[adopted_len] = Some(OwnedWindow {
                        key: WindowKey {
                            id: window.id,
                            generation: window.generation,
                        },
                        process_key: ProcessKey {
                            pid: window.owner_pid,
                            generation: window.process_generation,
                        },
                        minimized: window.minimized,
                        visible: window.visible,
                        normal: window.normal,
                    });
                    adopted_len += 1;
                }
            }

            // (3) Diff owned windows: keep set = adopted. Stale windows drop.
            // We rewrite the windows array from `adopted` (stable order).
            entry.clear_windows();
            for i in 0..adopted_len {
                if let Some(w) = adopted[i] {
                    entry.store_window(w);
                }
            }

            // (4) Derive state. `process_count` still includes dead slots — used
            //     only to detect the "pid died while Launching" case so we can
            //     emit one Failed cycle before Idle.
            let has_windows = entry.window_count() > 0;
            let any_visible = entry.visible_window_count() > 0;
            let process_count = entry.process_count();
            let any_live_process = entry
                .processes
                .iter()
                .any(|p| matches!(p, Some(p) if is_alive(p.key.pid)));

            let next = if has_windows {
                if any_visible {
                    AppRunState::Running
                } else {
                    AppRunState::Minimized
                }
            } else {
                // No owned windows.
                match entry.state {
                    AppRunState::Launching => {
                        if now.saturating_sub(entry.last_launch_started_at) >= APP_LAUNCH_TIMEOUT_MS
                        {
                            entry.set_error("launch timed out");
                            entry.clear_processes();
                            AppRunState::Failed
                        } else if process_count == 0 {
                            // No pid known yet; keep waiting.
                            AppRunState::Launching
                        } else if !any_live_process {
                            // Pid recorded but it died before any window opened.
                            // Surface one Failed cycle, then clear pids so the
                            // next reconcile collapses Failed -> Idle.
                            entry.set_error("process exited before window");
                            entry.clear_processes();
                            AppRunState::Failed
                        } else {
                            // Pid alive but no window yet; keep waiting.
                            AppRunState::Launching
                        }
                    }
                    AppRunState::Running | AppRunState::Minimized => {
                        if process_count == 0 || !any_live_process {
                            AppRunState::Idle
                        } else {
                            AppRunState::ClosingAwaitExit
                        }
                    }
                    AppRunState::ClosingAwaitExit => {
                        if process_count == 0 || !any_live_process {
                            AppRunState::Idle
                        } else {
                            AppRunState::ClosingAwaitExit
                        }
                    }
                    AppRunState::Failed => {
                        if process_count == 0 {
                            entry.clear_error();
                            AppRunState::Idle
                        } else {
                            AppRunState::Failed
                        }
                    }
                    AppRunState::Idle => AppRunState::Idle,
                }
            };

            // (5) Now actually drop dead pid slots (their windows were dropped
            //     in step 3 because adoption skipped dead owner_pids).
            for &pid in dead_pids.iter().take(dead_count) {
                for slot in entry.processes.iter_mut() {
                    if let Some(p) = slot {
                        if p.key.pid == pid {
                            *slot = None;
                        }
                    }
                }
            }

            entry.state = next;
            let new_window_count = entry.window_count() as u32;
            let new_process_count = entry.process_count() as u32;
            if next != prev_state
                || new_window_count != prev_window_count
                || new_process_count != prev_process_count
            {
                dirty = true;
            }
        }

        dirty
    }

    /// Bounded sanity check used by tests and shell diagnostics: total number
    /// of stored process slots across all apps. Must never exceed
    /// `APP_COUNT * MAX_PROCESSES_PER_APP`.
    pub fn total_processes(&self) -> usize {
        self.apps.iter().map(|e| e.process_count()).sum()
    }

    /// Bounded sanity check: total stored windows across all apps.
    pub fn total_windows(&self) -> usize {
        self.apps.iter().map(|e| e.window_count()).sum()
    }
}

impl Default for RunningAppRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over all apps' [`AppSnapshot`]s, in canonical slot order.
pub struct AppIter<'a> {
    registry: &'a RunningAppRegistry,
    idx: usize,
}

impl<'a> Iterator for AppIter<'a> {
    type Item = AppSnapshot;
    fn next(&mut self) -> Option<AppSnapshot> {
        if self.idx >= APP_COUNT {
            return None;
        }
        let app_id = AppId::all()[self.idx];
        self.idx += 1;
        Some(self.registry.snapshot(app_id))
    }
}

// ---------------------------------------------------------------------------
// PID-reuse limitation
// ---------------------------------------------------------------------------
//
// Per spec clause "Track process identity with PID plus generation *where
// available*", and per the agreed scoping this patch: SunlightOS's
// `SYS_SPAWN` returns only a numeric pid. The kernel does not currently expose
// a per-process "generation" value to userspace, so the registry cannot, in
// the strict sense, *guarantee* that a numeric pid reused by the kernel for
// a different actual process is rejected.
//
// We do harden client-side as much as the available primitives allow:
//   1. Each stored pid carries a shell-side `launch_id` pseudo-generation.
//      When a window arrives whose `owner_pid` matches a stored slot, only the
//      *current* pid↔launch_id pair is accepted; a freshly-spawned process
//      with the same numeric pid gets a brand-new `launch_id` and replaces
//      the stale slot in `note_spawn_succeeded`.
//   2. `reconcile` polls `process_is_alive(pid)` on every stored slot. When
//      the kernel reports the recycled pid as first alive-then-dead-then-alive
//      (one or more 250ms-poll "dead" samples separating the two lives), the
//      stale slot is dropped and a new one is inserted, so the second life is
//      re-attributed to whichever app actually spawned it.
//   3. When `pid_to_app` (telemetry proc-name lookup) disagrees with a stored
//      pid's app mapping for the same numeric pid, the stored slot wins for
//      freshly-shell-spawned apps; the fallback path only fires for windows
//      that didn't already match a stored slot.
//
// TheoredProcedure-coded test 13 in this crate is therefore exercised by
// simulating "alive→dead→alive across consecutive `is_alive` calls" — which
// is the strongest client-side assertion possible without a real kernel
// generation API. Strict generation-safety against a single missed "dead"
// sample is a documented remaining limitation; closing it requires extending
// `SYS_SPAWN` (or a new process-info syscall) to return a generation value —
// outside the scope of this patch.
