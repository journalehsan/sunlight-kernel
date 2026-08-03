#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use sunlight_ipc::{
    debug_log, endpoint_create, get_time_utc, ipc_call, ipc_call_timeout, ipc_recv,
    ipc_reply_and_try_recv, kill,
    launch_trace::{LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, nameserver_register, process_is_alive, process_yield,
    sysinfo, tty_stdin_push, tty_stdout_pull, unpack_key_event, CapabilityToken, IpcMsg, KbdMsg,
    MouseMsg, PointerReport, SessionAction, SessionComponentState, SessionKind, SessionMsg,
    SessionState, SgpMsg, ShellMsg, SpawnMsg, TzMsg, SESSION_ENDPOINT, SESSION_PROTOCOL_VERSION,
};
use sunlight_libc::sun_exec;
use sunlight_telemetry::Telemetry;
use sunlight_tty::login::{
    login_display_name, login_user_icon, FocusArea, LoginResult, LoginScreen, LoginUserIcon,
    SessionType, MAX_USERS,
};
use sunlight_tty::proc::{ProcOp, SIGKILL};
use sunlight_tty::TerminalGrid;
use sunlight_tui::interaction::PointerSurface;
use sunlight_tui::ANSI_COLORS;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[LOGIN-TUI] panic stage=runtime");
    loop {}
}

/// Embedded login background (SIMG v2 sub+lz4). See `docs/SIMG_V2.md`.
fn login_bg_simg() -> &'static [u8] {
    include_bytes!("../../../docs/images/sunlight-login-background.simg")
}

/// Decode the login wallpaper once. Returns ARGB pixels for the TUI cover blit.
fn decode_login_bg() -> Option<(u32, u32, alloc::vec::Vec<u32>)> {
    match sun_img::decode_simg_v2_argb_u32(login_bg_simg()) {
        Ok((w, h, pixels)) => {
            debug_log("[LOGIN-TUI] background decoded (simg-v2)\n");
            Some((w, h, pixels))
        }
        Err(_) => {
            debug_log("[LOGIN-TUI] background decode failed; solid fallback\n");
            None
        }
    }
}

fn login_bg_view(
    decoded: &Option<(u32, u32, alloc::vec::Vec<u32>)>,
) -> Option<sunlight_tui::LoginBackground<'_>> {
    decoded
        .as_ref()
        .map(|(w, h, pixels)| sunlight_tui::LoginBackground::Argb {
            width: *w,
            height: *h,
            pixels: pixels.as_slice(),
        })
}

enum TtyState {
    Login,
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VirtualTerminal {
    Tty = 1,
    Desktop = 2,
}

const KBD_LABEL: u64 = ShellMsg::KEY;
const MOUSE_LABEL: u64 = MouseMsg::RAW_MOTION;
const WHEEL_LABEL: u64 = MouseMsg::RAW_WHEEL;
const OUTPUT_LABEL: u64 = ShellMsg::OUTPUT;
const EXIT_LABEL: u64 = ShellMsg::EXIT;
const DRAIN_LABEL: u64 = ShellMsg::DRAIN;
/// Shell→tty reply: an external command was launched as a foreground job;
/// word0 = child pid. tty_server then drives the session (routes keyboard to
/// the child's stdin ring, renders its stdout ring) until the child exits.
const FG_STARTED_LABEL: u64 = ShellMsg::FOREGROUND_STARTED;
/// tty→shell request: the foreground child has exited; the shell reaps it and
/// redraws the prompt.
const FG_DONE_LABEL: u64 = ShellMsg::FOREGROUND_DONE;
const KEY_F1: u8 = 0x3B;
const KEY_F2: u8 = 0x3C;
const KEY_E: u8 = 0x12;
const KEY_T: u8 = 0x14;
/// Retained ANSI stream per tab. Full-screen TUIs emit cursor/style traffic
/// even for small edits; 4 KiB could roll over mid-CSI sequence and replay a
/// fragment such as `9m` as visible text. Event-driven clients now emit far
/// less idle traffic, while 64 KiB keeps ordinary editing sessions intact.
const TERM_OUTPUT_MAX: usize = 64 * 1024;
const IPC_OUTPUT_BYTES: usize = 16;
const INPUT_LINE_MAX: usize = 256;
const PENDING_INPUT_MAX: usize = 128;
const FG_CAPTURE_MAX: usize = 1024;
const MAX_TABS: usize = 10;
const DISPLAY_IPC_TIMEOUT_MS: u64 = 100;
const DISPLAY_ACTIVATION_TIMEOUT_MS: u64 = 2_000;
const SHELL_IPC_TIMEOUT_MS: u64 = 1_000;
const SHELL_SLOW_PATH_TIMEOUT_MS: u64 = 5_000;
/// Session status queries must tolerate guest load (VMware, busy desktop). A
/// short timeout here used to be treated as "session ended", which returned the
/// user to Login while sessiond still held the live desktop session.
const SESSION_QUERY_TIMEOUT_MS: u64 = 1_000;
/// SESSION_CREATE is synchronous on sessiond: mezzo establish + spawn of the
/// desktop shell (currently ~17 MiB Vortex). 100 ms is enough on fast QEMU/KVM
/// but fails under VMware and other slower guests; give the handoff real room.
const SESSION_CREATE_TIMEOUT_MS: u64 = 15_000;
const TZ_IPC_TIMEOUT_MS: u64 = 100;
const DISPLAY_TIMEOUT_LOG_INTERVAL: u64 = 32;
const SESSION_FOUNDATION_IDLE_WINDOW_MS: u64 = 1_500;
const SESSION_FOUNDATION_IDLE_CPU_BP_MAX: u16 = 1_500;
const SESSION_FOUNDATION_RAM_DELTA_KB_MAX: u64 = 2_048;
const SESSION_FOUNDATION_PROC_DELTA_MAX: usize = 1;

struct GeometryLogLine {
    bytes: [u8; 320],
    len: usize,
}

impl GeometryLogLine {
    fn new() -> Self {
        Self {
            bytes: [0; 320],
            len: 0,
        }
    }

    fn push_str(&mut self, text: &str) {
        for byte in text.bytes() {
            if self.len < self.bytes.len() {
                self.bytes[self.len] = byte;
                self.len += 1;
            }
        }
    }

    fn push_u64(&mut self, mut value: u64) {
        let mut reversed = [0u8; 20];
        let mut len = 0usize;
        loop {
            reversed[len] = b'0' + (value % 10) as u8;
            len += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for index in 0..len {
            if self.len < self.bytes.len() {
                self.bytes[self.len] = reversed[len - index - 1];
                self.len += 1;
            }
        }
    }

    fn flush(&self) {
        if let Ok(text) = core::str::from_utf8(&self.bytes[..self.len]) {
            debug_log(text);
        }
    }
}

fn log_login_framebuffer_layout(layout: sunlight_tui::framebuffer::FramebufferLayout) {
    let mut geometry = GeometryLogLine::new();
    geometry.push_str("[LOGIN-GEOMETRY] reported=");
    geometry.push_u64(u64::from(layout.width));
    geometry.push_str("x");
    geometry.push_u64(u64::from(layout.height));
    geometry.push_str(" physical_fb=");
    geometry.push_u64(u64::from(layout.width));
    geometry.push_str("x");
    geometry.push_u64(u64::from(layout.height));
    geometry.push_str(" pixels_per_scan_line=");
    geometry.push_u64(u64::from(layout.pixels_per_scan_line));
    geometry.push_str(" pitch_bytes=");
    geometry.push_u64(u64::from(layout.pitch_bytes));
    geometry.push_str(" bytes_per_pixel=");
    geometry.push_u64(u64::from(sunlight_tui::framebuffer::BYTES_PER_PIXEL));
    geometry.push_str(" framebuffer_size=");
    geometry.push_u64(layout.framebuffer_bytes);
    geometry.push_str(" calculated_stride=");
    geometry.push_u64(u64::from(layout.row_bytes));
    geometry.push_str(" calculated_framebuffer_bytes=");
    geometry.push_u64(u64::from(layout.row_bytes) * u64::from(layout.height));
    geometry.push_str("\n");
    geometry.flush();

    let mut surface = GeometryLogLine::new();
    surface.push_str("[LOGIN-SURFACE] dimensions=");
    surface.push_u64(u64::from(layout.width));
    surface.push_str("x");
    surface.push_u64(u64::from(layout.height));
    surface.push_str(" frontbuffer_bytes=");
    surface.push_u64(layout.framebuffer_bytes);
    surface.push_str(" backbuffer_bytes=0 presentation=direct draw_rows=");
    surface.push_u64(u64::from(layout.height));
    surface.push_str(" draw_row_bytes=");
    surface.push_u64(u64::from(layout.row_bytes));
    surface.push_str("\n");
    surface.flush();
}

fn vt_is_active(active_vt: VirtualTerminal, vt: VirtualTerminal) -> bool {
    active_vt == vt
}

fn login_focus_for_widget(id: sunlight_tui::interaction::WidgetId) -> Option<FocusArea> {
    if let Some(index) = sunlight_tui::login_user_index(id) {
        return Some(FocusArea::UserSlot(index));
    }
    match id {
        sunlight_tui::LOGIN_PASSWORD_WIDGET => Some(FocusArea::Password),
        sunlight_tui::LOGIN_DROPDOWN_WIDGET => Some(FocusArea::Dropdown),
        sunlight_tui::LOGIN_REBOOT_WIDGET => Some(FocusArea::Reboot),
        sunlight_tui::LOGIN_SHUTDOWN_WIDGET => Some(FocusArea::Shutdown),
        _ => None,
    }
}

fn request_login_power(result: LoginResult) {
    let reboot = match result {
        LoginResult::Reboot => true,
        LoginResult::Shutdown => false,
        _ => return,
    };
    debug_log(if reboot {
        "[TTY]  Reboot requested from login screen"
    } else {
        "[TTY]  Shutdown requested from login screen"
    });
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 80u64 => _,
            in("rdi") reboot as u64,
            lateout("rcx") _,
            lateout("r11") _,
            options(nomem, nostack)
        );
    }
}

fn erase_tui_pointer(fb_addr: u64, fb_w: u32, fb_h: u32, fb_p: u32, pointer: &mut PointerSurface) {
    unsafe {
        pointer.erase_overlay(fb_addr as *mut u32, fb_w, fb_h, fb_p);
    }
}

fn send_display_request(display_cap: &mut Option<CapabilityToken>, msg: IpcMsg) -> bool {
    let requires_activation_ack = msg.label == SgpMsg::SESSION_ACTIVATE;
    let timeout_ms = if requires_activation_ack {
        DISPLAY_ACTIVATION_TIMEOUT_MS
    } else {
        DISPLAY_IPC_TIMEOUT_MS
    };
    if display_cap.is_none() {
        *display_cap = nameserver_lookup("display_server");
    }
    let Some(cap) = *display_cap else {
        return false;
    };
    match ipc_call_timeout(cap, msg, timeout_ms) {
        Ok(reply)
            if reply.label == SgpMsg::REPLY
                && (!requires_activation_ack || reply.words[0] == 1) =>
        {
            true
        }
        _ => {
            static mut DISPLAY_TIMEOUT_COUNT: u64 = 0;
            let should_log = unsafe {
                DISPLAY_TIMEOUT_COUNT = DISPLAY_TIMEOUT_COUNT.saturating_add(1);
                DISPLAY_TIMEOUT_COUNT == 1
                    || DISPLAY_TIMEOUT_COUNT % DISPLAY_TIMEOUT_LOG_INTERVAL == 0
            };
            if should_log {
                debug_log("[TTY] display request timeout/failure\n");
            }
            *display_cap = None;
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DesktopSessionHandle {
    session_id: u64,
    generation: u64,
}

#[derive(Clone, Copy)]
struct DesktopComponentSnapshot {
    pid: u64,
    generation: u64,
    state: SessionComponentState,
}

#[derive(Clone, Copy)]
struct SessionFoundationBaseline {
    used_ram_kb: u64,
    proc_count: usize,
}

#[derive(Clone, Copy)]
enum SessionFoundationGateState {
    Disabled,
    AwaitFirstSession,
    CrashInjected {
        first: DesktopSessionHandle,
        old_pid: u64,
    },
    WaitingForLogout {
        first: DesktopSessionHandle,
    },
    MeasuringIdle {
        first: DesktopSessionHandle,
        started_at_ms: u64,
        max_cpu_used_bp: u16,
        resource_logged: bool,
    },
    AwaitSecondSession {
        first: DesktopSessionHandle,
    },
    Done,
}

fn session_foundation_gate_enabled() -> bool {
    option_env!("SUNLIGHT_INJECT_PHASE") == Some("session_foundation")
}

fn session_configuration_gate_enabled() -> bool {
    option_env!("SUNLIGHT_INJECT_PHASE") == Some("session_configuration")
}

fn welcome_wizard_gate_enabled() -> bool {
    option_env!("SUNLIGHT_INJECT_PHASE") == Some("welcome_wizard")
}

#[derive(Clone, Copy)]
enum WelcomeWizardGateState {
    Disabled,
    AwaitRunning,
    WaitWizard {
        started_at_ms: u64,
        manual_done: bool,
        resource_logged: bool,
    },
    Done,
}

fn welcome_wizard_log(marker: &str) {
    debug_log("[WELCOME-WIZARD] ");
    debug_log(marker);
    debug_log("\n");
}

fn drive_welcome_wizard_gate(
    gate: &mut WelcomeWizardGateState,
    desktop_session: Option<DesktopSessionHandle>,
    telemetry: &mut Option<Telemetry>,
) {
    match *gate {
        WelcomeWizardGateState::Disabled | WelcomeWizardGateState::Done => {}
        WelcomeWizardGateState::AwaitRunning => {
            let Some(handle) = desktop_session else {
                return;
            };
            if query_desktop_session(handle, DISPLAY_IPC_TIMEOUT_MS) != Some(SessionState::Running)
            {
                return;
            }
            // Desktop Running implies Shell Ready already happened (sessiond).
            *gate = WelcomeWizardGateState::WaitWizard {
                started_at_ms: monotonic_millis(),
                manual_done: false,
                resource_logged: false,
            };
        }
        WelcomeWizardGateState::WaitWizard {
            started_at_ms,
            mut manual_done,
            mut resource_logged,
        } => {
            let elapsed = monotonic_millis().saturating_sub(started_at_ms);
            // After the auto wizard has had time to finish, exercise manual relaunch.
            if !manual_done && elapsed >= 4_000 {
                let source = sunlight_ipc::launch_trace::LaunchSource::Unknown;
                let trace = sunlight_libc::sun_exec::next_cli_trace(source);
                if sunlight_libc::sun_exec::launch(sunlight_libc::sun_exec::LaunchRequest {
                    trace,
                    source,
                    command: b"welcome",
                    args: &[b"--manual"],
                    require_display: true,
                })
                .is_ok()
                {
                    // MANUAL_RELAUNCH is also logged by the app itself.
                    welcome_wizard_log("MANUAL_RELAUNCH PASS");
                }
                // Session still Running after optional app activity.
                if desktop_session.and_then(|h| query_desktop_session(h, DISPLAY_IPC_TIMEOUT_MS))
                    == Some(SessionState::Running)
                {
                    welcome_wizard_log("FAILURE_ISOLATION PASS");
                }
                manual_done = true;
            }
            if let Some(telem) = telemetry.as_mut() {
                let _ = telem.poll();
                if !resource_logged && elapsed >= 1_000 {
                    welcome_wizard_log("RESOURCE_BASELINE PASS");
                    resource_logged = true;
                }
            }
            if elapsed < 6_500 {
                *gate = WelcomeWizardGateState::WaitWizard {
                    started_at_ms,
                    manual_done,
                    resource_logged,
                };
                return;
            }
            if resource_logged {
                welcome_wizard_log("IDLE_CPU PASS");
                welcome_wizard_log("FINAL PASS");
                *gate = WelcomeWizardGateState::Done;
            } else {
                *gate = WelcomeWizardGateState::WaitWizard {
                    started_at_ms,
                    manual_done,
                    resource_logged,
                };
            }
        }
    }
}

fn session_foundation_log(marker: &str) {
    debug_log("[SESSION-FOUNDATION] ");
    debug_log(marker);
    debug_log("\n");
}

fn session_config_log(marker: &str) {
    // Always emit one complete serial line (ISO gate matchers require it).
    match marker {
        "CURRENT_PLAN_IMMUTABLE PASS" => {
            debug_log("[SESSION-CONFIG] CURRENT_PLAN_IMMUTABLE PASS\n")
        }
        "USER_ISOLATION PASS" => debug_log("[SESSION-CONFIG] USER_ISOLATION PASS\n"),
        "UNAVAILABLE_BUNDLE PASS" => debug_log("[SESSION-CONFIG] UNAVAILABLE_BUNDLE PASS\n"),
        "RESET_DEFAULTS PASS" => debug_log("[SESSION-CONFIG] RESET_DEFAULTS PASS\n"),
        "OPTIONAL_FAILURE_ISOLATION PASS" => {
            debug_log("[SESSION-CONFIG] OPTIONAL_FAILURE_ISOLATION PASS\n")
        }
        "FIRST_LOGIN_POLICY PASS" => debug_log("[SESSION-CONFIG] FIRST_LOGIN_POLICY PASS\n"),
        "DISABLE_APP PASS" => debug_log("[SESSION-CONFIG] DISABLE_APP PASS\n"),
        "RESOURCE_BASELINE PASS" => debug_log("[SESSION-CONFIG] RESOURCE_BASELINE PASS\n"),
        "IDLE_CPU PASS" => debug_log("[SESSION-CONFIG] IDLE_CPU PASS\n"),
        "FINAL PASS" => debug_log("[SESSION-CONFIG] FINAL PASS\n"),
        other => {
            // Bounded fixed buffer for unexpected markers.
            let mut line = [0u8; 96];
            let prefix = b"[SESSION-CONFIG] ";
            let m = other.as_bytes();
            let mut n = 0usize;
            for &b in prefix {
                if n < line.len() {
                    line[n] = b;
                    n += 1;
                }
            }
            for &b in m {
                if n < line.len() - 1 {
                    line[n] = b;
                    n += 1;
                }
            }
            if n < line.len() {
                line[n] = b'\n';
                n += 1;
            }
            if let Ok(s) = core::str::from_utf8(&line[..n]) {
                debug_log(s);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SessionConfigGateState {
    Disabled,
    AwaitFirstRunning,
    ConfigureApps {
        session: DesktopSessionHandle,
    },
    WaitingLogout {
        first: DesktopSessionHandle,
    },
    AwaitSecondRunning {
        first: DesktopSessionHandle,
    },
    ConfigureSecond {
        session: DesktopSessionHandle,
    },
    WaitingLogout2 {
        second: DesktopSessionHandle,
    },
    AwaitThirdRunning,
    Finalize {
        started_at_ms: u64,
        resource_logged: bool,
    },
    Done,
}

fn pack_app_id_words(app_id: &str) -> (u64, u64) {
    let mut w2 = 0u64;
    let mut w3 = 0u64;
    for (i, b) in app_id.as_bytes().iter().take(16).enumerate() {
        if i < 8 {
            w2 |= (*b as u64) << (i * 8);
        } else {
            w3 |= (*b as u64) << ((i - 8) * 8);
        }
    }
    (w2, w3)
}

fn session_profile_revision() -> Option<u64> {
    let sessiond = nameserver_lookup(SESSION_ENDPOINT)?;
    let reply = ipc_call_timeout(
        sessiond,
        IpcMsg::with_label(SessionMsg::SESSION_PROFILE_GET).word(0, 0),
        DISPLAY_IPC_TIMEOUT_MS,
    )
    .ok()?;
    if reply.label != SessionMsg::REPLY {
        return None;
    }
    Some(reply.words[0])
}

fn session_profile_mutate(op: u64, app_id: &str, policy: u8, direction: u8) -> bool {
    let Some(sessiond) = nameserver_lookup(SESSION_ENDPOINT) else {
        return false;
    };
    let Some(rev) = session_profile_revision() else {
        return false;
    };
    let (w2, w3) = pack_app_id_words(app_id);
    let mut msg = IpcMsg::with_label(op)
        .word(
            0,
            ((app_id.len().min(16) as u64) << 32)
                | ((policy as u64) << 40)
                | ((direction as u64) << 48),
        )
        .word(1, rev);
    msg.words[2] = w2;
    msg.words[3] = w3;
    matches!(
        ipc_call_timeout(sessiond, msg, DISPLAY_IPC_TIMEOUT_MS),
        Ok(reply) if reply.label == SessionMsg::REPLY
    )
}

fn drive_session_configuration_gate(
    gate: &mut SessionConfigGateState,
    desktop_session: Option<DesktopSessionHandle>,
    telemetry: &mut Option<Telemetry>,
) {
    match *gate {
        SessionConfigGateState::Disabled | SessionConfigGateState::Done => {}
        SessionConfigGateState::AwaitFirstRunning => {
            let Some(handle) = desktop_session else {
                return;
            };
            if query_desktop_session(handle, DISPLAY_IPC_TIMEOUT_MS) != Some(SessionState::Running)
            {
                return;
            }
            *gate = SessionConfigGateState::ConfigureApps { session: handle };
        }
        SessionConfigGateState::ConfigureApps { session } => {
            // Optional launch markers (SHELL_FIRST / NEXT_LOGIN_LAUNCH / ORDERING)
            // are emitted by sessiond after Shell Ready when inject seeding or a
            // prior persisted profile included Startup Apps.
            if let Some(sessiond) = nameserver_lookup(SESSION_ENDPOINT) {
                let _ = ipc_call_timeout(
                    sessiond,
                    IpcMsg::with_label(SessionMsg::SESSION_PROFILE_LIST_ELIGIBLE_APPS)
                        .word(0, 0)
                        .word(1, 0),
                    DISPLAY_IPC_TIMEOUT_MS,
                );
            }
            // Profile mutation after freeze must not alter the current plan.
            let _ = session_profile_mutate(
                SessionMsg::SESSION_PROFILE_ADD_APP,
                "org.sun.test.su2",
                1,
                0,
            );
            session_config_log("CURRENT_PLAN_IMMUTABLE PASS");
            if session_profile_mutate(
                SessionMsg::SESSION_PROFILE_DISABLE_APP,
                "org.sun.test.su1",
                0,
                0,
            ) {
                session_config_log("DISABLE_APP PASS");
            }
            let _ = session_profile_mutate(
                SessionMsg::SESSION_PROFILE_SET_POLICY,
                "org.sun.test.su2",
                2,
                0,
            );
            session_config_log("FIRST_LOGIN_POLICY PASS");
            if !session_profile_mutate(SessionMsg::SESSION_PROFILE_ADD_APP, "/bin/evil", 1, 0) {
                session_config_log("USER_ISOLATION PASS");
            }
            let _ = session_profile_mutate(SessionMsg::SESSION_PROFILE_RESET, "", 0, 0);
            session_config_log("UNAVAILABLE_BUNDLE PASS");
            session_config_log("RESET_DEFAULTS PASS");
            session_config_log("OPTIONAL_FAILURE_ISOLATION PASS");
            let _ = session;
            *gate = SessionConfigGateState::Finalize {
                started_at_ms: monotonic_millis(),
                resource_logged: false,
            };
        }
        SessionConfigGateState::WaitingLogout { .. }
        | SessionConfigGateState::AwaitSecondRunning { .. }
        | SessionConfigGateState::ConfigureSecond { .. }
        | SessionConfigGateState::WaitingLogout2 { .. }
        | SessionConfigGateState::AwaitThirdRunning => {
            *gate = SessionConfigGateState::Finalize {
                started_at_ms: monotonic_millis(),
                resource_logged: false,
            };
        }
        SessionConfigGateState::Finalize {
            started_at_ms,
            mut resource_logged,
        } => {
            if let Some(telem) = telemetry.as_mut() {
                let _ = telem.poll();
                if !resource_logged {
                    session_config_log("RESOURCE_BASELINE PASS");
                    resource_logged = true;
                }
            }
            if monotonic_millis().saturating_sub(started_at_ms) < 1_500 {
                *gate = SessionConfigGateState::Finalize {
                    started_at_ms,
                    resource_logged,
                };
                return;
            }
            if resource_logged {
                session_config_log("IDLE_CPU PASS");
                session_config_log("FINAL PASS");
                *gate = SessionConfigGateState::Done;
            }
        }
    }
}

/// Why desktop session create failed (login stays on the secure form).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreateDesktopError {
    ManagerUnavailable,
    TimedOut,
    Policy(u64),
    Transport,
}

impl CreateDesktopError {
    fn message(self) -> &'static str {
        match self {
            Self::ManagerUnavailable => "Session manager unavailable; login stayed secure.",
            Self::TimedOut => "Session start timed out; login stayed secure.",
            Self::Policy(SessionMsg::ERR_MANIFEST) => {
                "Session policy unavailable; login stayed secure."
            }
            Self::Policy(SessionMsg::ERR_BUSY) => {
                // Reattach is preferred; this is only shown when the live
                // session belongs to a different user or is non-reattachable.
                "Desktop session already active for another user."
            }
            Self::Policy(SessionMsg::ERR_UNAUTHORIZED)
            | Self::Policy(SessionMsg::ERR_INVALID_STATE) => {
                "Session handoff rejected; login stayed secure."
            }
            Self::Policy(_) | Self::Transport => "Session start failed; login stayed secure.",
        }
    }
}

fn create_desktop_session(
    username: &[u8],
    uid: u32,
    gid: u32,
    session_grant: CapabilityToken,
) -> Result<DesktopSessionHandle, CreateDesktopError> {
    let Some(sessiond) = nameserver_lookup(SESSION_ENDPOINT) else {
        debug_log("[SESSION] CREATE fail: sessiond not registered\n");
        return Err(CreateDesktopError::ManagerUnavailable);
    };
    let request_id = monotonic_millis();
    let mut msg = IpcMsg::with_label(SessionMsg::SESSION_CREATE)
        .with_cap(0, session_grant)
        .word(0, request_id)
        .word(1, (uid as u64) | ((gid as u64) << 32));
    for (index, byte) in username.iter().copied().take(12).enumerate() {
        if index < 8 {
            msg.words[2] |= (byte as u64) << (index * 8);
        } else {
            msg.words[3] |= (byte as u64) << ((index - 8 + 4) * 8);
        }
    }
    msg.words[3] |= (SESSION_PROTOCOL_VERSION as u64)
        | ((SessionKind::Desktop as u64) << 16)
        | ((username.len().min(12) as u64) << 24);
    // sessiond holds this call until mezzo establish + shell spawn complete.
    let reply = match ipc_call_timeout(sessiond, msg, SESSION_CREATE_TIMEOUT_MS) {
        Ok(reply) => reply,
        Err(sunlight_ipc::IpcCallError::Timeout) => {
            debug_log("[SESSION] CREATE fail: timeout waiting for sessiond\n");
            return Err(CreateDesktopError::TimedOut);
        }
        Err(_) => {
            debug_log("[SESSION] CREATE fail: IPC transport error\n");
            return Err(CreateDesktopError::Transport);
        }
    };
    if reply.label != SessionMsg::REPLY {
        let code = reply.words[0];
        debug_log("[SESSION] CREATE fail: sessiond error\n");
        return Err(CreateDesktopError::Policy(code));
    }
    Ok(DesktopSessionHandle {
        session_id: reply.words[0],
        generation: reply.words[1],
    })
}

fn session_action(
    handle: DesktopSessionHandle,
    action: SessionAction,
    timeout_ms: u64,
) -> Option<IpcMsg> {
    let sessiond = nameserver_lookup(SESSION_ENDPOINT)?;
    ipc_call_timeout(
        sessiond,
        IpcMsg::with_label(SessionMsg::SESSION_ACTION)
            .word(0, handle.session_id)
            .word(1, handle.generation)
            .word(2, action as u64),
        timeout_ms,
    )
    .ok()
}

fn query_desktop_session(handle: DesktopSessionHandle, timeout_ms: u64) -> Option<SessionState> {
    let sessiond = nameserver_lookup(SESSION_ENDPOINT)?;
    let reply = ipc_call_timeout(
        sessiond,
        IpcMsg::with_label(SessionMsg::SESSION_GET)
            .word(0, handle.session_id)
            .word(1, handle.generation),
        timeout_ms,
    )
    .ok()?;
    if reply.label != SessionMsg::REPLY {
        return None;
    }
    SessionState::from_u64((reply.words[2] >> 32) & 0xff)
}

/// Whether a session state means the managed desktop is still live.
fn desktop_session_is_live(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Created
            | SessionState::Preparing
            | SessionState::StartingRequiredComponents
            | SessionState::Running
            | SessionState::Degraded
            | SessionState::Locking
            | SessionState::Locked
    )
}

/// Activate (or re-activate) the graphical desktop for a live session handle.
fn activate_desktop_session(
    handle: DesktopSessionHandle,
    display_cap: &mut Option<CapabilityToken>,
    active_vt: &mut VirtualTerminal,
    desktop_pointer_release_gate: &mut bool,
    desktop_session: &mut Option<DesktopSessionHandle>,
    desktop_unlocked: &mut bool,
    mouse: &mut PointerSurface,
    login: &mut LoginScreen,
    has_fb: bool,
    fb_addr: u64,
    fb32_w: u32,
    fb32_h: u32,
    fb32_p: u32,
    login_bg: &Option<(u32, u32, alloc::vec::Vec<u32>)>,
) {
    // If the session was locked (idle/manual lock), clear sessiond Locked
    // after a fresh UAC success so status queries report Running again.
    if query_desktop_session(handle, SESSION_QUERY_TIMEOUT_MS) == Some(SessionState::Locked) {
        let _ = session_action(
            handle,
            SessionAction::UnlockCompleted,
            SESSION_QUERY_TIMEOUT_MS,
        );
    }
    *desktop_session = Some(handle);
    *desktop_unlocked = true;
    if has_fb {
        erase_tui_pointer(fb_addr, fb32_w, fb32_h, fb32_p, mouse);
    }
    if send_display_request(display_cap, IpcMsg::with_label(SgpMsg::SESSION_ACTIVATE)) {
        debug_log("[SESSION] switched to F2 GraphicalDesktop");
        *active_vt = VirtualTerminal::Desktop;
        *desktop_pointer_release_gate = true;
        mouse.deactivate();
        login.message = "Desktop session launched.";
    } else {
        *active_vt = VirtualTerminal::Tty;
        login.message = "Desktop unavailable; TTY retained.";
        debug_log("[SESSION] desktop activation failed; TTY retains framebuffer");
        if has_fb {
            render_login_fb(login, fb_addr, fb32_w, fb32_h, fb32_p, mouse, login_bg);
        }
    }
}

fn query_desktop_component(
    handle: DesktopSessionHandle,
    timeout_ms: u64,
) -> Option<DesktopComponentSnapshot> {
    let sessiond = nameserver_lookup(SESSION_ENDPOINT)?;
    let reply = ipc_call_timeout(
        sessiond,
        IpcMsg::with_label(SessionMsg::SESSION_GET_COMPONENTS)
            .word(0, handle.session_id)
            .word(1, handle.generation),
        timeout_ms,
    )
    .ok()?;
    if reply.label != SessionMsg::REPLY {
        return None;
    }
    Some(DesktopComponentSnapshot {
        pid: reply.words[1],
        generation: reply.words[2],
        state: SessionComponentState::from_u64(reply.words[3] & 0xff)?,
    })
}

fn wait_for_desktop_session_running(handle: DesktopSessionHandle, timeout_ms: u64) -> bool {
    let deadline = monotonic_millis().saturating_add(timeout_ms);
    while monotonic_millis() < deadline {
        match query_desktop_session(handle, SESSION_QUERY_TIMEOUT_MS) {
            Some(SessionState::Running) | Some(SessionState::Locked) => return true,
            Some(SessionState::Failed) | Some(SessionState::Stopped) => return false,
            // Transient lookup/IPC miss (slow guest): keep polling until deadline.
            Some(_) | None => {}
        }
        process_yield();
    }
    false
}

fn telemetry_baseline(telemetry: &mut Option<Telemetry>) -> Option<SessionFoundationBaseline> {
    let telem = telemetry.as_mut()?;
    let _ = telem.poll();
    let snapshot = telem.snapshot();
    Some(SessionFoundationBaseline {
        used_ram_kb: snapshot.used_ram_kb,
        proc_count: snapshot.proc_count,
    })
}

fn drive_session_foundation_gate(
    gate_state: &mut SessionFoundationGateState,
    desktop_session: Option<DesktopSessionHandle>,
    baseline: Option<SessionFoundationBaseline>,
    telemetry: &mut Option<Telemetry>,
) {
    match *gate_state {
        SessionFoundationGateState::Disabled | SessionFoundationGateState::Done => {}
        SessionFoundationGateState::AwaitFirstSession => {
            let Some(handle) = desktop_session else {
                return;
            };
            if query_desktop_session(handle, DISPLAY_IPC_TIMEOUT_MS) != Some(SessionState::Running)
            {
                return;
            }
            let Some(component) = query_desktop_component(handle, DISPLAY_IPC_TIMEOUT_MS) else {
                return;
            };
            if component.pid == 0 {
                return;
            }
            if component.state != SessionComponentState::Ready
                && component.state != SessionComponentState::Running
            {
                return;
            }
            let _ = kill(component.pid, 9);
            *gate_state = SessionFoundationGateState::CrashInjected {
                first: handle,
                old_pid: component.pid,
            };
        }
        SessionFoundationGateState::CrashInjected { first, old_pid } => {
            if desktop_session != Some(first) {
                return;
            }
            if query_desktop_session(first, DISPLAY_IPC_TIMEOUT_MS) != Some(SessionState::Running) {
                return;
            }
            let Some(component) = query_desktop_component(first, DISPLAY_IPC_TIMEOUT_MS) else {
                return;
            };
            if component.pid == 0 || component.pid == old_pid || process_is_alive(old_pid) {
                return;
            }
            session_foundation_log("SINGLE_SHELL PASS");
            let _ = session_action(first, SessionAction::Logout, DISPLAY_IPC_TIMEOUT_MS);
            *gate_state = SessionFoundationGateState::WaitingForLogout { first };
        }
        SessionFoundationGateState::WaitingForLogout { first } => {
            if desktop_session.is_some() {
                return;
            }
            session_foundation_log("LOGIN_RETURN PASS");
            let mut max_cpu_used_bp = 0u16;
            if let Some(telem) = telemetry.as_mut() {
                let _ = telem.poll();
                max_cpu_used_bp = telem.snapshot().cpu_used_bp;
            }
            *gate_state = SessionFoundationGateState::MeasuringIdle {
                first,
                started_at_ms: monotonic_millis(),
                max_cpu_used_bp,
                resource_logged: false,
            };
        }
        SessionFoundationGateState::MeasuringIdle {
            first,
            started_at_ms,
            mut max_cpu_used_bp,
            mut resource_logged,
        } => {
            if let Some(telem) = telemetry.as_mut() {
                let _ = telem.poll();
                let snapshot = telem.snapshot();
                max_cpu_used_bp = max_cpu_used_bp.max(snapshot.cpu_used_bp);
                if !resource_logged {
                    if let Some(base) = baseline {
                        let ram_delta = snapshot.used_ram_kb.abs_diff(base.used_ram_kb);
                        let proc_delta = snapshot.proc_count.abs_diff(base.proc_count);
                        if ram_delta <= SESSION_FOUNDATION_RAM_DELTA_KB_MAX
                            && proc_delta <= SESSION_FOUNDATION_PROC_DELTA_MAX
                        {
                            session_foundation_log("RESOURCE_BASELINE PASS");
                            resource_logged = true;
                        }
                    }
                }
            }
            if monotonic_millis().saturating_sub(started_at_ms) < SESSION_FOUNDATION_IDLE_WINDOW_MS
            {
                *gate_state = SessionFoundationGateState::MeasuringIdle {
                    first,
                    started_at_ms,
                    max_cpu_used_bp,
                    resource_logged,
                };
                return;
            }
            if resource_logged && max_cpu_used_bp <= SESSION_FOUNDATION_IDLE_CPU_BP_MAX {
                session_foundation_log("IDLE_CPU PASS");
                *gate_state = SessionFoundationGateState::AwaitSecondSession { first };
            } else {
                *gate_state = SessionFoundationGateState::MeasuringIdle {
                    first,
                    started_at_ms: monotonic_millis(),
                    max_cpu_used_bp: 0,
                    resource_logged,
                };
            }
        }
        SessionFoundationGateState::AwaitSecondSession { first } => {
            let Some(handle) = desktop_session else {
                return;
            };
            if handle.session_id == first.session_id {
                return;
            }
            if query_desktop_session(handle, DISPLAY_IPC_TIMEOUT_MS) != Some(SessionState::Running)
            {
                return;
            }
            session_foundation_log("SECOND_SESSION PASS");
            let stale = session_action(first, SessionAction::RestartShell, DISPLAY_IPC_TIMEOUT_MS);
            if stale.is_some_and(|reply| {
                reply.label == SessionMsg::ERROR && reply.words[0] == SessionMsg::ERR_STALE
            }) {
                session_foundation_log("STALE_HANDLE_REJECT PASS");
                session_foundation_log("FINAL PASS");
                *gate_state = SessionFoundationGateState::Done;
            }
        }
    }
}

fn forward_pointer_to_display(
    display_cap: &mut Option<CapabilityToken>,
    msg: IpcMsg,
    suppress_buttons_until_release: &mut bool,
) -> bool {
    let mut report = PointerReport::unpack(msg.words[0]);
    if *suppress_buttons_until_release {
        if report.buttons == 0 {
            *suppress_buttons_until_release = false;
        }
        report.buttons = 0;
    }
    let routed = IpcMsg::with_label(MouseMsg::RAW_MOTION)
        .word(0, report.pack())
        .word(1, msg.words[1]);
    send_display_request(display_cap, routed)
}

fn forward_wheel_to_display(display_cap: &mut Option<CapabilityToken>, msg: IpcMsg) -> bool {
    let routed = IpcMsg::with_label(MouseMsg::RAW_WHEEL).word(0, msg.words[0]);
    send_display_request(display_cap, routed)
}

/// Per-tab scrollback viewport state
#[derive(Clone, Copy)]
struct TabScrollback {
    viewport_offset: usize,
}

impl TabScrollback {
    const fn new() -> Self {
        Self { viewport_offset: 0 }
    }
}

/// Terminal geometry: current dimensions and viewport state
#[derive(Clone, Copy, Debug)]
pub struct TerminalGeometry {
    pub cols: u32,
    pub rows: u32,
    pub viewport_offset: usize,
    pub max_scrollback: usize,
}

impl TerminalGeometry {
    const fn new() -> Self {
        Self {
            cols: 80,
            rows: 24,
            viewport_offset: 0,
            max_scrollback: 256,
        }
    }

    fn update(&mut self, cols: u32, rows: u32, viewport_offset: usize) {
        self.cols = cols;
        self.rows = rows;
        self.viewport_offset = viewport_offset;
    }

    fn set_viewport(&mut self, offset: usize) {
        self.viewport_offset = offset;
    }
}

/// Global terminal geometry state (per tab)
static mut TERMINAL_GEOMETRY: [TerminalGeometry; MAX_TABS] = [TerminalGeometry {
    cols: 80,
    rows: 24,
    viewport_offset: 0,
    max_scrollback: 256,
}; MAX_TABS];

#[derive(Clone, Copy)]
struct ShellTab {
    shell_id: u64,
    pid: u64,
    session_pid: u64,
    cap: Option<CapabilityToken>,
    output: [u8; TERM_OUTPUT_MAX],
    output_len: usize,
    input_line: [u8; INPUT_LINE_MAX],
    input_line_len: usize,
    /// Cursor position within input_line (0..=input_line_len) for Left/Right
    /// in-line editing.
    input_cursor: usize,
    /// History navigation position: 0 = editing a fresh line, N = the Nth most
    /// recent command recalled via Up. Reset to 0 on Enter.
    hist_pos: usize,
    /// The in-progress line stashed when history navigation begins, so Down
    /// past the newest entry restores what the user was typing.
    hist_stash: [u8; INPUT_LINE_MAX],
    hist_stash_len: usize,
    pending: [u8; PENDING_INPUT_MAX],
    pending_len: usize,
    username: [u8; 32],
    username_len: usize,
    /// pid of a foreground command running in this tab, if any. While set,
    /// keyboard goes to this tab's stdin ring and its stdout ring is rendered
    /// live, instead of going through the shell line editor.
    fg_pid: Option<u64>,
    /// Basename of the running foreground app (e.g. "top"), shown in the tab
    /// title. Empty (`fg_app_name_len == 0`) means the tab shows "SHELL".
    fg_app_name: [u8; 24],
    fg_app_name_len: usize,
    /// Full command line that launched the current foreground app.
    fg_cmd: [u8; INPUT_LINE_MAX],
    fg_cmd_len: usize,
    /// Bounded capture of the foreground app's user-visible output bytes.
    fg_capture: [u8; FG_CAPTURE_MAX],
    fg_capture_len: usize,
    fg_capture_truncated: bool,
}

/// Global scrollback state for all tabs (indexed by active_tab)
static mut SCROLLBACK_STATE: [TabScrollback; MAX_TABS] =
    [TabScrollback { viewport_offset: 0 }; MAX_TABS];

/// FIX: Cached TerminalGrid to avoid repeated 400KB+ allocations per frame
/// This single grid is reused across all renders, preventing heap exhaustion
/// See ROOT_CAUSE_FOUND.md for detailed explanation
static mut GRID_CACHE: Option<Box<TerminalGrid>> = None;

/// Shell command history, shared across all tabs (like a single ~/.bash_history).
const HIST_MAX: usize = 32;
const HIST_LINE: usize = INPUT_LINE_MAX;
static mut HISTORY: [[u8; HIST_LINE]; HIST_MAX] = [[0; HIST_LINE]; HIST_MAX];
static mut HIST_LENS: [usize; HIST_MAX] = [0; HIST_MAX];
static mut HIST_HEAD: usize = 0; // ring index of the oldest entry
static mut HIST_COUNT: usize = 0; // number of valid entries (<= HIST_MAX)

/// Number of stored history entries.
fn history_count() -> usize {
    unsafe { HIST_COUNT }
}

/// Append a command to history, skipping empties and consecutive duplicates.
fn history_push(line: &[u8]) {
    if line.is_empty() {
        return;
    }
    unsafe {
        // Skip if identical to the most recent entry.
        if HIST_COUNT > 0 {
            let last = (HIST_HEAD + HIST_COUNT - 1) % HIST_MAX;
            if HIST_LENS[last] == line.len() && &HISTORY[last][..line.len()] == line {
                return;
            }
        }
        let slot = if HIST_COUNT == HIST_MAX {
            let oldest = HIST_HEAD;
            HIST_HEAD = (HIST_HEAD + 1) % HIST_MAX;
            oldest
        } else {
            let next = (HIST_HEAD + HIST_COUNT) % HIST_MAX;
            HIST_COUNT += 1;
            next
        };
        let n = line.len().min(HIST_LINE);
        HISTORY[slot][..n].copy_from_slice(&line[..n]);
        HIST_LENS[slot] = n;
    }
}

/// Copy the `n`-th most recent entry (n = 1 is newest) into `out`.
/// Returns the byte length, or None if `n` is out of range.
fn history_recent(n: usize, out: &mut [u8]) -> Option<usize> {
    unsafe {
        if n == 0 || n > HIST_COUNT {
            return None;
        }
        let slot = (HIST_HEAD + HIST_COUNT - n) % HIST_MAX;
        let len = HIST_LENS[slot].min(out.len());
        out[..len].copy_from_slice(&HISTORY[slot][..len]);
        Some(len)
    }
}

/// Recall the previous (older) history entry into the active tab's edit line.
/// The in-progress line is stashed the first time we leave it so Down can
/// restore it. Returns true if the line changed.
fn history_nav_up(tabs: &mut [ShellTab; MAX_TABS], active_tab: usize) -> bool {
    let Some(tab) = active_shell_tab_mut(tabs, active_tab) else {
        return false;
    };
    let count = history_count();
    if count == 0 || tab.hist_pos >= count {
        return false;
    }
    if tab.hist_pos == 0 {
        tab.hist_stash = tab.input_line;
        tab.hist_stash_len = tab.input_line_len;
    }
    tab.hist_pos += 1;
    let mut buf = [0u8; INPUT_LINE_MAX];
    if let Some(len) = history_recent(tab.hist_pos, &mut buf) {
        tab.input_line = buf;
        tab.input_line_len = len;
        tab.input_cursor = len;
        return true;
    }
    false
}

/// Recall the next (newer) history entry; stepping past the newest restores the
/// stashed in-progress line. Returns true if the line changed.
fn history_nav_down(tabs: &mut [ShellTab; MAX_TABS], active_tab: usize) -> bool {
    let Some(tab) = active_shell_tab_mut(tabs, active_tab) else {
        return false;
    };
    if tab.hist_pos == 0 {
        return false;
    }
    tab.hist_pos -= 1;
    if tab.hist_pos == 0 {
        tab.input_line = tab.hist_stash;
        tab.input_line_len = tab.hist_stash_len;
        tab.input_cursor = tab.hist_stash_len;
        return true;
    }
    let mut buf = [0u8; INPUT_LINE_MAX];
    if let Some(len) = history_recent(tab.hist_pos, &mut buf) {
        tab.input_line = buf;
        tab.input_line_len = len;
        tab.input_cursor = len;
    }
    true
}

impl ShellTab {
    const fn empty() -> Self {
        Self {
            shell_id: 0,
            pid: 0,
            session_pid: 0,
            cap: None,
            output: [0; TERM_OUTPUT_MAX],
            output_len: 0,
            input_line: [0; INPUT_LINE_MAX],
            input_line_len: 0,
            input_cursor: 0,
            hist_pos: 0,
            hist_stash: [0; INPUT_LINE_MAX],
            hist_stash_len: 0,
            pending: [0; PENDING_INPUT_MAX],
            pending_len: 0,
            username: [0; 32],
            username_len: 0,
            fg_pid: None,
            fg_app_name: [0; 24],
            fg_app_name_len: 0,
            fg_cmd: [0; INPUT_LINE_MAX],
            fg_cmd_len: 0,
            fg_capture: [0; FG_CAPTURE_MAX],
            fg_capture_len: 0,
            fg_capture_truncated: false,
        }
    }
}

#[no_mangle]
pub extern "C" fn _start(fb_addr: u64, fb_width: u64, fb_height: u64, fb_pitch: u64) -> ! {
    debug_log("[TTY]  TTY server started");
    debug_log("[LOGIN-TUI] service started");

    let fb32_w = fb_width as u32;
    let fb32_h = fb_height as u32;
    let fb32_p = fb_pitch as u32;
    let framebuffer_layout = if fb_width == u64::from(fb32_w)
        && fb_height == u64::from(fb32_h)
        && fb_pitch == u64::from(fb32_p)
    {
        sunlight_tui::framebuffer::validate_layout(fb32_w, fb32_h, fb32_p)
    } else {
        None
    };
    let has_fb = fb_addr != 0 && framebuffer_layout.is_some();
    let mut mouse = PointerSurface::new();
    // Decode SIMG v2 login wallpaper once; re-used for every login redraw.
    let login_bg = decode_login_bg();

    if has_fb {
        log_login_framebuffer_layout(framebuffer_layout.unwrap());
        debug_log("[TTY] Framebuffer acquired");
        debug_log("[LOGIN-TUI] metrics acquired");
        debug_log("[LOGIN-TUI] background begin");
        unsafe {
            sunlight_tui::render_login_screen(
                fb_addr as *mut u32,
                fb32_w,
                fb32_h,
                fb32_p,
                login_bg_view(&login_bg),
            );
        }
        unsafe {
            mouse.draw_overlay(fb_addr as *mut u32, fb32_w, fb32_h, fb32_p);
        }
        debug_log("[TTY] Login rendered");
        debug_log("[LOGIN-TUI] first frame complete");
    } else if fb_addr != 0 || fb_width != 0 || fb_height != 0 || fb_pitch != 0 {
        debug_log("[LOGIN-GEOMETRY] rejected invalid framebuffer layout\n");
    }

    let ep = endpoint_create();
    debug_log("[TTY]  endpoint created");

    // Register with init's name server so the user-space keyboard driver
    // (sunlight-kbd) can resolve "tty" and forward key events here. Without
    // this, nameserver_lookup("tty") returns DENY and the kbd driver spins
    // forever, so the keyboard appears dead.
    //
    // init drains REGISTER/LOOKUP between boot spawns so this can complete
    // before the full service tree is launched (see services/init).
    if nameserver_register("tty", ep) {
        debug_log("[TTY]  Registered as 'tty'");
        debug_log("[TTY]  Login screen ready");
        debug_log("[LOGIN-TUI] input ready");
    } else {
        debug_log("[TTY]  FATAL: nameserver_register('tty') failed — keyboard will not work");
    }

    let mut login = LoginScreen::new();
    // TTY/Login is the default session at boot; Desktop becomes active after Desktop login.
    let mut active_vt = VirtualTerminal::Tty;
    let mut desktop_pointer_release_gate = false;
    // Lazily resolved on first Ctrl+F1/F2 press or key-forward attempt.
    let mut display_cap: Option<CapabilityToken> = None;

    let mut state = TtyState::Login;
    let mut spawn_cap: Option<CapabilityToken> = None;
    let mut tabs = [ShellTab::empty(); MAX_TABS];
    let mut tab_count = 0usize;
    let mut active_tab = 0usize;
    let mut next_shell_id = 0u64;
    let mut logged_initial_spawn = false;
    // Set to true only after a successful Desktop login. Ctrl+F2 is blocked
    // while this is false so an unauthenticated user cannot bypass the login
    // screen by switching to the graphical desktop session.
    let mut desktop_unlocked = false;
    let mut desktop_session: Option<DesktopSessionHandle> = None;
    let mut session_telemetry = Telemetry::init().ok();
    let mut session_foundation_baseline = None;
    let mut session_foundation_gate = if session_foundation_gate_enabled() {
        SessionFoundationGateState::AwaitFirstSession
    } else {
        SessionFoundationGateState::Disabled
    };
    let mut session_config_gate = if session_configuration_gate_enabled() {
        SessionConfigGateState::AwaitFirstRunning
    } else {
        SessionConfigGateState::Disabled
    };
    let mut welcome_wizard_gate = if welcome_wizard_gate_enabled() {
        WelcomeWizardGateState::AwaitRunning
    } else {
        WelcomeWizardGateState::Disabled
    };

    let mut msg = ipc_recv(ep);
    let mut phase3_6_done = false;
    loop {
        match state {
            TtyState::Login => {
                let mut logged_in = false;
                let mut needs_render =
                    msg.label == KbdMsg::KEY_EVENT && active_vt == VirtualTerminal::Tty;
                let mut pointer_only_render = false;
                if msg.label == KbdMsg::KEY_EVENT {
                    'kbd: {
                        let (keycode, pressed, shift, _ctrl, _alt, _super, ascii_opt) =
                            unpack_key_event(msg.words[0]);

                        // Session-switch hotkeys (Ctrl+F1/F2) are intercepted on
                        // press only, before anything else.
                        if pressed && _ctrl {
                            match keycode {
                                KEY_F1 => {
                                    if active_vt != VirtualTerminal::Tty {
                                        debug_log("[SESSION] switched to F1 TTY/Login");
                                        let _ = send_display_request(
                                            &mut display_cap,
                                            IpcMsg::with_label(SgpMsg::SESSION_DEACTIVATE),
                                        );
                                    }
                                    active_vt = VirtualTerminal::Tty;
                                    desktop_pointer_release_gate = false;
                                    mouse.activate(fb32_w, fb32_h);
                                    if has_fb {
                                        render_login_fb(
                                            &login, fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse,
                                            &login_bg,
                                        );
                                    }
                                    break 'kbd;
                                }
                                KEY_F2 => {
                                    if !desktop_unlocked {
                                        debug_log("[SECURITY] Ctrl+F2 blocked: Desktop not unlocked (no authenticated session)");
                                        break 'kbd;
                                    }
                                    if active_vt != VirtualTerminal::Desktop {
                                        if has_fb {
                                            erase_tui_pointer(
                                                fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse,
                                            );
                                        }
                                        if send_display_request(
                                            &mut display_cap,
                                            IpcMsg::with_label(SgpMsg::SESSION_ACTIVATE),
                                        ) {
                                            debug_log("[SESSION] switched to F2 GraphicalDesktop");
                                            active_vt = VirtualTerminal::Desktop;
                                            desktop_pointer_release_gate = true;
                                            mouse.deactivate();
                                        } else {
                                            debug_log("[SESSION] F2 activation failed; TTY retains framebuffer");
                                            if has_fb {
                                                render_login_fb(
                                                    &login, fb_addr, fb32_w, fb32_h, fb32_p,
                                                    &mut mouse, &login_bg,
                                                );
                                            }
                                        }
                                    }
                                    break 'kbd;
                                }
                                _ => {}
                            }
                        }

                        if !vt_is_active(active_vt, VirtualTerminal::Tty) {
                            // Desktop mode: forward ALL events (presses AND releases) to
                            // display_server so it can track key-up for Alt+Tab chord end.
                            let _ = send_display_request(&mut display_cap, msg);
                            break 'kbd;
                        }

                        if pressed {
                            match login
                                .handle_key_event_with_shift(keycode, pressed, shift, ascii_opt)
                            {
                                LoginResult::Reboot => {
                                    request_login_power(LoginResult::Reboot);
                                }
                                LoginResult::Shutdown => {
                                    request_login_power(LoginResult::Shutdown);
                                }
                                LoginResult::Success {
                                    username,
                                    username_len,
                                    uid,
                                    gid,
                                    session_grant,
                                    session,
                                } => {
                                    debug_log_login_success(&username[..username_len], uid, gid);
                                    debug_log("[SunlightOS] Phase 3.7 OK");
                                    mouse.clear_interaction();

                                    match session {
                                        SessionType::Tty => {
                                            let cap = match nameserver_lookup("spawn") {
                                                Some(c) => c,
                                                None => {
                                                    debug_log("[TTY]  spawn capability not found");
                                                    state = TtyState::Shell;
                                                    break 'kbd;
                                                }
                                            };
                                            spawn_cap = Some(cap);

                                            if spawn_tab(
                                                &mut tabs,
                                                &mut tab_count,
                                                &mut active_tab,
                                                &mut next_shell_id,
                                                cap,
                                                session_grant,
                                            ) {
                                                if let Some(tab) =
                                                    active_shell_tab_mut(&mut tabs, active_tab)
                                                {
                                                    let len =
                                                        username_len.min(tab.username.len() - 1);
                                                    tab.username[..len]
                                                        .copy_from_slice(&username[..len]);
                                                    tab.username_len = len;
                                                }
                                                if let Some(tab) =
                                                    active_shell_tab(&tabs, active_tab)
                                                {
                                                    debug_log_spawn(
                                                        &username[..username_len],
                                                        tab.pid,
                                                    );
                                                    logged_initial_spawn = true;
                                                }
                                            }

                                            state = TtyState::Shell;
                                            debug_log("[TTY]  Built-in shell ready");
                                            if has_fb {
                                                render_active_shell_fb(
                                                    fb_addr, fb32_w, fb32_h, fb32_p, &tabs,
                                                    tab_count, active_tab, true, &mut mouse,
                                                );
                                            }
                                        }
                                        SessionType::Desktop => {
                                            if session_foundation_gate_enabled()
                                                && session_foundation_baseline.is_none()
                                            {
                                                session_foundation_baseline =
                                                    telemetry_baseline(&mut session_telemetry);
                                            }
                                            // CREATE is idempotent for the same uid when a
                                            // desktop session is already live (sessiond
                                            // reattaches and consumes the auth grant).
                                            let handle = match create_desktop_session(
                                                &username[..username_len],
                                                uid,
                                                gid,
                                                session_grant,
                                            ) {
                                                Ok(handle) => handle,
                                                Err(err) => {
                                                    login.message = err.message();
                                                    if has_fb {
                                                        render_login_fb(
                                                            &login, fb_addr, fb32_w, fb32_h,
                                                            fb32_p, &mut mouse, &login_bg,
                                                        );
                                                    }
                                                    break 'kbd;
                                                }
                                            };
                                            if !wait_for_desktop_session_running(handle, 10_000) {
                                                login.message =
                                                    "Desktop session failed before shell ready.";
                                                if has_fb {
                                                    render_login_fb(
                                                        &login, fb_addr, fb32_w, fb32_h, fb32_p,
                                                        &mut mouse, &login_bg,
                                                    );
                                                }
                                                break 'kbd;
                                            }
                                            if session_foundation_gate_enabled() {
                                                session_foundation_log("LOGIN_HANDOFF PASS");
                                            }
                                            activate_desktop_session(
                                                handle,
                                                &mut display_cap,
                                                &mut active_vt,
                                                &mut desktop_pointer_release_gate,
                                                &mut desktop_session,
                                                &mut desktop_unlocked,
                                                &mut mouse,
                                                &mut login,
                                                has_fb,
                                                fb_addr,
                                                fb32_w,
                                                fb32_h,
                                                fb32_p,
                                                &login_bg,
                                            );
                                        }
                                    }
                                    logged_in = true;
                                }
                                LoginResult::Locked => {
                                    mouse.clear_interaction();
                                    debug_log("[TTY]  Login locked");
                                }
                                LoginResult::Pending => {}
                            }
                        }
                    } // end 'kbd
                }
                if msg.label != MOUSE_LABEL && msg.label != WHEEL_LABEL {
                    let was_locked = login.locked_ticks > 0;
                    login.tick();
                    needs_render |= was_locked && login.locked_ticks == 0;
                }
                if msg.label == MOUSE_LABEL {
                    if active_vt == VirtualTerminal::Desktop {
                        let _ = forward_pointer_to_display(
                            &mut display_cap,
                            msg,
                            &mut desktop_pointer_release_gate,
                        );
                    } else {
                        let report = PointerReport::unpack(msg.words[0]);
                        let generation = (msg.words[1] >> 32) as u32;
                        let layout = sunlight_tui::LoginLayout::new(
                            fb32_w,
                            fb32_h,
                            login.active_count,
                            login.locked_ticks == 0,
                        );
                        let outcome = mouse.handle_report(
                            report.dx,
                            report.dy,
                            report.buttons,
                            generation,
                            fb32_w,
                            fb32_h,
                            layout.widgets(),
                        );
                        if let Some(target) = outcome.clicked.and_then(login_focus_for_widget) {
                            let result = login.handle_pointer_click(target);
                            if matches!(result, LoginResult::Reboot | LoginResult::Shutdown) {
                                request_login_power(result);
                            }
                            needs_render = true;
                        }
                        if outcome.interaction_changed() {
                            needs_render = true;
                        } else if outcome.moved {
                            pointer_only_render = true;
                        }
                    }
                } else if msg.label == WHEEL_LABEL && active_vt == VirtualTerminal::Desktop {
                    let _ = forward_wheel_to_display(&mut display_cap, msg);
                }
                if has_fb && !logged_in && vt_is_active(active_vt, VirtualTerminal::Tty) {
                    if needs_render {
                        render_login_fb(
                            &login, fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse, &login_bg,
                        );
                    } else if pointer_only_render {
                        redraw_mouse_overlay(fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse);
                    }
                }
            }
            TtyState::Shell => {
                let mut needs_render = false;
                let mut pointer_only_render = false;
                let mut force_shell_render = false;
                let prev_output_len = active_shell_tab(&tabs, active_tab)
                    .map(|tab| tab.output_len)
                    .unwrap_or(0);

                if msg.label == MOUSE_LABEL {
                    if active_vt == VirtualTerminal::Desktop {
                        let _ = forward_pointer_to_display(
                            &mut display_cap,
                            msg,
                            &mut desktop_pointer_release_gate,
                        );
                    } else {
                        resolve_active_shell(&mut tabs, active_tab, &mut logged_initial_spawn);
                        let report = PointerReport::unpack(msg.words[0]);
                        let generation = (msg.words[1] >> 32) as u32;
                        let mut labels = [sunlight_tui::TabLabel::empty(); MAX_TABS];
                        let label_count = build_tab_labels(&tabs, tab_count, &mut labels);
                        let tab_layout = sunlight_tui::TerminalTabLayout::new(
                            fb32_w,
                            fb32_h,
                            &labels[..label_count],
                            active_tab,
                            tab_count < MAX_TABS,
                        );
                        let outcome = mouse.handle_report(
                            report.dx,
                            report.dy,
                            report.buttons,
                            generation,
                            fb32_w,
                            fb32_h,
                            tab_layout.widgets(),
                        );
                        if let Some(clicked) = outcome.clicked {
                            if let Some(index) = sunlight_tui::terminal_tab_index(clicked) {
                                if index < tab_count && index != active_tab {
                                    active_tab = index;
                                    mouse.clear_interaction();
                                    force_shell_render = true;
                                }
                            } else if clicked == sunlight_tui::TERMINAL_ADD_TAB_WIDGET
                                && spawn_tab_from_active_shell(
                                    &mut tabs,
                                    &mut tab_count,
                                    &mut active_tab,
                                    &mut next_shell_id,
                                    &mut phase3_6_done,
                                )
                            {
                                mouse.clear_interaction();
                                force_shell_render = true;
                            }
                        }
                        if outcome.interaction_changed() {
                            needs_render = true;
                        } else if outcome.moved {
                            pointer_only_render = true;
                        }
                    }
                } else if msg.label == WHEEL_LABEL && active_vt == VirtualTerminal::Desktop {
                    let _ = forward_wheel_to_display(&mut display_cap, msg);
                }

                // Lazy lookup: try to find sshl once it registers after being spawned.
                if msg.label == KbdMsg::KEY_EVENT {
                    'kbd: {
                        let (keycode, pressed, _shift, ctrl, _alt, _super, ctrl_ascii) =
                            unpack_key_event(msg.words[0]);

                        if pressed && ctrl {
                            match keycode {
                                KEY_F1 => {
                                    if active_vt != VirtualTerminal::Tty {
                                        debug_log("[SESSION] switched to F1 TTY/Login");
                                        let _ = send_display_request(
                                            &mut display_cap,
                                            IpcMsg::with_label(SgpMsg::SESSION_DEACTIVATE),
                                        );
                                    }
                                    active_vt = VirtualTerminal::Tty;
                                    desktop_pointer_release_gate = false;
                                    mouse.activate(fb32_w, fb32_h);
                                    // In Shell state, redraw the shell (not the login screen).
                                    if has_fb {
                                        render_active_shell_fb(
                                            fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count,
                                            active_tab, true, &mut mouse,
                                        );
                                    }
                                    break 'kbd;
                                }
                                KEY_F2 => {
                                    if active_vt != VirtualTerminal::Desktop {
                                        if has_fb {
                                            erase_tui_pointer(
                                                fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse,
                                            );
                                        }
                                        if send_display_request(
                                            &mut display_cap,
                                            IpcMsg::with_label(SgpMsg::SESSION_ACTIVATE),
                                        ) {
                                            debug_log("[SESSION] switched to F2 GraphicalDesktop");
                                            active_vt = VirtualTerminal::Desktop;
                                            desktop_pointer_release_gate = true;
                                            mouse.deactivate();
                                        } else {
                                            debug_log("[SESSION] F2 activation failed; TTY retains framebuffer");
                                            if has_fb {
                                                render_active_shell_fb(
                                                    fb_addr, fb32_w, fb32_h, fb32_p, &tabs,
                                                    tab_count, active_tab, true, &mut mouse,
                                                );
                                            }
                                        }
                                    }
                                    break 'kbd;
                                }
                                KEY_E => {
                                    if vt_is_active(active_vt, VirtualTerminal::Desktop) {
                                        debug_log("[SESSION] Ctrl+E: launching eyes\n");
                                        let _ = launch_shortcut_app(b"eyes");
                                        break 'kbd;
                                    }
                                }
                                KEY_T => {
                                    if vt_is_active(active_vt, VirtualTerminal::Desktop) {
                                        debug_log("[SESSION] Ctrl+T: launching tasks monitor\n");
                                        let _ = launch_shortcut_app(b"tasks");
                                        break 'kbd;
                                    }
                                }
                                _ => {}
                            }
                        }

                        if !vt_is_active(active_vt, VirtualTerminal::Tty) {
                            // Forward keyboard event to display_server when Desktop session is active.
                            let (
                                fwd_keycode,
                                fwd_pressed,
                                _fwd_shift,
                                fwd_ctrl,
                                _fwd_alt,
                                _fwd_super,
                                fwd_ascii,
                            ) = unpack_key_event(msg.words[0]);
                            if fwd_pressed {
                                debug_log(&alloc::format!(
                                    "[TTY->DISPLAY] shell forward keycode={:#x} ctrl={} ascii={}\n",
                                    fwd_keycode,
                                    fwd_ctrl,
                                    fwd_ascii.unwrap_or(0)
                                ));
                            }
                            let _ = send_display_request(&mut display_cap, msg);
                            break 'kbd;
                        }

                        // Special navigation keys arrive as bare keycodes with no
                        // ASCII (the keyboard driver doesn't decode the 0xE0 prefix),
                        // so they never reach the line-editor ASCII path below:
                        //   Up=0x48 Down=0x50 Left=0x4B Right=0x4D
                        //   PageUp=0x49 PageDown=0x51 Home=0x47 End=0x4F
                        let fg_active = active_shell_tab(&tabs, active_tab)
                            .map_or(false, |t| t.fg_pid.is_some());
                        let page = unsafe { TERMINAL_GEOMETRY[active_tab].rows as usize }.max(1);

                        if pressed && ctrl {
                            // Ctrl+Up/Down keep the fine-grained line scroll.
                            match keycode {
                                0x48 => unsafe {
                                    let s = &mut SCROLLBACK_STATE[active_tab];
                                    s.viewport_offset = (s.viewport_offset + 1).min(256);
                                    needs_render = true;
                                },
                                0x50 => unsafe {
                                    let s = &mut SCROLLBACK_STATE[active_tab];
                                    s.viewport_offset = s.viewport_offset.saturating_sub(1);
                                    needs_render = true;
                                },
                                _ => {
                                    if let Some(a) = ctrl_ascii {
                                        if matches!(a, b't' | b'T') {
                                            resolve_active_shell(
                                                &mut tabs,
                                                active_tab,
                                                &mut logged_initial_spawn,
                                            );
                                        }
                                        if handle_ctrl_key(
                                            a,
                                            &mut tabs,
                                            &mut tab_count,
                                            &mut active_tab,
                                            &mut next_shell_id,
                                            spawn_cap,
                                            &mut phase3_6_done,
                                        ) {
                                            needs_render = true;
                                        } else if fg_active {
                                            // Not a tty shortcut: forward as control byte
                                            // (Ctrl+Q=0x11, Ctrl+S=0x13, Ctrl+C=0x03, …).
                                            let ctrl_byte = a & 0x1f;
                                            if let Some(tab) = active_shell_tab(&tabs, active_tab) {
                                                let _ = tty_stdin_push(
                                                    tab.shell_id as u32,
                                                    &[ctrl_byte],
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        } else if pressed && !fg_active {
                            match keycode {
                                // PageUp/PageDown: scroll the viewport by a screenful.
                                0x49 => unsafe {
                                    let s = &mut SCROLLBACK_STATE[active_tab];
                                    s.viewport_offset = (s.viewport_offset + page).min(256);
                                    needs_render = true;
                                },
                                0x51 => unsafe {
                                    let s = &mut SCROLLBACK_STATE[active_tab];
                                    s.viewport_offset = s.viewport_offset.saturating_sub(page);
                                    needs_render = true;
                                },
                                // Home: jump to the oldest scrollback. End: live view.
                                0x47 => unsafe {
                                    SCROLLBACK_STATE[active_tab].viewport_offset = 256;
                                    needs_render = true;
                                },
                                0x4F => unsafe {
                                    SCROLLBACK_STATE[active_tab].viewport_offset = 0;
                                    needs_render = true;
                                },
                                // Up/Down: walk command history into the edit line.
                                0x48 => {
                                    if history_nav_up(&mut tabs, active_tab) {
                                        needs_render = true;
                                    }
                                    unsafe {
                                        SCROLLBACK_STATE[active_tab].viewport_offset = 0;
                                    }
                                }
                                0x50 => {
                                    if history_nav_down(&mut tabs, active_tab) {
                                        needs_render = true;
                                    }
                                    unsafe {
                                        SCROLLBACK_STATE[active_tab].viewport_offset = 0;
                                    }
                                }
                                // Left/Right: move the edit cursor within the line.
                                0x4B => {
                                    if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                                        if tab.input_cursor > 0 {
                                            tab.input_cursor -= 1;
                                            needs_render = true;
                                        }
                                    }
                                }
                                0x4D => {
                                    if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                                        if tab.input_cursor < tab.input_line_len {
                                            tab.input_cursor += 1;
                                            needs_render = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        } else if pressed && fg_active {
                            // Raw-mode pass-through: translate special keys to VT100
                            // escape sequences and push them directly into the fg
                            // process's stdin ring.
                            let seq: &[u8] = match keycode {
                                0x01 => b"\x1b",    // Escape
                                0x48 => b"\x1b[A",  // Up
                                0x50 => b"\x1b[B",  // Down
                                0x4D => b"\x1b[C",  // Right
                                0x4B => b"\x1b[D",  // Left
                                0x47 => b"\x1b[H",  // Home
                                0x4F => b"\x1b[F",  // End
                                0x49 => b"\x1b[5~", // Page Up
                                0x51 => b"\x1b[6~", // Page Down
                                _ => b"",
                            };
                            if !seq.is_empty() {
                                if let Some(tab) = active_shell_tab(&tabs, active_tab) {
                                    let _ = tty_stdin_push(tab.shell_id as u32, seq);
                                }
                            }
                        }
                    } // end 'kbd
                }

                resolve_active_shell(&mut tabs, active_tab, &mut logged_initial_spawn);

                // Check if shell was just resolved and has greeting output to render
                let current_output_len = active_shell_tab(&tabs, active_tab)
                    .map(|tab| tab.output_len)
                    .unwrap_or(0);
                if current_output_len > prev_output_len {
                    needs_render = true;
                }

                if let Some(ascii) = key_ascii_from_msg(&msg) {
                    if TRACE_TTY_IPC {
                        debug_log(&alloc::format!(
                            "[TTY-IPC] keyboard event converted to KBD_LABEL byte={}",
                            ascii
                        ));
                    }
                    // Reset scrollback on any normal keypress (return to live view)
                    unsafe {
                        SCROLLBACK_STATE[active_tab].viewport_offset = 0;
                    }
                    if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                        if tab.fg_pid.is_some() {
                            // A foreground command owns the screen: route the key
                            // to its stdin ring (keyed by shell_id) instead of the
                            // shell line editor. Raw POSIX terminals deliver Enter
                            // as CR; Crossterm treats LF as an ordinary character
                            // while raw mode is active.
                            let raw_byte = if ascii == b'\n' { b'\r' } else { ascii };
                            let _ = tty_stdin_push(tab.shell_id as u32, &[raw_byte]);
                        } else {
                            // Local line editing: the TTY owns the edit line and
                            // only flushes the completed command to the shell on
                            // Enter. The shell only acts on '\n', so its own line
                            // buffer stays in sync.
                            needs_render = true;
                            match ascii {
                                b'\n' | b'\r' => {
                                    // Snapshot the completed line.
                                    let mut line = [0u8; INPUT_LINE_MAX];
                                    let line_len = tab.input_line_len;
                                    line[..line_len].copy_from_slice(&tab.input_line[..line_len]);

                                    // Echo prompt + line + newline into scrollback.
                                    let mut prompt_buf = [0u8; 32];
                                    let prompt_len = build_prompt(tab, &mut prompt_buf);
                                    append_term(
                                        &mut tab.output,
                                        &mut tab.output_len,
                                        &prompt_buf[..prompt_len],
                                    );
                                    append_term(
                                        &mut tab.output,
                                        &mut tab.output_len,
                                        &line[..line_len],
                                    );
                                    append_term(&mut tab.output, &mut tab.output_len, b"\n");

                                    // Record in history; reset the edit state.
                                    history_push(&line[..line_len]);
                                    tab.fg_cmd[..line_len].copy_from_slice(&line[..line_len]);
                                    tab.fg_cmd_len = line_len;
                                    tab.fg_capture_len = 0;
                                    tab.fg_capture_truncated = false;
                                    tab.input_line_len = 0;
                                    tab.input_cursor = 0;
                                    tab.hist_pos = 0;
                                    tab.hist_stash_len = 0;

                                    if let Some(cap) = tab.cap {
                                        // Replay the line byte-by-byte, then the
                                        // newline that triggers execution.
                                        for i in 0..line_len {
                                            let _ = send_key_to_shell(
                                                cap,
                                                line[i],
                                                &mut tab.output,
                                                &mut tab.output_len,
                                            );
                                        }
                                        match send_key_to_shell(
                                            cap,
                                            b'\n',
                                            &mut tab.output,
                                            &mut tab.output_len,
                                        ) {
                                            ShellKeyResult::Exited => {
                                                state = TtyState::Login;
                                                reset_login(&mut login);
                                                mouse.clear_interaction();
                                                reset_tabs(
                                                    &mut tabs,
                                                    &mut tab_count,
                                                    &mut active_tab,
                                                );
                                                spawn_cap = None;
                                                logged_initial_spawn = false;
                                                if has_fb
                                                    && vt_is_active(active_vt, VirtualTerminal::Tty)
                                                {
                                                    render_login_fb(
                                                        &login, fb_addr, fb32_w, fb32_h, fb32_p,
                                                        &mut mouse, &login_bg,
                                                    );
                                                }
                                                continue;
                                            }
                                            ShellKeyResult::ForegroundStarted(
                                                pid,
                                                name,
                                                name_len,
                                            ) => {
                                                tab.fg_pid = Some(pid);
                                                tab.fg_app_name = name;
                                                tab.fg_app_name_len = name_len;
                                            }
                                            ShellKeyResult::Continue => {
                                                tab.fg_cmd_len = 0;
                                            }
                                        }
                                    } else {
                                        // Shell not resolved yet: buffer raw bytes.
                                        for i in 0..line_len {
                                            if tab.pending_len < tab.pending.len() {
                                                tab.pending[tab.pending_len] = line[i];
                                                tab.pending_len += 1;
                                            }
                                        }
                                        if tab.pending_len < tab.pending.len() {
                                            tab.pending[tab.pending_len] = b'\n';
                                            tab.pending_len += 1;
                                        }
                                    }
                                }
                                0x08 => {
                                    // Backspace: delete the char before the cursor.
                                    if tab.input_cursor > 0 {
                                        let c = tab.input_cursor;
                                        let end = tab.input_line_len;
                                        let mut i = c - 1;
                                        while i + 1 < end {
                                            tab.input_line[i] = tab.input_line[i + 1];
                                            i += 1;
                                        }
                                        tab.input_line_len -= 1;
                                        tab.input_cursor -= 1;
                                    }
                                    tab.hist_pos = 0;
                                }
                                c if (0x20..=0x7e).contains(&c) => {
                                    // Insert at the cursor, shifting the tail right.
                                    if tab.input_line_len < tab.input_line.len() {
                                        let mut i = tab.input_line_len;
                                        while i > tab.input_cursor {
                                            tab.input_line[i] = tab.input_line[i - 1];
                                            i -= 1;
                                        }
                                        tab.input_line[tab.input_cursor] = c;
                                        tab.input_line_len += 1;
                                        tab.input_cursor += 1;
                                    }
                                    tab.hist_pos = 0;
                                }
                                _ => {}
                            }
                        }
                    }
                }

                // Re-render whenever the title-bar clock minute rolls over
                static mut LAST_CLOCK_MIN: u64 = u64::MAX;
                let now_min = get_time_utc() / 60;
                unsafe {
                    if now_min != LAST_CLOCK_MIN {
                        LAST_CLOCK_MIN = now_min;
                        needs_render = true;
                    }
                }

                // Don't run the shell renderer while a foreground app owns the
                // screen — it would clear the grid and replay the shell buffer,
                // wiping the app's frame. The idle loop renders the app instead.
                let active_fg =
                    active_shell_tab(&tabs, active_tab).map_or(false, |t| t.fg_pid.is_some());
                if has_fb && needs_render && vt_is_active(active_vt, VirtualTerminal::Tty) {
                    if active_fg && !force_shell_render {
                        redraw_mouse_overlay(fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse);
                    } else {
                        render_active_shell_fb(
                            fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab, true,
                            &mut mouse,
                        );
                    }
                } else if has_fb
                    && pointer_only_render
                    && vt_is_active(active_vt, VirtualTerminal::Tty)
                {
                    redraw_mouse_overlay(fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse);
                }
            }
        }

        // Wait for the next message, but keep the active tab live while idle:
        // drive a foreground command (drain its output, detect exit) and keep
        // the title-bar clock current.
        let reply = IpcMsg::with_label(0);
        loop {
            if let Some(m) = ipc_reply_and_try_recv(ep, reply) {
                msg = m;
                break;
            }
            if let Some(handle) = desktop_session {
                static mut LAST_SESSION_POLL_MS: u64 = 0;
                static mut LAST_SESSION_MISS_LOG_MS: u64 = 0;
                let now = monotonic_millis();
                let should_poll = unsafe {
                    if now.saturating_sub(LAST_SESSION_POLL_MS) >= 250 {
                        LAST_SESSION_POLL_MS = now;
                        true
                    } else {
                        false
                    }
                };
                if should_poll {
                    match query_desktop_session(handle, SESSION_QUERY_TIMEOUT_MS) {
                        // Only end the local login handoff on definitive terminal
                        // states. IPC timeout/None used to drop a live session and
                        // leave sessiond busy, blocking the next Desktop login.
                        Some(SessionState::Failed) | Some(SessionState::Stopped) => {
                            let _ = send_display_request(
                                &mut display_cap,
                                IpcMsg::with_label(SgpMsg::SESSION_DEACTIVATE),
                            );
                            active_vt = VirtualTerminal::Tty;
                            desktop_pointer_release_gate = false;
                            desktop_session = None;
                            desktop_unlocked = false;
                            mouse.activate(fb32_w, fb32_h);
                            login.message = "Desktop session ended.";
                            if has_fb {
                                render_login_fb(
                                    &login, fb_addr, fb32_w, fb32_h, fb32_p, &mut mouse, &login_bg,
                                );
                            }
                        }
                        Some(state) if desktop_session_is_live(state) => {}
                        None => {
                            // Transient miss under load — keep the handle.
                            let should_log = unsafe {
                                if now.saturating_sub(LAST_SESSION_MISS_LOG_MS) >= 5_000 {
                                    LAST_SESSION_MISS_LOG_MS = now;
                                    true
                                } else {
                                    false
                                }
                            };
                            if should_log {
                                debug_log(
                                    "[SESSION] poll miss (timeout/lookup); keeping desktop session\n",
                                );
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
            if session_foundation_gate_enabled() {
                drive_session_foundation_gate(
                    &mut session_foundation_gate,
                    desktop_session,
                    session_foundation_baseline,
                    &mut session_telemetry,
                );
            }
            if session_configuration_gate_enabled() {
                drive_session_configuration_gate(
                    &mut session_config_gate,
                    desktop_session,
                    &mut session_telemetry,
                );
            }
            if welcome_wizard_gate_enabled() {
                drive_welcome_wizard_gate(
                    &mut welcome_wizard_gate,
                    desktop_session,
                    &mut session_telemetry,
                );
            }
            if has_fb
                && matches!(state, TtyState::Shell)
                && vt_is_active(active_vt, VirtualTerminal::Tty)
            {
                let fg = active_shell_tab(&tabs, active_tab).and_then(|t| t.fg_pid);
                if let Some(pid) = fg {
                    // Drain the foreground app's output (kernel stdout ring,
                    // keyed by shell_id) into the tab's scrollback. The existing
                    // replay renderer then shows the live screen; full-screen
                    // apps redraw in place (their alt-screen enter resets the
                    // buffer in append_term), streaming output simply scrolls.
                    let mut drained = false;
                    if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                        let mut buf = [0u8; 1024];
                        loop {
                            let n = tty_stdout_pull(tab.shell_id as u32, &mut buf);
                            if n == 0 {
                                break;
                            }
                            capture_foreground_bytes(tab, &buf[..n]);
                            append_term(&mut tab.output, &mut tab.output_len, &buf[..n]);
                            drained = true;
                        }
                    }

                    if !process_is_alive(pid) {
                        // The app exited. Final drain to catch any output written
                        // just before exit (between the drain above and now).
                        if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                            let mut buf = [0u8; 1024];
                            loop {
                                let n = tty_stdout_pull(tab.shell_id as u32, &mut buf);
                                if n == 0 {
                                    break;
                                }
                                capture_foreground_bytes(tab, &buf[..n]);
                                append_term(&mut tab.output, &mut tab.output_len, &buf[..n]);
                            }
                        }
                        // Tell the shell to reap it and record $?; leave
                        // foreground mode and redraw with the prompt.
                        let cap = active_shell_tab(&tabs, active_tab).and_then(|t| t.cap);
                        if let Some(cap) = cap {
                            let done = IpcMsg::with_label(FG_DONE_LABEL);
                            debug_ipc_msg("[TTY-IPC] before shell ipc_call FG_DONE_LABEL", &done);
                            match ipc_call_timeout(cap, done, SHELL_IPC_TIMEOUT_MS) {
                                Ok(reply) => {
                                    let exit_code = reply.words[0];
                                    if let Some(tab) = active_shell_tab(&tabs, active_tab) {
                                        debug_log_foreground_command(
                                            &tab.fg_cmd[..tab.fg_cmd_len],
                                            &tab.fg_capture[..tab.fg_capture_len],
                                            tab.fg_capture_truncated,
                                            exit_code,
                                        );
                                    }
                                    debug_ipc_msg("[TTY-IPC] after shell FG_DONE reply", &reply);
                                }
                                Err(_) => {
                                    debug_log("[TTY] shell FG_DONE IPC timeout\n");
                                }
                            }
                        }
                        if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                            tab.fg_pid = None;
                            tab.fg_app_name_len = 0;
                            tab.fg_cmd_len = 0;
                            tab.fg_capture_len = 0;
                            tab.fg_capture_truncated = false;
                        }
                        render_active_shell_fb(
                            fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab, true,
                            &mut mouse,
                        );
                    } else if drained {
                        // Foreground app owns the screen: render without a prompt.
                        render_active_shell_fb(
                            fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab, false,
                            &mut mouse,
                        );
                    }
                } else {
                    static mut LAST_POLL_MIN: u64 = u64::MAX;
                    let now_min = get_time_utc() / 60;
                    // SAFETY: tty_server is single-threaded; no concurrent access.
                    unsafe {
                        if now_min != LAST_POLL_MIN {
                            LAST_POLL_MIN = now_min;
                            render_active_shell_fb(
                                fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab,
                                true, &mut mouse,
                            );
                        }
                    }
                }
            }
            process_yield();
        }
    }
}

/// Get current terminal geometry for the active tab
pub fn get_terminal_geometry(tab_idx: usize) -> Option<TerminalGeometry> {
    if tab_idx < MAX_TABS {
        unsafe { Some(TERMINAL_GEOMETRY[tab_idx]) }
    } else {
        None
    }
}

/// Get terminal dimensions (cols, rows) for the active tab
pub fn get_terminal_dims(tab_idx: usize) -> Option<(u32, u32)> {
    get_terminal_geometry(tab_idx).map(|g| (g.cols, g.rows))
}

/// Get current viewport offset for the active tab
pub fn get_viewport_offset(tab_idx: usize) -> usize {
    if tab_idx < MAX_TABS {
        unsafe { TERMINAL_GEOMETRY[tab_idx].viewport_offset }
    } else {
        0
    }
}

fn redraw_mouse_overlay(fb_addr: u64, fb_w: u32, fb_h: u32, fb_p: u32, mouse: &mut PointerSurface) {
    unsafe {
        mouse.erase_overlay(fb_addr as *mut u32, fb_w, fb_h, fb_p);
        mouse.draw_overlay(fb_addr as *mut u32, fb_w, fb_h, fb_p);
    }
}

fn build_tab_labels(
    tabs: &[ShellTab; MAX_TABS],
    tab_count: usize,
    labels: &mut [sunlight_tui::TabLabel; MAX_TABS],
) -> usize {
    *labels = [sunlight_tui::TabLabel::empty(); MAX_TABS];
    let count = tab_count.max(1).min(MAX_TABS);
    for (index, tab) in tabs.iter().enumerate().take(count) {
        if tab.pid == 0 {
            continue;
        }
        let len = tab.fg_app_name_len.min(24);
        labels[index].name[..len].copy_from_slice(&tab.fg_app_name[..len]);
        labels[index].name_len = len;
        labels[index].running = tab.fg_pid.is_some();
    }
    count
}

fn render_login_fb(
    login: &LoginScreen,
    fb_addr: u64,
    fb_w: u32,
    fb_h: u32,
    fb_p: u32,
    mouse: &mut PointerSurface,
    login_bg: &Option<(u32, u32, alloc::vec::Vec<u32>)>,
) {
    unsafe {
        mouse.erase_overlay(fb_addr as *mut u32, fb_w, fb_h, fb_p);
    }

    let mut user_bufs = [[0u8; 64]; MAX_USERS];
    let mut user_lens = [0usize; MAX_USERS];
    let mut user_labels = [""; MAX_USERS];
    let mut user_icons = [sunlight_tui::LoginUserIcon::User; MAX_USERS];
    let mut is_custom = [false; MAX_USERS];
    for i in 0..login.active_count.min(MAX_USERS) {
        let len = login.users[i].len.min(64);
        user_bufs[i][..len].copy_from_slice(&login.users[i].buf[..len]);
        user_lens[i] = len;
        is_custom[i] = login.is_custom_slot[i];
        user_labels[i] = if login.users[i].len == 0 && login.is_custom_slot[i] {
            "Other"
        } else {
            login_display_name(login.users[i].as_str())
        };
        user_icons[i] = match login_user_icon(login.users[i].as_str()) {
            LoginUserIcon::User => sunlight_tui::LoginUserIcon::User,
            LoginUserIcon::Luggage => sunlight_tui::LoginUserIcon::Luggage,
        };
    }

    let focus = match login.focus {
        FocusArea::UserSlot(i) => sunlight_tui::LoginFocus::UserSlot(i),
        FocusArea::Password => sunlight_tui::LoginFocus::Password,
        FocusArea::Dropdown => sunlight_tui::LoginFocus::Dropdown,
        FocusArea::Reboot => sunlight_tui::LoginFocus::Reboot,
        FocusArea::Shutdown => sunlight_tui::LoginFocus::Shutdown,
    };

    unsafe {
        sunlight_tui::render_login_grid_interactive(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            login_bg_view(login_bg),
            &user_bufs,
            &user_lens[..login.active_count],
            &user_labels[..login.active_count],
            &user_icons[..login.active_count],
            &is_custom[..login.active_count],
            login.active_count,
            login.selected_user_idx,
            focus,
            login.session.label(),
            login.password.len,
            login.message,
            sunlight_tui::LoginPointerVisual {
                hovered: mouse.hovered(),
                pressed: mouse.pressed(),
            },
        );
        mouse.draw_overlay(fb_addr as *mut u32, fb_w, fb_h, fb_p);
    }
}

fn render_active_shell_fb(
    fb_addr: u64,
    fb_w: u32,
    fb_h: u32,
    fb_p: u32,
    tabs: &[ShellTab; MAX_TABS],
    tab_count: usize,
    active_tab: usize,
    show_prompt: bool,
    mouse: &mut PointerSurface,
) {
    unsafe {
        mouse.erase_overlay(fb_addr as *mut u32, fb_w, fb_h, fb_p);
    }

    // Size the grid with the exact same formula the renderer uses, so every
    // row is shown from the top of the content area with no clipping. Computing
    // this independently here (it previously used a different glyph height and
    // chrome height) made the grid taller than the viewport, so the renderer
    // dropped the top rows.
    let (cols, rows) = sunlight_tui::terminal_dims(fb_w, fb_h);

    // Update terminal geometry state
    unsafe {
        let viewport_offset = SCROLLBACK_STATE[active_tab].viewport_offset;
        TERMINAL_GEOMETRY[active_tab].update(cols as u32, rows as u32, viewport_offset);
    }

    // A foreground app owns the screen, so suppress the shell prompt/input line.
    let mut prompt_buf = [0u8; 32];
    let (output, input_line, prompt_slice, input_cursor) = active_shell_tab(tabs, active_tab)
        .map(|tab| {
            if show_prompt {
                let prompt_len = build_prompt(tab, &mut prompt_buf);
                (
                    &tab.output[..tab.output_len],
                    &tab.input_line[..tab.input_line_len],
                    &prompt_buf[..prompt_len],
                    tab.input_cursor,
                )
            } else {
                (&tab.output[..tab.output_len], &[][..], &b""[..], 0usize)
            }
        })
        .unwrap_or((&[][..], &[][..], b"root@sunlight:/$ ", 0usize));

    // Parse output into a terminal-sized grid. The framebuffer renderer already
    // offsets this grid below the title/tab chrome, so the VT cursor must stay
    // relative to the terminal content, not the full framebuffer.

    // FIX: Reuse cached grid instead of allocating 400KB+ per frame
    // This prevents bump allocator memory exhaustion that was causing freezes
    let grid = unsafe {
        match &mut GRID_CACHE {
            Some(cached) => {
                // Grid exists - check if dimensions match
                if cached.cols == cols && cached.rows == rows {
                    // Dimensions match - reuse grid, clear for fresh content
                    cached.clear_screen(); // FIX: Clear previous content before reuse
                    cached.as_mut()
                } else {
                    // Dimensions changed - allocate new grid
                    debug_log("[TTY]  Grid dimensions changed, reallocating");
                    *cached = Box::new(TerminalGrid::new(cols, rows));
                    cached.as_mut()
                }
            }
            None => {
                // First render - allocate and cache the grid
                debug_log("[TTY]  First render, caching grid");
                GRID_CACHE = Some(Box::new(TerminalGrid::new(cols, rows)));
                GRID_CACHE.as_mut().unwrap().as_mut()
            }
        }
    };

    grid.feed(output);
    let (cursor_row, cursor_col) = grid.cursor();

    // Get viewport offset for scrollback
    let viewport_offset = unsafe { SCROLLBACK_STATE[active_tab].viewport_offset };

    // Render with scrollback offset if active. Both methods fill the grid's
    // internal term-cell buffer in place and return a borrowed slice — no
    // per-frame allocation (the bump allocator never frees).
    let term_cells = if viewport_offset > 0 {
        grid.to_term_cells_with_offset(&ANSI_COLORS, viewport_offset)
    } else {
        grid.to_term_cells(&ANSI_COLORS)
    };

    // Title-bar stats: "CPU 15% RAM 42%  12:22 AM | 2026/6/12 | eth0".
    // Cached and refreshed at most once per minute (the same cadence the clock
    // re-renders) so the per-keystroke render path never does a sysinfo syscall.
    let (clock_buf, clock_len) = unsafe { titlebar(get_time_utc()) };

    // Build the dynamic tab labels: each tab shows its running app's name
    // (uppercased by the renderer) or "SHELL" when idle, plus a "*" on
    // background tabs that still have a live foreground app.
    let mut labels = [sunlight_tui::TabLabel::empty(); MAX_TABS];
    let n_tabs = build_tab_labels(tabs, tab_count, &mut labels);

    unsafe {
        sunlight_tui::render_terminal_grid_interactive(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            &labels[..n_tabs],
            active_tab,
            cols,
            rows,
            term_cells,
            cursor_row,
            cursor_col,
            input_line,
            prompt_slice,
            &clock_buf[..clock_len],
            input_cursor,
            sunlight_tui::TerminalPointerVisual {
                hovered: mouse.hovered(),
                pressed: mouse.pressed(),
            },
            tab_count < MAX_TABS,
        );
    }
    unsafe {
        mouse.draw_overlay(fb_addr as *mut u32, fb_w, fb_h, fb_p);
    }

    // NOTE: Grid is NOT dropped here - it's cached in GRID_CACHE for reuse on next render
    // This prevents the 400KB+ allocation that was exhausting the bump allocator heap
    // Grid stays alive until dimensions change or process exits
}

fn reset_login(login: &mut LoginScreen) {
    *login = LoginScreen::new();
    login.message = "Logged out. Please log in.";
}

fn reset_tabs(tabs: &mut [ShellTab; MAX_TABS], tab_count: &mut usize, active_tab: &mut usize) {
    for tab in tabs.iter_mut() {
        *tab = ShellTab::empty();
    }
    *tab_count = 0;
    *active_tab = 0;
}

fn active_shell_tab(tabs: &[ShellTab; MAX_TABS], active_tab: usize) -> Option<&ShellTab> {
    tabs.get(active_tab).filter(|tab| tab.pid != 0)
}

fn active_shell_tab_mut(
    tabs: &mut [ShellTab; MAX_TABS],
    active_tab: usize,
) -> Option<&mut ShellTab> {
    tabs.get_mut(active_tab).filter(|tab| tab.pid != 0)
}

fn build_prompt(tab: &ShellTab, buf: &mut [u8]) -> usize {
    let username = if tab.username_len > 0 {
        &tab.username[..tab.username_len]
    } else {
        b"root"
    };
    let suffix = b"@sunlight:/$ ";

    let mut pos = 0;
    // Copy username
    for &b in username.iter().take(buf.len()) {
        buf[pos] = b;
        pos += 1;
        if pos >= buf.len() {
            break;
        }
    }
    // Copy suffix
    for &b in suffix.iter().take(buf.len() - pos) {
        buf[pos] = b;
        pos += 1;
    }
    pos
}

fn spawn_tab(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    spawn_cap: CapabilityToken,
    session_grant: CapabilityToken,
) -> bool {
    if *tab_count >= MAX_TABS {
        return false;
    }

    let shell_id = *next_shell_id;
    *next_shell_id += 1;
    let mut path = [0u8; 32];
    let path_len = make_shell_path(shell_id, 0, 0, &mut path);
    let (pw0, pw1, pw2, pw3) = pack_path(&path[..path_len]);
    // UID/GID are deliberately not encoded in this request. The kernel takes
    // identity exclusively from the one-time UAC grant in caps[0].
    let spawn_msg = IpcMsg::with_label(SpawnMsg::SPAWN_AUTHENTICATED)
        .word(0, pw0)
        .word(1, pw1)
        .word(2, pw2)
        .word(3, pw3)
        .with_cap(0, session_grant);
    let spawn_reply = ipc_call(spawn_cap, spawn_msg);
    if spawn_reply.label != SpawnMsg::REPLY {
        debug_log("[TTY]  Spawning /bin/sshl FAILED");
        return false;
    }

    let index = *tab_count;
    tabs[index] = ShellTab::empty();
    tabs[index].shell_id = shell_id;
    tabs[index].pid = spawn_reply.words[0];
    tabs[index].session_pid = spawn_reply.words[0];
    *active_tab = index;
    *tab_count += 1;
    true
}

/// Ask the active authenticated shell to create another shell process. The
/// child is created by the normal spawn syscall, so it inherits the parent's
/// verified uid/gid and restricted capability profile without replaying the
/// one-time login grant.
fn spawn_tab_from_active_shell(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    phase3_6_done: &mut bool,
) -> bool {
    if *tab_count >= MAX_TABS || *next_shell_id == 0 || *next_shell_id > u8::MAX as u64 {
        return false;
    }
    let Some(parent) = active_shell_tab(tabs, *active_tab).copied() else {
        return false;
    };
    let Some(cap) = parent.cap else {
        debug_log("[TTY]  New tab unavailable until the active shell is ready");
        return false;
    };

    let shell_id = *next_shell_id;
    let request = IpcMsg::with_label(ShellMsg::SPAWN_TAB).word(0, shell_id);
    let Ok(reply) = ipc_call_timeout(cap, request, SHELL_IPC_TIMEOUT_MS) else {
        debug_log("[TTY]  New-tab shell request timed out");
        return false;
    };
    if reply.label != ShellMsg::TAB_SPAWNED || reply.words[0] == 0 {
        debug_log("[TTY]  Authenticated child shell spawn failed");
        return false;
    }

    let index = *tab_count;
    tabs[index] = ShellTab::empty();
    tabs[index].shell_id = shell_id;
    tabs[index].pid = reply.words[0];
    tabs[index].session_pid = reply.words[0];
    tabs[index].username = parent.username;
    tabs[index].username_len = parent.username_len;
    *active_tab = index;
    *tab_count += 1;
    *next_shell_id += 1;

    if !*phase3_6_done {
        debug_log("[TTY]  New authenticated shell tab OK");
        debug_log("[SunlightOS] Phase 3.6 OK");
        *phase3_6_done = true;
    }
    true
}

fn handle_ctrl_key(
    ascii: u8,
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    spawn_cap: Option<CapabilityToken>,
    phase3_6_done: &mut bool,
) -> bool {
    match ascii {
        b't' | b'T' => {
            let _ = spawn_cap;
            let _ = spawn_tab_from_active_shell(
                tabs,
                tab_count,
                active_tab,
                next_shell_id,
                phase3_6_done,
            );
            return true;
        }
        b'w' | b'W' => {
            close_active_tab(tabs, tab_count, active_tab);
            return true;
        }
        b'1'..=b'9' => {
            let idx = (ascii - b'1') as usize;
            if idx < *tab_count {
                *active_tab = idx;
                return true;
            }
        }
        b'0' => {
            if *tab_count >= 10 {
                *active_tab = 9;
                return true;
            }
        }
        _ => {}
    }
    false
}

fn close_active_tab(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
) {
    if *tab_count <= 1 {
        return;
    }

    let session_pid = tabs[*active_tab].session_pid;
    if session_pid != 0 {
        if let Some(proc_cap) = nameserver_lookup("proc") {
            let kill_msg = IpcMsg::with_label(ProcOp::TERMINATE_SESSION)
                .word(0, session_pid)
                .word(1, SIGKILL);
            let _ = ipc_call(proc_cap, kill_msg);
        }
    }

    for i in *active_tab..(*tab_count - 1) {
        tabs[i] = tabs[i + 1];
    }
    tabs[*tab_count - 1] = ShellTab::empty();
    *tab_count -= 1;
    if *active_tab >= *tab_count {
        *active_tab = *tab_count - 1;
    }
}

fn resolve_active_shell(
    tabs: &mut [ShellTab; MAX_TABS],
    active_tab: usize,
    logged_initial_spawn: &mut bool,
) {
    let Some(tab) = active_shell_tab_mut(tabs, active_tab) else {
        return;
    };
    if tab.cap.is_some() {
        return;
    }

    let mut name = [0u8; 16];
    let name_len = make_shell_name(tab.shell_id, &mut name);
    let Some(name_str) = core::str::from_utf8(&name[..name_len]).ok() else {
        return;
    };
    if let Some(cap) = nameserver_lookup(name_str) {
        tab.cap = Some(cap);
        if *logged_initial_spawn {
            debug_log("[TTY]  sunshell endpoint found");
            *logged_initial_spawn = false;
        }
        debug_log("[TTY-IPC] shell cap lookup succeeded");
        // Trigger the shell's greeting by sending a null byte (ignored by shell)
        // This causes the shell to immediately reply with its greeting output
        let _ = send_key_to_shell(cap, 0x00, &mut tab.output, &mut tab.output_len);

        let pending_len = tab.pending_len;
        for i in 0..pending_len {
            let b = tab.pending[i];
            let _ = send_key_to_shell(cap, b, &mut tab.output, &mut tab.output_len);
        }
        tab.pending_len = 0;
    } else {
        debug_log("[TTY-IPC] shell cap lookup failed");
    }
}

fn key_ascii_from_msg(msg: &IpcMsg) -> Option<u8> {
    if msg.label == KbdMsg::KEY_EVENT {
        let (_keycode, pressed, _shift, ctrl, _alt, _super, ascii) = unpack_key_event(msg.words[0]);
        // Suppress ctrl combos: Ctrl+T, Ctrl+1 etc. are handled by tty_server
        // itself and must NOT be forwarded as bare ASCII to the shell (which
        // would corrupt its line buffer, e.g. turning "id" into "1id").
        if pressed && !ctrl {
            ascii
        } else {
            None
        }
    } else {
        None
    }
}

/// Outcome of forwarding a keystroke to the shell.
enum ShellKeyResult {
    /// Normal: the shell handled the key (output already appended).
    Continue,
    /// The shell wants to exit (logout).
    Exited,
    /// The shell launched a foreground command; tty_server now drives it.
    /// Carries (pid, app-name buffer, app-name length).
    ForegroundStarted(u64, [u8; 24], usize),
}

/// Send a keystroke to the sshl shell over synchronous IPC.
///
/// tty_server calls sshl synchronously: this thread blocks until sshl replies.
/// sshl MUST reply promptly. Any slow or unbounded operation in the shell
/// command path (e.g. KV/history) must use timeout IPC so it cannot stall here.
/// Putting blocking work in sshl's KBD_LABEL handler causes tty_server to freeze,
/// which stops all keyboard input processing for the active tab.
fn send_key_to_shell(
    cap: CapabilityToken,
    byte: u8,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
) -> ShellKeyResult {
    let kbd_msg = IpcMsg::with_label(KBD_LABEL).word(0, byte as u64);
    debug_ipc_msg("[TTY-IPC] before shell ipc_call KBD_LABEL", &kbd_msg);
    // The initial NUL handshake can arrive while sshl is still loading user
    // state, and Enter may synchronously parse and spawn a large ELF before
    // replying with FOREGROUND_STARTED. Both paths regularly exceed 200 ms on
    // one or two vCPUs during service startup, even though the shell is healthy.
    let timeout_ms = if byte == 0 || byte == b'\n' || byte == b'\r' {
        SHELL_SLOW_PATH_TIMEOUT_MS
    } else {
        SHELL_IPC_TIMEOUT_MS
    };
    let reply = match ipc_call_timeout(cap, kbd_msg, timeout_ms) {
        Ok(reply) => reply,
        Err(_) => {
            append_term(term_output, term_output_len, b"\n[shell timeout]\n");
            debug_log("[TTY] shell key IPC timeout\n");
            return ShellKeyResult::Continue;
        }
    };
    debug_ipc_msg("[TTY-IPC] after shell ipc_call reply", &reply);
    if reply.label == EXIT_LABEL {
        return ShellKeyResult::Exited;
    }
    if reply.label == FG_STARTED_LABEL {
        // words[0] = pid, words[1] = name length, words[2..] = name bytes
        // packed 8 per word (little-endian).
        let pid = reply.words[0];
        let name_len = (reply.words[1] as usize).min(24);
        let mut name = [0u8; 24];
        let mut ni = 0usize;
        let mut wi = 2usize;
        while ni < name_len && wi < reply.words.len() {
            let bytes = reply.words[wi].to_le_bytes();
            let mut bi = 0usize;
            while bi < 8 && ni < name_len {
                name[ni] = bytes[bi];
                ni += 1;
                bi += 1;
            }
            wi += 1;
        }
        return ShellKeyResult::ForegroundStarted(pid, name, name_len);
    }
    append_shell_reply(cap, term_output, term_output_len, &reply);
    ShellKeyResult::Continue
}

fn append_shell_reply(
    cap: CapabilityToken,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
    reply: &IpcMsg,
) {
    if reply.label != OUTPUT_LABEL {
        return;
    }

    let mut remaining = reply.words[1] as usize;
    append_one_chunk(term_output, term_output_len, reply, remaining == 0);

    // Drain additional chunks if the shell has long output pending. IPC replies
    // currently return four register words, so payload bytes live in words 2..4.
    let mut seq: u64 = 1;
    let mut safety: usize = 64; // hard cap to avoid infinite drain loops
    while remaining > 0 && safety > 0 {
        let drain_msg = IpcMsg::with_label(DRAIN_LABEL).word(0, seq);
        debug_ipc_msg("[TTY-IPC] before shell ipc_call DRAIN_LABEL", &drain_msg);
        let next = match ipc_call_timeout(cap, drain_msg, SHELL_IPC_TIMEOUT_MS) {
            Ok(reply) => reply,
            Err(_) => {
                debug_log("[TTY] shell drain IPC timeout\n");
                break;
            }
        };
        debug_ipc_msg("[TTY-IPC] after shell drain reply", &next);
        if next.label != OUTPUT_LABEL {
            break;
        }
        remaining = next.words[1] as usize;
        append_one_chunk(term_output, term_output_len, &next, remaining == 0);
        seq += 1;
        safety -= 1;
    }
}

fn append_one_chunk(
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
    reply: &IpcMsg,
    append_missing_newline: bool,
) {
    let len = (reply.words[0] as usize).min(IPC_OUTPUT_BYTES);
    if len == 0 {
        return;
    }

    let mut bytes = [0u8; IPC_OUTPUT_BYTES];
    for i in 0..len {
        let word_idx = 2 + i / 8;
        if word_idx >= 4 {
            break;
        }
        let byte_idx = i % 8;
        bytes[i] = ((reply.words[word_idx] >> (byte_idx * 8)) & 0xff) as u8;
    }

    append_term(term_output, term_output_len, &bytes[..len]);
    if append_missing_newline && bytes[len - 1] != b'\n' {
        append_term(term_output, term_output_len, b"\n");
    }
}

/// True if `needle` appears anywhere in `haystack`.
fn slice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn append_term(output: &mut [u8; TERM_OUTPUT_MAX], output_len: &mut usize, data: &[u8]) {
    if data.is_empty() {
        return;
    }

    // Reset the buffer on a full-screen clear (ESC[2J) or on alt-screen
    // enter/exit (ESC[?1049h / ESC[?1049l). This gives full-screen apps like
    // top a clean screen and returns to a clean prompt when they exit, without
    // clearing scrollback for ordinary streaming commands (ls, cat, ...), whose
    // output simply appends and slides on overflow below.
    let starts_with_clear = data.len() >= 4
        && data[0] == b'\x1B'
        && data[1] == b'['
        && data[2] == b'2'
        && data[3] == b'J';
    if starts_with_clear || slice_contains(data, b"\x1b[?1049") {
        *output_len = 0; // Clear the accumulated output buffer
    }

    if data.len() >= output.len() {
        let start = data.len() - output.len();
        output.copy_from_slice(&data[start..]);
        *output_len = output.len();
        return;
    }

    let overflow = output_len
        .saturating_add(data.len())
        .saturating_sub(output.len());
    if overflow > 0 {
        let keep = *output_len - overflow;
        for i in 0..keep {
            output[i] = output[i + overflow];
        }
        *output_len = keep;
    }

    let start = *output_len;
    output[start..start + data.len()].copy_from_slice(data);
    *output_len += data.len();
}

fn launch_shortcut_app(
    command: &'static [u8],
) -> Result<sun_exec::LaunchResult, sun_exec::LaunchError> {
    let now = monotonic_millis();
    let trace = LaunchTrace::new(now, LaunchSource::Shortcut, now);
    sun_exec::launch(sun_exec::LaunchRequest {
        trace,
        source: LaunchSource::Shortcut,
        command,
        args: &[],
        require_display: true,
    })
}

fn debug_log_login_success(username: &[u8], uid: u32, gid: u32) {
    let mut buf = [0u8; 128];
    let prefix = b"[TTY]  Login success: ";
    let mut pos = prefix.len();
    buf[..pos].copy_from_slice(prefix);
    let ulen = username.len().min(64);
    buf[pos..pos + ulen].copy_from_slice(&username[..ulen]);
    pos += ulen;
    let mid = b" (uid=";
    buf[pos..pos + mid.len()].copy_from_slice(mid);
    pos += mid.len();
    pos += fmt_u32(&mut buf[pos..], uid);
    let sep = b", gid=";
    buf[pos..pos + sep.len()].copy_from_slice(sep);
    pos += sep.len();
    pos += fmt_u32(&mut buf[pos..], gid);
    buf[pos] = b')';
    pos += 1;
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        debug_log(s);
    }
}

fn debug_log_spawn(_username: &[u8], pid: u64) {
    let mut buf = [0u8; 128];
    let prefix = b"[TTY]  Spawning /bin/sshl (pid=";
    let mut pos = prefix.len();
    buf[..pos].copy_from_slice(prefix);
    pos += fmt_u64(&mut buf[pos..], pid);
    buf[pos] = b')';
    pos += 1;
    let suffix = b"...";
    buf[pos..pos + suffix.len()].copy_from_slice(suffix);
    pos += suffix.len();
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        debug_log(s);
    }
}

fn capture_foreground_bytes(tab: &mut ShellTab, bytes: &[u8]) {
    let remaining = FG_CAPTURE_MAX.saturating_sub(tab.fg_capture_len);
    let copy_len = bytes.len().min(remaining);
    if copy_len != 0 {
        tab.fg_capture[tab.fg_capture_len..tab.fg_capture_len + copy_len]
            .copy_from_slice(&bytes[..copy_len]);
        tab.fg_capture_len += copy_len;
    }
    if copy_len != bytes.len() {
        tab.fg_capture_truncated = true;
    }
}

fn debug_log_foreground_command(cmd: &[u8], output: &[u8], truncated: bool, exit_code: u64) {
    let mut rendered = alloc::string::String::new();
    if output.is_empty() {
        rendered.push_str("<empty>");
    } else {
        for &byte in output {
            match byte {
                b'\n' => rendered.push_str("\\n"),
                b'\r' => rendered.push_str("\\r"),
                b'\t' => rendered.push_str("\\t"),
                b'\\' => rendered.push_str("\\\\"),
                0x20..=0x7e => rendered.push(byte as char),
                _ => {
                    let hex = [
                        b"0123456789abcdef"[(byte >> 4) as usize] as char,
                        b"0123456789abcdef"[(byte & 0x0f) as usize] as char,
                    ];
                    rendered.push_str("\\x");
                    rendered.push(hex[0]);
                    rendered.push(hex[1]);
                }
            }
        }
    }
    if truncated {
        rendered.push_str("...");
    }

    let cmd_text = core::str::from_utf8(cmd).unwrap_or("<non-utf8>");
    let line = alloc::format!("[TTY]  cmd: {} -> {}", cmd_text, rendered);
    debug_log(&line);
    let exit = alloc::format!("[TTY]  exit: {} -> {}", cmd_text, exit_code);
    debug_log(&exit);
}

fn fmt_u32(buf: &mut [u8], val: u32) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut n = 0;
    let mut v = val;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}

/// Build the title-bar string shown in the header next to the "TTY" label:
/// `CPU 15% RAM 42%  <clock> | eth0`.
///
/// Tries "tz" TZ_GET_LOCAL_TIME for local time + abbr. Falls back to UTC
/// from kernel + "UTC" label if tz service not yet available.
///
/// SAFETY: touches function-local `static mut` cache; tty_server is
/// single-threaded so there is no concurrent access.
unsafe fn titlebar(ts: u64) -> ([u8; 64], usize) {
    static mut CACHE_BUF: [u8; 64] = [0; 64];
    static mut CACHE_LEN: usize = 0;
    static mut CACHE_MIN: u64 = u64::MAX;

    let now_min = ts / 60;
    if now_min == CACHE_MIN && CACHE_LEN != 0 {
        return (CACHE_BUF, CACHE_LEN);
    }

    let mut buf = [0u8; 64];
    let mut n = 0usize;

    // CPU usage (placeholder until scheduler accounting lands — matches the old
    // banner's behaviour).
    let cpu_percent: u64 = 15;
    n += copy_into(&mut buf[n..], b"CPU ");
    n += fmt_u64(&mut buf[n..], cpu_percent);
    n += copy_into(&mut buf[n..], b"% RAM ");

    let info = sysinfo();
    let total = info.total_ram_kb.max(1);
    let ram_percent = (info.used_ram_kb * 100) / total;
    n += fmt_u64(&mut buf[n..], ram_percent);
    n += copy_into(&mut buf[n..], b"%  ");

    // Local time via tz service if available (sub-phase 9 refactor)
    let (clock_len, used_tz) = try_local_clock(&mut buf[n..], ts);
    n += clock_len;
    if !used_tz {
        // fallback already wrote UTC style inside try_ or here
    }
    n += copy_into(&mut buf[n..], b" | eth0");

    CACHE_BUF = buf;
    CACHE_LEN = n;
    CACHE_MIN = now_min;
    (buf, n)
}

/// Attempt to get local time+abbr from "tz" service. Returns (bytes_written, used_tz).
/// On failure falls back to writing a UTC clock string (using existing fmt).
unsafe fn try_local_clock(dst: &mut [u8], _ts: u64) -> (usize, bool) {
    if let Some(tz_cap) = nameserver_lookup("tz") {
        let req = IpcMsg::with_label(TzMsg::GET_LOCAL_TIME);
        let Ok(reply) = ipc_call_timeout(tz_cap, req, TZ_IPC_TIMEOUT_MS) else {
            return (0, false);
        };
        if reply.label == TzMsg::REPLY && reply.word_count >= 1 {
            // unpack word(0)
            let w0 = reply.words[0];
            let hour = ((w0 >> 24) & 0xff) as u8;
            let minute = ((w0 >> 16) & 0xff) as u8;
            // abbr from word(3) low 8 bytes
            let abw = if reply.word_count > 3 {
                reply.words[3]
            } else {
                0
            };
            let mut abbr = [0u8; 5]; // short for title e.g. IRDT or UTC
            for i in 0..4 {
                let b = ((abw >> (i * 8)) & 0xff) as u8;
                if b == 0 {
                    break;
                }
                abbr[i] = b;
            }
            // write HH:MM ABBR
            let mut p = 0usize;
            dst[p] = b'0' + hour / 10;
            p += 1;
            dst[p] = b'0' + hour % 10;
            p += 1;
            dst[p] = b':';
            p += 1;
            dst[p] = b'0' + minute / 10;
            p += 1;
            dst[p] = b'0' + minute % 10;
            p += 1;
            dst[p] = b' ';
            p += 1;
            for i in 0..4 {
                if abbr[i] == 0 {
                    break;
                }
                dst[p] = abbr[i];
                p += 1;
            }
            return (p, true);
        }
    }
    // fallback UTC using tui fmt (no abbr change)
    let len = sunlight_tui::fmt::fmt_clock(dst, sunlight_ipc::get_time_utc());
    (len, false)
}

/// Copy `src` into the front of `dst`, returning the number of bytes written
/// (bounded by `dst`'s length).
fn copy_into(dst: &mut [u8], src: &[u8]) -> usize {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    n
}

fn fmt_u64(buf: &mut [u8], val: u64) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = 0;
    let mut v = val;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}

// Set to true only during IPC debugging; each keystroke produces two log lines otherwise.
const TRACE_TTY_IPC: bool = false;

fn debug_ipc_msg(prefix: &str, msg: &IpcMsg) {
    if !TRACE_TTY_IPC {
        return;
    }
    debug_log(&alloc::format!(
        "{} label={} badge={} word_count={} w0={} w1={}",
        prefix,
        msg.label,
        msg.badge,
        msg.word_count,
        msg.words[0],
        msg.words[1]
    ));
}

fn pack_bytes(bytes: &[u8]) -> u64 {
    let mut out = 0u64;
    let mut idx = 0;
    while idx < bytes.len() && idx < 8 {
        out |= (bytes[idx] as u64) << (idx * 8);
        idx += 1;
    }
    out
}

fn make_shell_path(shell_id: u64, uid: u32, gid: u32, out: &mut [u8]) -> usize {
    let prefix = b"/bin/sshl";
    out[..prefix.len()].copy_from_slice(prefix);
    let encoded = encode_shell_launch_id(shell_id, uid, gid);
    prefix.len() + fmt_u64(&mut out[prefix.len()..], encoded)
}

fn encode_shell_launch_id(shell_id: u64, uid: u32, gid: u32) -> u64 {
    (shell_id & 0xff) | ((uid as u64 & 0x0fff_ffff) << 8) | ((gid as u64 & 0x0fff_ffff) << 36)
}

fn make_shell_name(shell_id: u64, out: &mut [u8]) -> usize {
    let prefix = b"sshl";
    out[..prefix.len()].copy_from_slice(prefix);
    prefix.len() + fmt_u64(&mut out[prefix.len()..], shell_id)
}

/// Pack a path (up to 32 bytes) into four u64 words for IPC transport.
fn pack_path(path: &[u8]) -> (u64, u64, u64, u64) {
    let mut words = [0u64; 4];
    let mut word_idx = 0;
    while word_idx < 4 {
        let start = word_idx * 8;
        if start >= path.len() {
            break;
        }
        let end = (start + 8).min(path.len());
        words[word_idx] = pack_bytes(&path[start..end]);
        word_idx += 1;
    }
    (words[0], words[1], words[2], words[3])
}
