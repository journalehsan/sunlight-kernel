use alloc::boxed::Box;
use heapless::{String, Vec};

use crate::protocol::{MAX_HIGHLIGHT_VALUE, MAX_LOCALE_LEN, MAX_NAME_LEN, MAX_VERSION_LEN};

const HOSTNAME_LEN: usize = 64;
const TIMEZONE_LEN: usize = 64;
const MODE_LEN: usize = 24;
const BUILD_LEN: usize = 24;
const ARCH_LEN: usize = 16;
const MAX_CONTEXT_PROVIDERS: usize = 16;
const FAST_REFRESH_MS: u64 = 5_000;
const SLOW_REFRESH_MS: u64 = 30_000;

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
    pub os_version: Option<String<MAX_VERSION_LEN>>,
    pub build: Option<String<BUILD_LEN>>,
    pub architecture: Option<String<ARCH_LEN>>,
    pub locale: Option<String<MAX_LOCALE_LEN>>,
    pub uptime_secs: Option<u64>,
    pub hostname: Option<String<HOSTNAME_LEN>>,
    pub cpu_count: Option<u32>,
    pub ram_mib: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionRuntimeContext {
    pub current_user: Option<String<MAX_NAME_LEN>>,
    pub state: Option<String<MODE_LEN>>,
    pub boot_mode: Option<String<MODE_LEN>>,
    pub desktop_mode: Option<bool>,
    pub installer_mode: Option<bool>,
    pub recovery_mode: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct TimezoneRuntimeContext {
    pub identifier: Option<String<TIMEZONE_LEN>>,
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
}

#[derive(Debug, Clone, Default)]
pub struct PowerRuntimeContext {
    pub requested_profile: Option<String<MODE_LEN>>,
    pub effective_profile: Option<String<MODE_LEN>>,
    pub on_ac: Option<bool>,
    pub battery_percent: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct ThermalRuntimeContext {
    pub state: Option<String<MODE_LEN>>,
    pub temperature_milli_c: Option<i32>,
    pub fan_rpm: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct StorageRuntimeContext {
    pub root_total_bytes: Option<u64>,
    pub root_available_bytes: Option<u64>,
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
    pub provider_count: u8,
    pub provider_failures: u8,
    pub system: SystemRuntimeContext,
    pub session: SessionRuntimeContext,
    pub timezone: TimezoneRuntimeContext,
    pub network: NetworkRuntimeContext,
    pub display: DisplayRuntimeContext,
    pub power: PowerRuntimeContext,
    pub thermal: ThermalRuntimeContext,
    pub storage: StorageRuntimeContext,
    pub services: ServiceRuntimeContext,
}

impl RuntimeContextSnapshot {
    pub fn availability_summary(&self) -> String<MAX_HIGHLIGHT_VALUE> {
        let mut out = String::new();
        if let Some(hostname) = self.system.hostname.as_ref() {
            let _ = out.push_str(hostname.as_str());
        }
        if let Some(zone) = self.timezone.identifier.as_ref() {
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
            || self.timezone.identifier.is_some()
            || self.network.connected.is_some()
            || self.display.width_px.is_some()
            || self.services.sunlightd.is_some();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshClass {
    Static,
    Slow,
    Fast,
}

impl RefreshClass {
    const fn interval_ms(self) -> Option<u64> {
        match self {
            Self::Static => None,
            Self::Slow => Some(SLOW_REFRESH_MS),
            Self::Fast => Some(FAST_REFRESH_MS),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextProviderError {
    Unavailable,
}

/// A read-only adapter over one existing subsystem.
///
/// Providers never own service state. `clear` identifies their snapshot fields,
/// while `collect` copies a bounded current view from the owning subsystem.
pub trait ContextProvider {
    fn name(&self) -> &'static str;
    fn refresh_class(&self) -> RefreshClass;
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot);
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError>;
}

struct RegisteredProvider {
    provider: Box<dyn ContextProvider>,
    last_refresh_mono_ms: Option<u64>,
    failed: bool,
}

pub struct RuntimeContextCache {
    snapshot: RuntimeContextSnapshot,
    providers: Vec<RegisteredProvider, MAX_CONTEXT_PROVIDERS>,
}

impl RuntimeContextCache {
    pub fn new() -> Self {
        let mut cache = Self::empty();
        let _ = cache.register(Box::new(SystemProvider));
        let _ = cache.register(Box::new(UptimeProvider));
        let _ = cache.register(Box::new(SessionProvider));
        let _ = cache.register(Box::new(TimezoneProvider));
        let _ = cache.register(Box::new(NetworkProvider));
        let _ = cache.register(Box::new(DisplayProvider));
        let _ = cache.register(Box::new(ServiceProvider));
        cache.refresh_at(monotonic_ms(), true);
        cache
    }

    pub fn empty() -> Self {
        Self {
            snapshot: RuntimeContextSnapshot::default(),
            providers: Vec::new(),
        }
    }

    pub fn register(
        &mut self,
        provider: Box<dyn ContextProvider>,
    ) -> Result<(), Box<dyn ContextProvider>> {
        self.providers
            .push(RegisteredProvider {
                provider,
                last_refresh_mono_ms: None,
                failed: false,
            })
            .map_err(|entry| entry.provider)
    }

    pub fn snapshot(&self) -> &RuntimeContextSnapshot {
        &self.snapshot
    }

    pub fn refresh_if_due(&mut self) {
        self.refresh_at(monotonic_ms(), false);
    }

    pub fn refresh(&mut self) {
        self.refresh_at(monotonic_ms(), true);
    }

    fn refresh_at(&mut self, now: u64, force: bool) {
        let mut next = self.snapshot.clone();
        for entry in self.providers.iter_mut() {
            let due = force
                || match (
                    entry.last_refresh_mono_ms,
                    entry.provider.refresh_class().interval_ms(),
                ) {
                    (None, _) => true,
                    (Some(_), None) => false,
                    (Some(last), Some(interval)) => now.saturating_sub(last) >= interval,
                };
            if !due {
                continue;
            }

            let mut cleared = next.clone();
            entry.provider.clear(&mut cleared);
            let mut collected = cleared.clone();
            if entry.provider.collect(&mut collected).is_ok() {
                next = collected;
                entry.failed = false;
            } else {
                next = cleared;
                entry.failed = true;
            }
            entry.last_refresh_mono_ms = Some(now);
        }
        next.captured_mono_ms = now;
        next.provider_count = self.providers.len() as u8;
        next.provider_failures = self.providers.iter().filter(|entry| entry.failed).count() as u8;
        next.recompute_available();
        self.snapshot = next;
    }
}

impl Default for RuntimeContextCache {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SystemProvider;

impl ContextProvider for SystemProvider {
    fn name(&self) -> &'static str {
        "system"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Static
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.system.os_version = None;
        snapshot.system.build = None;
        snapshot.system.architecture = None;
        snapshot.system.locale = None;
        snapshot.system.hostname = None;
        snapshot.system.cpu_count = None;
        snapshot.system.ram_mib = None;
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        snapshot.system.os_version = Some(fixed_str(env!("CARGO_PKG_VERSION")));
        snapshot.system.build =
            Some(read_release_generation().unwrap_or_else(|| fixed_str(env!("CARGO_PKG_VERSION"))));
        snapshot.system.architecture = Some(fixed_str(target_arch_label()));
        snapshot.system.locale = read_locale();
        snapshot.system.hostname = read_hostname();
        refresh_static_system(snapshot);
        Ok(())
    }
}

pub struct UptimeProvider;

impl ContextProvider for UptimeProvider {
    fn name(&self) -> &'static str {
        "uptime"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Fast
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.system.uptime_secs = None;
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_uptime(snapshot);
        snapshot
            .system
            .uptime_secs
            .map(|_| ())
            .ok_or(ContextProviderError::Unavailable)
    }
}

pub struct SessionProvider;

impl ContextProvider for SessionProvider {
    fn name(&self) -> &'static str {
        "session"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Fast
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.session = SessionRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_session(snapshot);
        if snapshot.session.state.is_some() || snapshot.session.current_user.is_some() {
            Ok(())
        } else {
            Err(ContextProviderError::Unavailable)
        }
    }
}

pub struct TimezoneProvider;

impl ContextProvider for TimezoneProvider {
    fn name(&self) -> &'static str {
        "timezone"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Slow
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.timezone = TimezoneRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_timezone(snapshot);
        snapshot
            .timezone
            .identifier
            .as_ref()
            .map(|_| ())
            .ok_or(ContextProviderError::Unavailable)
    }
}

pub struct NetworkProvider;

impl ContextProvider for NetworkProvider {
    fn name(&self) -> &'static str {
        "network"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Slow
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.network = NetworkRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_network(snapshot);
        snapshot
            .network
            .available
            .map(|_| ())
            .ok_or(ContextProviderError::Unavailable)
    }
}

pub struct DisplayProvider;

impl ContextProvider for DisplayProvider {
    fn name(&self) -> &'static str {
        "display"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Slow
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.display = DisplayRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_display(snapshot);
        snapshot
            .display
            .width_px
            .map(|_| ())
            .ok_or(ContextProviderError::Unavailable)
    }
}

pub struct PowerProvider;

impl ContextProvider for PowerProvider {
    fn name(&self) -> &'static str {
        "power"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Fast
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.power = PowerRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_power(snapshot)
    }
}

pub struct ThermalProvider;

impl ContextProvider for ThermalProvider {
    fn name(&self) -> &'static str {
        "thermal"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Fast
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.thermal = ThermalRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_thermal(snapshot)
    }
}

pub struct StorageProvider;

impl ContextProvider for StorageProvider {
    fn name(&self) -> &'static str {
        "storage"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Slow
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.storage = StorageRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_storage(snapshot)
    }
}

pub struct ServiceProvider;

impl ContextProvider for ServiceProvider {
    fn name(&self) -> &'static str {
        "services"
    }
    fn refresh_class(&self) -> RefreshClass {
        RefreshClass::Slow
    }
    fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
        snapshot.services = ServiceRuntimeContext::default();
    }
    fn collect(&self, snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
        refresh_services(snapshot);
        snapshot
            .services
            .sessiond
            .map(|_| ())
            .ok_or(ContextProviderError::Unavailable)
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
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
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
fn refresh_static_system(snapshot: &mut RuntimeContextSnapshot) {
    let info = sunlight_ipc::sysinfo();
    snapshot.system.ram_mib = u32::try_from(info.total_ram_kb / 1024)
        .ok()
        .filter(|mib| *mib > 0);
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_static_system(_snapshot: &mut RuntimeContextSnapshot) {}

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
        let identifier = fixed_str(id);
        snapshot.timezone.identifier = Some(identifier);
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
        snapshot.session.current_user = Some(name);
    }
    if let Some(state) = SessionState::from_u64(state_raw) {
        let state = fixed_str(session_state_label(state));
        snapshot.session.state = Some(state);
    }
    if let Some(kind) = SessionKind::from_u64(kind_raw) {
        match kind {
            SessionKind::Desktop => {
                let mode = fixed_str("desktop");
                snapshot.session.boot_mode = Some(mode);
                snapshot.session.desktop_mode = Some(true);
                snapshot.session.recovery_mode = Some(false);
            }
            SessionKind::SafeDesktop => {
                let mode = fixed_str("safe-desktop");
                snapshot.session.boot_mode = Some(mode);
                snapshot.session.desktop_mode = Some(false);
                snapshot.session.recovery_mode = Some(true);
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
        let Some(uid_part) = parts.next() else {
            continue;
        };
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
    snapshot.network.connected = Some(matches!(panel.state, DerivedConnectionState::Connected));
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
    snapshot.display.scale_percent =
        Some(((metrics.scale_fp as u64).saturating_mul(100) / 65_536).min(u32::MAX as u64) as u32);
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_display(_snapshot: &mut RuntimeContextSnapshot) {}

#[cfg(feature = "sunlightos")]
fn refresh_power(snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
    use sunlight_ipc::{
        ipc_call_timeout, nameserver_lookup_timeout, IpcMsg, PowerProfile, PowerdMsg,
    };

    let ep = nameserver_lookup_timeout("powerd", 50).ok_or(ContextProviderError::Unavailable)?;
    let reply = ipc_call_timeout(ep, IpcMsg::with_label(PowerdMsg::GET_STATUS), 50)
        .map_err(|_| ContextProviderError::Unavailable)?;
    if reply.label != PowerdMsg::REPLY {
        return Err(ContextProviderError::Unavailable);
    }
    snapshot.power.requested_profile =
        Some(fixed_str(PowerProfile::from_u64(reply.words[0]).as_str()));
    snapshot.power.effective_profile =
        Some(fixed_str(PowerProfile::from_u64(reply.words[1]).as_str()));
    let context = reply.words[2];
    if context & 1 != 0 {
        snapshot.power.on_ac = Some(context & 2 != 0);
    }
    if context & (1 << 2) != 0 {
        snapshot.power.battery_percent = Some(((context >> 3) & 0xff) as u8);
    }
    Ok(())
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_power(_snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
    Err(ContextProviderError::Unavailable)
}

#[cfg(feature = "sunlightos")]
fn refresh_thermal(snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
    use sunlight_ipc::{
        ipc_call_timeout, nameserver_lookup_timeout, IpcMsg, ThermalState, ThermaldMsg,
    };

    let ep = nameserver_lookup_timeout("thermald", 50).ok_or(ContextProviderError::Unavailable)?;
    let reply = ipc_call_timeout(ep, IpcMsg::with_label(ThermaldMsg::GET_STATUS), 50)
        .map_err(|_| ContextProviderError::Unavailable)?;
    if reply.label != ThermaldMsg::REPLY {
        return Err(ContextProviderError::Unavailable);
    }
    let state = ThermalState::from_u64(reply.words[0] & 0xff);
    snapshot.thermal.state = Some(fixed_str(state.as_str()));
    let temperature = reply.words[1] as u32 as i32;
    if temperature != i32::MIN && state.has_valid_controlling_sensor() {
        snapshot.thermal.temperature_milli_c = Some(temperature);
    }
    snapshot.thermal.fan_rpm = Some(((reply.words[2] >> 8) & 0xffff) as u32);
    Ok(())
}

#[cfg(not(feature = "sunlightos"))]
fn refresh_thermal(_snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
    Err(ContextProviderError::Unavailable)
}

// SunlightOS currently has no read-only filesystem-capacity service contract.
// Keep storage unknown rather than reading or duplicating storage-manager state.
fn refresh_storage(_snapshot: &mut RuntimeContextSnapshot) -> Result<(), ContextProviderError> {
    Err(ContextProviderError::Unavailable)
}

#[cfg(feature = "sunlightos")]
fn refresh_services(snapshot: &mut RuntimeContextSnapshot) {
    snapshot.services.display = service_status_from_lookup("display_server");
    snapshot.services.sessiond = service_status_from_lookup(sunlight_ipc::SESSION_ENDPOINT);
    snapshot.services.networkd = service_status_from_lookup("networkd");
    snapshot.services.resolved = service_status_from_lookup("resolved");
    snapshot.services.timed = service_status_from_lookup("timed");
    snapshot.services.timezone_service = service_status_from_lookup("tz");
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

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;

    use super::*;

    struct TestStorageProvider {
        class: RefreshClass,
        calls: Rc<Cell<u32>>,
        fail: Rc<Cell<bool>>,
    }

    impl ContextProvider for TestStorageProvider {
        fn name(&self) -> &'static str {
            "test-storage"
        }

        fn refresh_class(&self) -> RefreshClass {
            self.class
        }

        fn clear(&self, snapshot: &mut RuntimeContextSnapshot) {
            snapshot.storage = StorageRuntimeContext::default();
        }

        fn collect(
            &self,
            snapshot: &mut RuntimeContextSnapshot,
        ) -> Result<(), ContextProviderError> {
            self.calls.set(self.calls.get() + 1);
            if self.fail.get() {
                return Err(ContextProviderError::Unavailable);
            }
            snapshot.storage.root_total_bytes = Some(1024);
            Ok(())
        }
    }

    #[test]
    fn provider_failure_clears_owned_fields_and_publishes_snapshot() {
        let calls = Rc::new(Cell::new(0));
        let fail = Rc::new(Cell::new(false));
        let mut cache = RuntimeContextCache::empty();
        assert!(cache
            .register(Box::new(TestStorageProvider {
                class: RefreshClass::Fast,
                calls,
                fail: fail.clone(),
            }))
            .is_ok());

        cache.refresh_at(1, true);
        assert_eq!(cache.snapshot().storage.root_total_bytes, Some(1024));
        fail.set(true);
        cache.refresh_at(2, true);

        assert_eq!(cache.snapshot().storage.root_total_bytes, None);
        assert_eq!(cache.snapshot().provider_failures, 1);
    }

    #[test]
    fn refresh_class_avoids_per_request_collection() {
        let calls = Rc::new(Cell::new(0));
        let mut cache = RuntimeContextCache::empty();
        assert!(cache
            .register(Box::new(TestStorageProvider {
                class: RefreshClass::Fast,
                calls: calls.clone(),
                fail: Rc::new(Cell::new(false)),
            }))
            .is_ok());

        cache.refresh_at(100, false);
        cache.refresh_at(4_999, false);
        assert_eq!(calls.get(), 1);
        cache.refresh_at(5_100, false);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn empty_registry_produces_a_safe_unknown_snapshot() {
        let mut cache = RuntimeContextCache::empty();
        cache.refresh_at(42, false);
        assert!(!cache.snapshot().available);
        assert_eq!(cache.snapshot().provider_count, 0);
        assert_eq!(cache.snapshot().provider_failures, 0);
    }
}
