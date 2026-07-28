use heapless::String;

use crate::protocol::{MAX_HIGHLIGHT_VALUE, MAX_LOCALE_LEN, MAX_NAME_LEN, MAX_VERSION_LEN};

const HOSTNAME_LEN: usize = 64;
const TIMEZONE_LEN: usize = 64;
const MODE_LEN: usize = 24;
const BUILD_LEN: usize = 24;
const ARCH_LEN: usize = 16;
const REFRESH_INTERVAL_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeServiceStatus {
    Running,
    Starting,
    Stopping,
    Stopped,
    Failed,
    Restarting,
    Unavailable,
}

impl RuntimeServiceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Starting => "starting",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Restarting => "restarting",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SystemRuntimeContext {
    pub boot_mode: Option<String<MODE_LEN>>,
    pub desktop_mode: Option<bool>,
    pub installer_mode: Option<bool>,
    pub recovery_mode: Option<bool>,
    pub os_version: Option<String<MAX_VERSION_LEN>>,
    pub build: Option<String<BUILD_LEN>>,
    pub architecture: Option<String<ARCH_LEN>>,
    pub locale: Option<String<MAX_LOCALE_LEN>>,
    pub timezone: Option<String<TIMEZONE_LEN>>,
    pub uptime_secs: Option<u64>,
    pub hostname: Option<String<HOSTNAME_LEN>>,
    pub current_user: Option<String<MAX_NAME_LEN>>,
    pub session_state: Option<String<MODE_LEN>>,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkRuntimeContext {
    pub available: Option<bool>,
    pub connected: Option<bool>,
    pub interface_count: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayRuntimeContext {
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub scale_percent: Option<u32>,
    pub session_state: Option<String<MODE_LEN>>,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceRuntimeContext {
    pub sunlightd: Option<RuntimeServiceStatus>,
    pub sessiond: Option<RuntimeServiceStatus>,
    pub networkd: Option<RuntimeServiceStatus>,
    pub resolved: Option<RuntimeServiceStatus>,
    pub timezone_service: Option<RuntimeServiceStatus>,
    pub timed: Option<RuntimeServiceStatus>,
    pub powerd: Option<RuntimeServiceStatus>,
    pub thermald: Option<RuntimeServiceStatus>,
    pub display: Option<RuntimeServiceStatus>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeContextSnapshot {
    pub available: bool,
    pub captured_mono_ms: u64,
    pub system: SystemRuntimeContext,
    pub network: NetworkRuntimeContext,
    pub display: DisplayRuntimeContext,
    pub services: ServiceRuntimeContext,
}

impl RuntimeContextSnapshot {
    pub fn availability_summary(&self) -> String<MAX_HIGHLIGHT_VALUE> {
        let mut out = String::new();
        if let Some(hostname) = self.system.hostname.as_ref() {
            let _ = out.push_str(hostname.as_str());
        }
        if let Some(zone) = self.system.timezone.as_ref() {
            if !out.is_empty() {
                let _ = out.push_str(" ");
            }
            let _ = out.push_str(zone.as_str());
        }
        if let Some(connected) = self.network.connected {
            if !out.is_empty() {
                let _ = out.push_str(" ");
            }
            let _ = out.push_str(if connected { "online" } else { "offline" });
        }
        out
    }

    fn recompute_available(&mut self) {
        self.available = self.system.os_version.is_some()
            || self.system.hostname.is_some()
            || self.system.timezone.is_some()
            || self.network.connected.is_some()
            || self.display.width_px.is_some()
            || self.services.sunlightd.is_some();
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeContextCache {
    snapshot: RuntimeContextSnapshot,
    next_refresh_mono_ms: u64,
    fixed_loaded: bool,
}

impl RuntimeContextCache {
    pub fn new() -> Self {
        let mut cache = Self::default();
        cache.refresh_all();
        cache
    }

    pub fn snapshot(&self) -> &RuntimeContextSnapshot {
        &self.snapshot
    }

    pub fn refresh_if_due(&mut self) {
        let now = monotonic_ms();
        if !self.fixed_loaded || now >= self.next_refresh_mono_ms {
            self.refresh_all();
        }
    }

    fn refresh_all(&mut self) {
        if !self.fixed_loaded {
            self.load_fixed_fields();
            self.fixed_loaded = true;
        }
        self.refresh_dynamic_fields();
        self.snapshot.recompute_available();
        self.next_refresh_mono_ms = self
            .snapshot
            .captured_mono_ms
            .saturating_add(REFRESH_INTERVAL_MS);
    }

    fn load_fixed_fields(&mut self) {
        self.snapshot.system.os_version = Some(fixed_str(env!("CARGO_PKG_VERSION")));
        self.snapshot.system.build = Some(read_release_generation().unwrap_or_else(|| {
            let mut build = String::new();
            let _ = build.push_str(env!("CARGO_PKG_VERSION"));
            build
        }));
        self.snapshot.system.architecture = Some(fixed_str(target_arch_label()));
        self.snapshot.system.locale = read_locale();
        self.snapshot.system.hostname = read_hostname();
    }

    fn refresh_dynamic_fields(&mut self) {
        self.snapshot.captured_mono_ms = monotonic_ms();
        self.snapshot.system.timezone = None;
        self.snapshot.system.uptime_secs = None;
        self.snapshot.system.current_user = None;
        self.snapshot.system.boot_mode = None;
        self.snapshot.system.desktop_mode = None;
        self.snapshot.system.installer_mode = None;
        self.snapshot.system.recovery_mode = None;
        self.snapshot.system.session_state = None;

        self.snapshot.network = NetworkRuntimeContext::default();
        self.snapshot.display = DisplayRuntimeContext::default();
        self.snapshot.services = ServiceRuntimeContext::default();

        refresh_uptime(&mut self.snapshot);
        refresh_timezone(&mut self.snapshot);
        refresh_session(&mut self.snapshot);
        refresh_network(&mut self.snapshot);
        refresh_display(&mut self.snapshot);
        refresh_services(&mut self.snapshot);
    }
}

fn fixed_str<const N: usize>(value: &str) -> String<N> {
    let mut out = String::new();
    for ch in value.chars().take(N) {
        let _ = out.push(ch);
    }
    out
}

fn target_arch_label() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64"
    }
    #[cfg(target_arch = "riscv64")]
    {
        "riscv64"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64")))]
    {
        "unknown"
    }
}

#[cfg(feature = "sunlightos")]
fn monotonic_ms() -> u64 {
    sunlight_ipc::monotonic_millis()
}

#[cfg(not(feature = "sunlightos"))]
fn monotonic_ms() -> u64 {
    0
}

#[cfg(feature = "sunlightos")]
fn read_text_file<const N: usize>(path: &[u8]) -> Option<String<N>> {
    let fd = sunlight_libc::open(path).ok()?;
    let mut buf = [0u8; 256];
    let read = sunlight_libc::read(fd, &mut buf).ok()?;
    let _ = sunlight_libc::close(fd);
    let text = core::str::from_utf8(&buf[..read]).ok()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(fixed_str(text))
    }
}

#[cfg(not(feature = "sunlightos"))]
fn read_text_file<const N: usize>(_path: &[u8]) -> Option<String<N>> {
    None
}

fn read_hostname() -> Option<String<HOSTNAME_LEN>> {
    read_text_file(b"/etc/hostname")
}

fn read_release_generation() -> Option<String<BUILD_LEN>> {
    read_text_file(b"/etc/sunlight/release-generation")
}

#[cfg(feature = "sunlightos")]
fn read_locale() -> Option<String<MAX_LOCALE_LEN>> {
    let fd = sunlight_libc::open(b"/etc/locale.conf").ok()?;
    let mut buf = [0u8; 512];
    let read = sunlight_libc::read(fd, &mut buf).ok()?;
    let _ = sunlight_libc::close(fd);
    let cfg = sunlight_locale::parse_locale_conf(&buf[..read]);
    let locale = cfg.lc_time();
    if locale.is_empty() {
        None
    } else {
        Some(fixed_str(locale))
    }
}

#[cfg(not(feature = "sunlightos"))]
fn read_locale() -> Option<String<MAX_LOCALE_LEN>> {
    None
}

#[cfg(feature = "sunlightos")]
fn refresh_uptime(snapshot: &mut RuntimeContextSnapshot) {
    let info = sunlight_ipc::sysinfo();
    if info.uptime_secs > 0 {
        snapshot.system.uptime_secs = Some(info.uptime_secs);
    }
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_uptime(_snapshot: &mut RuntimeContextSnapshot) {}

#[cfg(feature = "sunlightos")]
fn refresh_timezone(snapshot: &mut RuntimeContextSnapshot) {
    let Ok(client) = sunlight_tz::client::TzClient::connect() else {
        return;
    };
    let Ok(zone) = client.get_zone() else {
        return;
    };
    let id = zone.id_str();
    if !id.is_empty() {
        snapshot.system.timezone = Some(fixed_str(id));
    }
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_timezone(_snapshot: &mut RuntimeContextSnapshot) {}

#[cfg(feature = "sunlightos")]
fn refresh_session(snapshot: &mut RuntimeContextSnapshot) {
    use sunlight_ipc::{
        ipc_call_timeout, nameserver_lookup_timeout, IpcMsg, SessionKind, SessionMsg, SessionState,
        SESSION_ENDPOINT,
    };

    let Some(ep) = nameserver_lookup_timeout(SESSION_ENDPOINT, 50) else {
        return;
    };
    let reply = match ipc_call_timeout(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_LIST).word(0, 0),
        50,
    ) {
        Ok(reply) => reply,
        Err(_) => return,
    };
    if reply.label != SessionMsg::REPLY {
        return;
    }

    let uid = reply.words[2] as u32;
    let state_raw = (reply.words[2] >> 32) & 0xff;
    let kind_raw = (reply.words[2] >> 40) & 0xff;

    if let Some(name) = lookup_uid_name(uid) {
        snapshot.system.current_user = Some(name);
    }
    if let Some(state) = SessionState::from_u64(state_raw) {
        snapshot.system.session_state = Some(fixed_str(session_state_label(state)));
        snapshot.display.session_state = Some(fixed_str(session_state_label(state)));
    }
    if let Some(kind) = SessionKind::from_u64(kind_raw) {
        match kind {
            SessionKind::Desktop => {
                snapshot.system.boot_mode = Some(fixed_str("desktop"));
                snapshot.system.desktop_mode = Some(true);
                snapshot.system.recovery_mode = Some(false);
            }
            SessionKind::SafeDesktop => {
                snapshot.system.boot_mode = Some(fixed_str("safe-desktop"));
                snapshot.system.desktop_mode = Some(false);
                snapshot.system.recovery_mode = Some(true);
            }
        }
    }
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_session(_snapshot: &mut RuntimeContextSnapshot) {}

#[cfg(feature = "sunlightos")]
fn session_state_label(state: sunlight_ipc::SessionState) -> &'static str {
    use sunlight_ipc::SessionState;

    match state {
        SessionState::Created => "created",
        SessionState::Preparing => "preparing",
        SessionState::StartingRequiredComponents => "starting",
        SessionState::Running => "running",
        SessionState::Degraded => "degraded",
        SessionState::Locking => "locking",
        SessionState::Locked => "locked",
        SessionState::Stopping => "stopping",
        SessionState::Stopped => "stopped",
        SessionState::Failed => "failed",
    }
}

#[cfg(feature = "sunlightos")]
fn lookup_uid_name(uid: u32) -> Option<String<MAX_NAME_LEN>> {
    let fd = sunlight_libc::open(b"/etc/passwd").ok()?;
    let mut buf = [0u8; 2048];
    let read = sunlight_libc::read(fd, &mut buf).ok()?;
    let _ = sunlight_libc::close(fd);
    let text = core::str::from_utf8(&buf[..read]).ok()?;
    for line in text.lines() {
        let mut parts = line.split(':');
        let Some(name) = parts.next() else { continue };
        let _ = parts.next();
        let Some(uid_part) = parts.next() else { continue };
        if uid_part.parse::<u32>().ok() == Some(uid) {
            return Some(fixed_str(name));
        }
    }
    None
}

#[cfg(feature = "sunlightos")]
fn refresh_network(snapshot: &mut RuntimeContextSnapshot) {
    use sunlight_networkd::{DerivedConnectionState, NetworkClient};

    let client = NetworkClient::new();
    let Ok(network) = client.snapshot() else {
        return;
    };
    let panel = sunlight_networkd::NetworkPanelSummary::from_snapshot(&network);
    snapshot.network.available = Some(panel.has_interface());
    snapshot.network.connected = Some(matches!(
        panel.state,
        DerivedConnectionState::Connected
    ));
    snapshot.network.interface_count = u8::try_from(network.interfaces.len()).ok();
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_network(_snapshot: &mut RuntimeContextSnapshot) {}

#[cfg(feature = "sunlightos")]
fn refresh_display(snapshot: &mut RuntimeContextSnapshot) {
    let Some(display_ep) = sunlight_ipc::nameserver_lookup_timeout("display_server", 50) else {
        return;
    };
    let Some(metrics) = sunlight_ipc::query_display_metrics(display_ep) else {
        return;
    };
    snapshot.display.width_px = Some(metrics.width_px);
    snapshot.display.height_px = Some(metrics.height_px);
    snapshot.display.scale_percent = Some(
        ((metrics.scale_fp as u64).saturating_mul(100) / 65_536).min(u32::MAX as u64) as u32,
    );
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_display(_snapshot: &mut RuntimeContextSnapshot) {}

#[cfg(feature = "sunlightos")]
fn refresh_services(snapshot: &mut RuntimeContextSnapshot) {
    use sunlight_ipc::{ipc_call_timeout, nameserver_lookup_timeout, IpcMsg};
    use sunlightd::ipc::SunlightdOp;

    snapshot.services.sunlightd = service_status_from_lookup("sunlightd");
    snapshot.services.powerd = service_status_from_lookup("powerd");
    snapshot.services.display = service_status_from_lookup("display_server");
    snapshot.services.sessiond = service_status_from_lookup(sunlight_ipc::SESSION_ENDPOINT);
    snapshot.services.networkd = service_status_from_lookup("networkd");
    snapshot.services.resolved = service_status_from_lookup("resolved");
    snapshot.services.timed = service_status_from_lookup("timed");
    snapshot.services.timezone_service = service_status_from_lookup("tz");
    snapshot.services.thermald = service_status_from_lookup("thermald");

    let Some(ep) = nameserver_lookup_timeout("sunlightd", 50) else {
        return;
    };
    let first = match ipc_call_timeout(ep, IpcMsg::with_label(SunlightdOp::List as u64).word(0, 0), 50) {
        Ok(reply) => reply,
        Err(_) => return,
    };
    if first.label != SunlightdOp::List as u64 && first.label != 0 {
        // sunlightd replies with the original control op label.
    }

    let total = (first.words[0] & 0xffff_ffff) as usize;
    apply_supervised_service_entry(&first, &mut snapshot.services);
    for index in 1..total.min(32) {
        let reply = match ipc_call_timeout(
            ep,
            IpcMsg::with_label(SunlightdOp::List as u64).word(0, index as u64),
            50,
        ) {
            Ok(reply) => reply,
            Err(_) => break,
        };
        apply_supervised_service_entry(&reply, &mut snapshot.services);
    }
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_services(_snapshot: &mut RuntimeContextSnapshot) {}

#[cfg(feature = "sunlightos")]
fn service_status_from_lookup(name: &str) -> Option<RuntimeServiceStatus> {
    if sunlight_ipc::nameserver_lookup_timeout(name, 20).is_some() {
        Some(RuntimeServiceStatus::Running)
    } else {
        Some(RuntimeServiceStatus::Unavailable)
    }
}

#[cfg(feature = "sunlightos")]
fn apply_supervised_service_entry(
    reply: &sunlight_ipc::IpcMsg,
    services: &mut ServiceRuntimeContext,
) {
    let state = runtime_service_status_from_wire(((reply.words[0] >> 32) & 0xff) as u32);
    let name = unpack_name(reply.words[2], reply.words[3]);
    match name.as_str() {
        "networkd" => services.networkd = Some(state),
        "resolved" => services.resolved = Some(state),
        "thermald" => services.thermald = Some(state),
        "timed" => services.timed = Some(state),
        "timezone_service" => services.timezone_service = Some(state),
        "sunlight-sessiond" => services.sessiond = Some(state),
        "sunlight-display" => services.display = Some(state),
        _ => {}
    }
}

#[cfg(feature = "sunlightos")]
fn unpack_name(word2: u64, word3: u64) -> String<32> {
    let mut out = String::new();
    for word in [word2, word3] {
        for idx in 0..8 {
            let byte = ((word >> (idx * 8)) & 0xff) as u8;
            if byte == 0 {
                return out;
            }
            let _ = out.push(byte as char);
        }
    }
    out
}

#[cfg(feature = "sunlightos")]
fn runtime_service_status_from_wire(state: u32) -> RuntimeServiceStatus {
    match state {
        0 => RuntimeServiceStatus::Stopped,
        1 => RuntimeServiceStatus::Starting,
        2 => RuntimeServiceStatus::Running,
        3 => RuntimeServiceStatus::Failed,
        4 => RuntimeServiceStatus::Restarting,
        5 => RuntimeServiceStatus::Stopping,
        _ => RuntimeServiceStatus::Unavailable,
    }
}
