use alloc::string::String;
use alloc::vec::Vec;
use sunlight_ipc::{debug_log, kill, process_is_alive};

const ZOMBIE_SWEEP_INTERVAL_MS: u64 = 1000;
const TERMINATE_GRACE_PERIOD_MS: u64 = 500;
const SIGKILL: u32 = 9;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum LifecyclePolicy {
    ExitOnLastWindowClosed,
    KeepAlive,
    BackgroundAllowed,
    System,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum AppState {
    Launching,
    Running,
    Closing,
    Background,
    IdleNoWindows,
    Exited,
    Crashed,
}

pub struct AppInstance {
    app_name: String,
    pid: u64,
    windows: Vec<u64>,
    policy: LifecyclePolicy,
    state: AppState,
    last_window_close_time_ms: u64,
}

pub enum AppAction {
    None,
    Terminate(u64),
    SystemProtected,
}

pub struct SweepResult {
    pub pids_killed: Vec<u64>,
    pub window_ids_to_cleanup: Vec<u64>,
}

pub struct AppTracker {
    apps: Vec<AppInstance>,
    pub next_zombie_sweep_ms: u64,
}

impl AppTracker {
    pub fn new() -> Self {
        Self {
            apps: Vec::new(),
            next_zombie_sweep_ms: 0,
        }
    }

    pub fn register_window(&mut self, pid: u64, win_id: u64, app_name: &str, is_desktop: bool) {
        if let Some(app) = self.apps.iter_mut().find(|a| a.pid == pid) {
            app.windows.push(win_id);
            debug_log(&alloc::format!(
                "[APP_LIFECYCLE] app_window_attached pid={} win_id={} app_name={}\n",
                pid,
                win_id,
                app_name
            ));
            return;
        }

        let policy = if is_desktop || pid <= 1 {
            LifecyclePolicy::System
        } else {
            LifecyclePolicy::ExitOnLastWindowClosed
        };

        let mut app = AppInstance {
            app_name: String::from(app_name),
            pid,
            windows: Vec::new(),
            policy,
            state: AppState::Running,
            last_window_close_time_ms: 0,
        };
        app.windows.push(win_id);

        debug_log(&alloc::format!(
            "[APP_LIFECYCLE] app_instance_created pid={} name={} policy={:?}\n",
            pid,
            app_name,
            policy
        ));
        debug_log(&alloc::format!(
            "[APP_LIFECYCLE] app_window_attached pid={} win_id={}\n",
            pid,
            win_id
        ));
        debug_log(&alloc::format!(
            "[APP_LIFECYCLE] app_lifecycle_policy pid={} policy={:?}\n",
            pid,
            policy
        ));

        self.apps.push(app);
    }

    pub fn unregister_window(&mut self, win_id: u64, now_ms: u64) -> AppAction {
        let Some(app) = self.apps.iter_mut().find(|a| a.windows.contains(&win_id)) else {
            return AppAction::None;
        };

        app.windows.retain(|&id| id != win_id);

        debug_log(&alloc::format!(
            "[APP_LIFECYCLE] app_window_detached pid={} win_id={} remaining={}\n",
            app.pid,
            win_id,
            app.windows.len()
        ));

        if !app.windows.is_empty() {
            return AppAction::None;
        }

        debug_log(&alloc::format!(
            "[APP_LIFECYCLE] app_last_window_closed pid={} name={} policy={:?}\n",
            app.pid,
            app.app_name,
            app.policy
        ));

        match app.policy {
            LifecyclePolicy::ExitOnLastWindowClosed | LifecyclePolicy::BackgroundAllowed => {
                app.state = AppState::Closing;
                app.last_window_close_time_ms = now_ms;
                debug_log(&alloc::format!(
                    "[APP_LIFECYCLE] app_terminate_requested pid={} name={} signal=SIGTERM\n",
                    app.pid,
                    app.app_name
                ));
                AppAction::Terminate(app.pid)
            }
            LifecyclePolicy::KeepAlive => {
                app.state = AppState::IdleNoWindows;
                AppAction::None
            }
            LifecyclePolicy::System => AppAction::SystemProtected,
        }
    }

    /// Remove any AppInstance for a pid that has exited (called on detection).
    /// Emits app_instance_removed.
    pub fn remove_instance_for_pid(&mut self, pid: u64) -> bool {
        let before = self.apps.len();
        self.apps.retain(|a| {
            if a.pid == pid {
                debug_log(&alloc::format!(
                    "[APP_LIFECYCLE] app_instance_removed pid={} name={}\n",
                    pid,
                    a.app_name
                ));
                false
            } else {
                true
            }
        });
        self.apps.len() != before
    }

    pub fn sweep_zombies(&mut self, now_ms: u64) -> SweepResult {
        let mut pids_killed = Vec::new();
        let mut window_ids = Vec::new();

        let mut i = 0;
        while i < self.apps.len() {
            let app = &self.apps[i];
            let should_cleanup = match app.state {
                AppState::Closing => {
                    app.last_window_close_time_ms
                        .saturating_add(TERMINATE_GRACE_PERIOD_MS)
                        <= now_ms
                }
                AppState::Running | AppState::Launching => {
                    app.windows.is_empty() && app.policy != LifecyclePolicy::System
                }
                _ => false,
            };

            if should_cleanup {
                let alive = process_is_alive(app.pid);
                if alive {
                    let _ = kill(app.pid, SIGKILL);
                    pids_killed.push(app.pid);
                    debug_log(&alloc::format!(
                        "[APP_LIFECYCLE] app_terminate_forced pid={} name={} signal=SIGKILL\n",
                        app.pid,
                        app.app_name
                    ));
                }

                window_ids.extend(app.windows.iter().cloned());

                debug_log(&alloc::format!(
                    "[APP_LIFECYCLE] zombie_app_reaped pid={} name={} windows_cleaned={}\n",
                    app.pid,
                    app.app_name,
                    app.windows.len()
                ));

                debug_log(&alloc::format!(
                    "[APP_LIFECYCLE] app_exited pid={} name={}\n",
                    app.pid,
                    app.app_name
                ));

                // Record display windows reaped for this process exit.
                if !app.windows.is_empty() {
                    debug_log(&alloc::format!(
                        "[DISPLAY] display_windows_reaped_for_process pid={} count={}\n",
                        app.pid,
                        app.windows.len()
                    ));
                }

                debug_log(&alloc::format!(
                    "[APP_LIFECYCLE] app_instance_removed pid={} name={}\n",
                    app.pid,
                    app.app_name
                ));
                self.apps.remove(i);
            } else {
                let alive = process_is_alive(app.pid);
                if !alive && app.policy != LifecyclePolicy::System {
                    window_ids.extend(app.windows.iter().cloned());
                    if !app.windows.is_empty() {
                        debug_log(&alloc::format!(
                            "[DISPLAY] display_windows_reaped_for_process pid={} count={}\n",
                            app.pid,
                            app.windows.len()
                        ));
                    }
                    debug_log(&alloc::format!(
                        "[APP_LIFECYCLE] app_exited pid={} name={} (detected_dead)\n",
                        app.pid,
                        app.app_name
                    ));
                    debug_log(&alloc::format!(
                        "[APP_LIFECYCLE] app_instance_removed pid={} name={}\n",
                        app.pid,
                        app.app_name
                    ));
                    self.apps.remove(i);
                } else {
                    i += 1;
                }
            }
        }

        self.next_zombie_sweep_ms = now_ms.saturating_add(ZOMBIE_SWEEP_INTERVAL_MS);

        SweepResult {
            pids_killed,
            window_ids_to_cleanup: window_ids,
        }
    }
}
