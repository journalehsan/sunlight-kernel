#![no_std]

pub mod display_metrics;
pub mod display_modes;
pub mod swap_policy;
pub use display_metrics::{
    validate_size, DisplayMetrics, PixelFormat, ScreenBackend, ScreenRect,
    BORDER_W as DISPLAY_BORDER_W, MAX_DIM, MIN_DIM, SAFE_FALLBACK_H, SAFE_FALLBACK_W, SCALE_FP_ONE,
};
pub use display_modes::{
    DisplayMode, DisplayModeCapabilities, DisplayModeManagement, DisplayModeReadOnlyReason,
    DisplayModeTransaction, DEFAULT_MODE_PREVIEW_TIMEOUT_MS, MAX_DISPLAY_MODES,
};

pub const IPC_MAX_WORDS: usize = 8;
/// Register IPC currently transports only `words[0..4)` via r8/r9/r10/r12.
pub const IPC_REGISTER_WORDS: usize = 4;
pub const IPC_MAX_CAPS: usize = 2;
pub const INIT_NAMESERVER_ENDPOINT: u64 = 0;

/// Syscall numbers for SunlightOS.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunlightSyscall {
    IpcCall = 1,
    IpcReply = 2,
    IpcReplyWait = 3,
    IpcRecv = 4,
    IpcNotifySend = 5,
    IpcNotifyWait = 6,
    IpcCancel = 7,
    IpcSetDeadline = 8,
    EndpointCreate = 10,
    EndpointBind = 11,
    NameserverEndpointValidate = 12,
    EndpointDestroy = 13,
    NameserverDiagnosticEvent = 14,
    /// Retain the current server reply target for a later explicit completion.
    IpcDeferReply = 15,
    /// Deliver a reply to a target previously retained with IpcDeferReply.
    IpcCompleteDeferredReply = 16,
    /// Query whether a deferred server reply target is still live.
    IpcDeferredReplyIsLive = 17,
    ProcessExit = 20,
    ProcessYield = 21,
    ThreadSpawn = 22,
    // TTY mux for foreground input routing — see kernel process::tty_io.
    TtyStdinPush = 23,
    TtyStdoutPull = 24,
    ProcessIsAlive = 25,
    WaitPid = 32,
    GetPid = 33,
    Kill = 72,
    // NOTE: 50 belongs to sys_mmap in the kernel dispatcher — GetTimeUtc
    // previously sat there and silently invoked mmap.
    GetTimeUtc = 81,
    MonotonicMs = 86,
    SysInfo = 82,
    SetNice = 83,
    GetNice = 84,
    SwapCtl = 85,
    /// Conditioned secure entropy word. Check `SecureEntropyReady` first.
    GetEntropy = 87,
    /// Returns 1 only after approved entropy collection completes.
    SecureEntropyReady = 89,
    // Phase 3.4: net_server (pid 5) frame proxy — kernel owns the virtio-net
    // device (ring-0 port I/O); these exchange raw Ethernet frames.
    NetTx = 90,
    NetRx = 91,
    NetInfo = 96,
    // Shared memory grant for large zero-copy IPC (Bite 4)
    ShmAlloc = 92,
    ShmMap = 93,
    ShmFree = 94,
    MapTelemetry = 95,
    MouseRegister = 114,
    MousePopByte = 115,
    MouseInit = 116,
    MousePortRead = 117,
    // GUI / Display: map the physical Limine framebuffer into user space for the compositor
    MapFramebuffer = 118,
    // VirtIO GPU proxy syscalls (display_server only, gated by process name)
    GpuGetInfo = 119,
    GpuAttachBacking = 120,
    GpuSetScanout = 121,
    GpuFlush = 122,
    GpuUpdateCursor = 123,
    GpuMoveCursor = 124,
    HardwareInventory = 126,
    /// VMware SVGA II proxy (display_server only).
    SvgaGetInfo = 127,
    SvgaUpdate = 128,
    SvgaSetMode = 129,
    /// Public system identity (no serial/UUID). Gated to thermald/deviced/UI.
    SystemIdentity = 133,
    /// Thermal sensor snapshot (read-only). Gated to thermald/UI.
    ThermalSensors = 134,
    /// Mezzo-only: consume a UAC-minted authentication grant bound to a presenter PID.
    LockAuthConsume = 135,
    /// Display-only validation of the kernel-marked Mezzo process identity.
    ValidateLockServicePid = 136,
    /// Mezzo-only validation of trusted session and authenticated TTY callers.
    ValidateLockCaller = 137,
    DebugLog = 99,
    /// Trusted PTY service credential lookup for an IPC caller PID.
    PtyGetCredentials = 103,
}

/// Public SMBIOS-derived system identity (never includes serial/UUID).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SystemIdentityRecord {
    pub manufacturer: [u8; 64],
    pub product_name: [u8; 64],
    pub product_version: [u8; 64],
    pub board_manufacturer: [u8; 64],
    pub board_product: [u8; 64],
    pub bios_vendor: [u8; 64],
    pub bios_version: [u8; 64],
    pub smbios_major: u8,
    pub smbios_minor: u8,
    pub identity_confidence: u8,
    pub ready: u8,
    pub _pad: [u8; 4],
}

impl SystemIdentityRecord {
    pub const fn empty() -> Self {
        Self {
            manufacturer: [0; 64],
            product_name: [0; 64],
            product_version: [0; 64],
            board_manufacturer: [0; 64],
            board_product: [0; 64],
            bios_vendor: [0; 64],
            bios_version: [0; 64],
            smbios_major: 0,
            smbios_minor: 0,
            identity_confidence: 0,
            ready: 0,
            _pad: [0; 4],
        }
    }

    pub fn field_str(field: &[u8; 64]) -> &str {
        let len = field.iter().position(|&b| b == 0).unwrap_or(field.len());
        core::str::from_utf8(&field[..len]).unwrap_or("")
    }

    pub fn manufacturer_str(&self) -> &str {
        Self::field_str(&self.manufacturer)
    }

    pub fn product_name_str(&self) -> &str {
        Self::field_str(&self.product_name)
    }
}

/// One thermal sensor record exported by the kernel (read-only).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ThermalSensorRecord {
    pub id: u32,
    pub class: u8,
    pub label: u8,
    pub location: u8,
    pub unit: u8,
    pub scope: u8,
    pub source: u8,
    pub read_only: u8,
    pub status: u8,
    pub _pad: [u8; 3],
    pub value: i32,
    pub mono_ms: u64,
}

impl ThermalSensorRecord {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            class: 0,
            label: 0,
            location: 0,
            unit: 0,
            scope: 0,
            source: 0,
            read_only: 1,
            status: 2, // Unavailable
            _pad: [0; 3],
            value: 0,
            mono_ms: 0,
        }
    }

    /// Temperature only when status is Valid (1). Never treat Unavailable as 0°C.
    pub fn temp_milli_c(self) -> Option<i32> {
        if self.status == 1 {
            Some(self.value)
        } else {
            None
        }
    }
}

/// System statistics filled by the SysInfo syscall (kernel writes four u64s).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemInfo {
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    pub uptime_secs: u64,
    pub unix_time: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    pub swap_compressed_kb: u64,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityToken(pub u64);

impl CapabilityToken {
    pub const INVALID: Self = Self(0);
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointId(pub u64);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IpcMsg {
    pub label: u64,
    pub badge: u64,
    pub word_count: u32,
    pub cap_count: u32,
    pub words: [u64; IPC_MAX_WORDS],
    pub caps: [CapabilityToken; IPC_MAX_CAPS],
}

impl IpcMsg {
    pub const fn empty() -> Self {
        Self {
            label: 0,
            badge: 0,
            word_count: 0,
            cap_count: 0,
            words: [0; IPC_MAX_WORDS],
            caps: [CapabilityToken::INVALID; IPC_MAX_CAPS],
        }
    }

    pub const fn with_label(label: u64) -> Self {
        Self {
            label,
            badge: 0,
            word_count: 0,
            cap_count: 0,
            words: [0; IPC_MAX_WORDS],
            caps: [CapabilityToken::INVALID; IPC_MAX_CAPS],
        }
    }

    pub fn word(mut self, idx: usize, val: u64) -> Self {
        if idx < IPC_MAX_WORDS {
            self.words[idx] = val;
            let count = (idx + 1) as u32;
            if self.word_count < count {
                self.word_count = count;
            }
        }
        self
    }

    pub fn with_cap(mut self, idx: usize, val: CapabilityToken) -> Self {
        if idx < IPC_MAX_CAPS {
            self.caps[idx] = val;
            let count = (idx + 1) as u32;
            if self.cap_count < count {
                self.cap_count = count;
            }
        }
        self
    }
}

#[allow(non_snake_case)]
pub mod InitMsg {
    pub const REGISTER: u64 = 1;
    pub const LOOKUP: u64 = 2;
    pub const GRANT: u64 = 3;
    pub const DENY: u64 = 4;
}

#[allow(non_snake_case)]
pub mod InitStatus {
    pub const OK: u64 = 0;
    pub const INVALID_REGISTRATION: u64 = 1;
    pub const LIVE_NAME_CONFLICT: u64 = 2;
    pub const REGISTRY_FULL: u64 = 3;
    pub const NOT_FOUND: u64 = 4;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ServiceCapability {
    Network = 0,
    Authentication = 1,
    Pty = 2,
    SpawnUser = 3,
    ConfigRead = 4,
    HostKeyAdmin = 5,
    ServiceLifecycle = 6,
    Logging = 7,
    SecureRandom = 8,
    MonotonicTime = 9,
    Vfs = 10,
    KvStore = 11,
    StorageAdmin = 12,
    Display = 13,
    Tty = 14,
    SchedulerControl = 15,
    TimeSync = 16,
    Resolver = 17,
    /// User-facing broker set inherited by an authenticated interactive
    /// session and its descendants. This deliberately excludes system/service
    /// administration endpoints such as spawn, sm, deviced, and sunlightd.
    UserSession = 18,
    /// Administrative access to the networkd control plane. This is separate
    /// from `Network`, which grants access only to the packet/socket service.
    NetworkControl = 19,
    /// Administrative access to the deviced registry.
    DeviceControl = 20,
    /// Administrative access to the powerd policy service.
    PowerControl = 21,
    /// Administrative access to the thermal policy service (thermald).
    ThermalControl = 22,
    SessionLock = 23,
}

impl ServiceCapability {
    pub const fn bit(self) -> u64 {
        1u64 << (self as u8)
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "network" => Some(Self::Network),
            "authentication" => Some(Self::Authentication),
            "pty" => Some(Self::Pty),
            "spawn-user" => Some(Self::SpawnUser),
            "config-read" => Some(Self::ConfigRead),
            "host-key-admin" => Some(Self::HostKeyAdmin),
            "service-lifecycle" => Some(Self::ServiceLifecycle),
            "logging" => Some(Self::Logging),
            "secure-random" => Some(Self::SecureRandom),
            "monotonic-time" => Some(Self::MonotonicTime),
            "vfs" => Some(Self::Vfs),
            "kv-store" => Some(Self::KvStore),
            "storage-admin" => Some(Self::StorageAdmin),
            "display" => Some(Self::Display),
            "tty" => Some(Self::Tty),
            "scheduler-control" => Some(Self::SchedulerControl),
            "time-sync" => Some(Self::TimeSync),
            "resolver" => Some(Self::Resolver),
            "user-session" => Some(Self::UserSession),
            "network-control" => Some(Self::NetworkControl),
            "device-control" => Some(Self::DeviceControl),
            "power-control" => Some(Self::PowerControl),
            "thermal-control" => Some(Self::ThermalControl),
            "session-lock" => Some(Self::SessionLock),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Authentication => "authentication",
            Self::Pty => "pty",
            Self::SpawnUser => "spawn-user",
            Self::ConfigRead => "config-read",
            Self::HostKeyAdmin => "host-key-admin",
            Self::ServiceLifecycle => "service-lifecycle",
            Self::Logging => "logging",
            Self::SecureRandom => "secure-random",
            Self::MonotonicTime => "monotonic-time",
            Self::Vfs => "vfs",
            Self::KvStore => "kv-store",
            Self::StorageAdmin => "storage-admin",
            Self::Display => "display",
            Self::Tty => "tty",
            Self::SchedulerControl => "scheduler-control",
            Self::TimeSync => "time-sync",
            Self::Resolver => "resolver",
            Self::UserSession => "user-session",
            Self::NetworkControl => "network-control",
            Self::DeviceControl => "device-control",
            Self::PowerControl => "power-control",
            Self::ThermalControl => "thermal-control",
            Self::SessionLock => "session-lock",
        }
    }
}

pub const ALL_SERVICE_CAPABILITIES: [ServiceCapability; 24] = [
    ServiceCapability::Network,
    ServiceCapability::Authentication,
    ServiceCapability::Pty,
    ServiceCapability::SpawnUser,
    ServiceCapability::ConfigRead,
    ServiceCapability::HostKeyAdmin,
    ServiceCapability::ServiceLifecycle,
    ServiceCapability::Logging,
    ServiceCapability::SecureRandom,
    ServiceCapability::MonotonicTime,
    ServiceCapability::Vfs,
    ServiceCapability::KvStore,
    ServiceCapability::StorageAdmin,
    ServiceCapability::Display,
    ServiceCapability::Tty,
    ServiceCapability::SchedulerControl,
    ServiceCapability::TimeSync,
    ServiceCapability::Resolver,
    ServiceCapability::UserSession,
    ServiceCapability::NetworkControl,
    ServiceCapability::DeviceControl,
    ServiceCapability::PowerControl,
    ServiceCapability::ThermalControl,
    ServiceCapability::SessionLock,
];

pub fn service_capability_mask_to_names(mask: u64) -> impl Iterator<Item = &'static str> + Clone {
    ALL_SERVICE_CAPABILITIES
        .into_iter()
        .filter(move |capability| mask & capability.bit() != 0)
        .map(ServiceCapability::as_str)
}

pub fn service_capability_allows_hashed_name(mask: u64, name_key: u64) -> bool {
    if mask & ServiceCapability::UserSession.bit() != 0 && matches_user_session_service(name_key) {
        return true;
    }
    if mask & ServiceCapability::Network.bit() != 0 && name_key == name_to_u64("net") {
        return true;
    }
    if mask & ServiceCapability::Authentication.bit() != 0 && name_key == name_to_u64("uac") {
        return true;
    }
    if mask & ServiceCapability::Pty.bit() != 0 && name_key == name_to_u64("pty") {
        return true;
    }
    if mask & ServiceCapability::SpawnUser.bit() != 0 && name_key == name_to_u64("spawn") {
        return true;
    }
    if mask & ServiceCapability::ConfigRead.bit() != 0 && name_key == name_to_u64("vfs") {
        return true;
    }
    if mask & ServiceCapability::HostKeyAdmin.bit() != 0 && name_key == name_to_u64("vfs") {
        return true;
    }
    if mask & ServiceCapability::ServiceLifecycle.bit() != 0 && name_key == name_to_u64("sunlightd")
    {
        return true;
    }
    if mask & ServiceCapability::SecureRandom.bit() != 0 && name_key == name_to_u64("rand") {
        return true;
    }
    if mask & ServiceCapability::Vfs.bit() != 0 && name_key == name_to_u64("vfs") {
        return true;
    }
    if mask & ServiceCapability::KvStore.bit() != 0 && name_key == name_to_u64("sunlight-kv") {
        return true;
    }
    if mask & ServiceCapability::StorageAdmin.bit() != 0 && name_key == name_to_u64("sm") {
        return true;
    }
    if mask & ServiceCapability::Display.bit() != 0 && name_key == name_to_u64("display_server") {
        return true;
    }
    if mask & ServiceCapability::Tty.bit() != 0 && name_key == name_to_u64("tty") {
        return true;
    }
    if mask & ServiceCapability::SchedulerControl.bit() != 0
        && (name_key == name_to_u64("gcd") || name_key == name_to_u64("niced"))
    {
        return true;
    }
    if mask & ServiceCapability::TimeSync.bit() != 0
        && (name_key == name_to_u64("timed") || name_key == name_to_u64("tz"))
    {
        return true;
    }
    if mask & ServiceCapability::Resolver.bit() != 0 && name_key == name_to_u64("resolved") {
        return true;
    }
    if mask & ServiceCapability::NetworkControl.bit() != 0 && name_key == name_to_u64("networkd") {
        return true;
    }
    if mask & ServiceCapability::DeviceControl.bit() != 0 && name_key == name_to_u64("deviced") {
        return true;
    }
    if mask & ServiceCapability::PowerControl.bit() != 0 && name_key == name_to_u64("powerd") {
        return true;
    }
    if mask & ServiceCapability::ThermalControl.bit() != 0 && name_key == name_to_u64("thermald")
    {
        return true;
    }
    if mask & ServiceCapability::SessionLock.bit() != 0 && name_key == name_to_u64("mezzo") {
        return true;
    }
    false
}

/// Brokers ordinary interactive applications may resolve. Keep privileged
/// control-plane services out of this list: notably spawn, sm, deviced,
/// sunlightd, networkd, powerd, thermald, gcd/proc, niced, and timed.
fn matches_user_session_service(name_key: u64) -> bool {
    name_key == name_to_u64("vfs")
        || name_key == name_to_u64("display_server")
        || name_key == name_to_u64("net")
        || name_key == name_to_u64("resolved")
        || name_key == name_to_u64("pty")
        || name_key == name_to_u64("tty")
        || name_key == name_to_u64("uac")
        || name_key == name_to_u64("rand")
        || name_key == name_to_u64("tz")
        || name_key == name_to_u64("clipd")
        || name_key == name_to_u64("dialogd")
        || name_key == name_to_u64("thumbd")
        || name_key == name_to_u64("sunlight-kv")
        || name_key == name_to_u64("sunlight-tls")
        || name_key == name_to_u64("mezzo")
}

#[cfg(test)]
mod service_capability_tests {
    use super::{name_to_u64, service_capability_allows_hashed_name, ServiceCapability};

    #[test]
    fn control_plane_capabilities_are_service_specific() {
        let network = ServiceCapability::NetworkControl.bit();
        assert!(service_capability_allows_hashed_name(
            network,
            name_to_u64("networkd")
        ));
        assert!(!service_capability_allows_hashed_name(
            network,
            name_to_u64("deviced")
        ));

        let device = ServiceCapability::DeviceControl.bit();
        assert!(service_capability_allows_hashed_name(
            device,
            name_to_u64("deviced")
        ));
        assert!(!service_capability_allows_hashed_name(
            device,
            name_to_u64("powerd")
        ));

        let power = ServiceCapability::PowerControl.bit();
        assert!(service_capability_allows_hashed_name(
            power,
            name_to_u64("powerd")
        ));
        assert!(!service_capability_allows_hashed_name(
            power,
            name_to_u64("networkd")
        ));
    }

    #[test]
    fn ordinary_user_session_still_cannot_resolve_control_plane_services() {
        let session = ServiceCapability::UserSession.bit();
        for name in [
            "networkd",
            "deviced",
            "powerd",
            "thermald",
            "sunlightd",
            "niced",
        ] {
            assert!(!service_capability_allows_hashed_name(
                session,
                name_to_u64(name)
            ));
        }
        assert!(service_capability_allows_hashed_name(
            session,
            name_to_u64("net")
        ));
    }

    #[test]
    fn thermal_control_resolves_only_thermald() {
        let thermal = ServiceCapability::ThermalControl.bit();
        assert!(service_capability_allows_hashed_name(
            thermal,
            name_to_u64("thermald")
        ));
        assert!(!service_capability_allows_hashed_name(
            thermal,
            name_to_u64("powerd")
        ));
    }
}

#[allow(non_snake_case)]
pub mod NameserverDiagnosticEvent {
    pub const LIVE_CONFLICT: u64 = 1;
    pub const STALE_REMOVAL: u64 = 2;
    pub const DEAD_REPLACEMENT: u64 = 3;
    pub const STALE_LOOKUP: u64 = 4;
    pub const REGISTRY_FULL: u64 = 5;
}

#[allow(non_snake_case)]
pub mod TimerMsg {
    pub const TICK: u64 = 1;
    pub const GET_TICKS: u64 = 2;
    pub const REPLY: u64 = 3;
    pub const ERROR: u64 = 4;
}

#[allow(non_snake_case)]
pub mod TimeMsg {
    pub const GET_TIME: u64 = 1; // Query current UTC time
    pub const GET_STATE: u64 = 2; // Get full TimeState (back-compat: tz fields are 0)
    pub const SET_TIMEZONE: u64 = 3; // No-op (timezone moved to "tz")
    pub const SYNC_NTP: u64 = 4; // Trigger NTP sync
    pub const GET_UTC: u64 = 5; // Preferred alias for GET_TIME (pure UTC)
    pub const REPLY: u64 = 100;
    pub const ERROR: u64 = 101;
}

/// Timezone service opcodes (registered as "tz")
#[allow(non_snake_case)]
pub mod TzMsg {
    pub const GET_LOCAL_TIME: u64 = 0x7001;
    pub const GET_ZONE: u64 = 0x7002;
    pub const SET_ZONE: u64 = 0x7003; // arg: zone id in data[0..64]
    pub const LIST_ZONES: u64 = 0x7004; // arg: page in word(0), 8 per page (but one zone per reply for packing)
    pub const NOTIFY_CHANGED: u64 = 0x7005; // sent TO timed after SET_ZONE (best effort)
    pub const REPLY: u64 = 0x70FF;
    pub const ERROR: u64 = 0x70FE;
}

/// Minimal NetOp constants for cross-service queries (e.g. networkd ingesting
/// current addressing from net_server without taking a direct dep on sunlight-net).
#[allow(non_snake_case)]
pub mod NetOp {
    pub const GETIP: u64 = 10;
    pub const GET_BACKEND: u64 = 16;
    /// Return the executing net stack's current lease without consulting
    /// networkd policy.  This breaks the otherwise circular networkd -> net
    /// -> networkd refresh path.
    pub const GETIP_LIVE: u64 = 19;
}

/// Random service opcodes (registered as "rand").
///
/// The crypto path is intentionally chunked: each GET asks for up to 32 bytes
/// (`words[0..3]`, the real register-IPC inline budget — see `raw_syscall_ipc`)
/// and the reply packs that many bytes back into `words[0..3]`. Callers wanting
/// more than 32 bytes loop. This avoids any shared-memory cap-transfer.
#[allow(non_snake_case)]
pub mod RandMsg {
    /// Request random bytes. `words[0]` = requested length (clamped to 32).
    pub const GET: u64 = 0x7201;
    /// Non-sensitive service statistics (additive; existing clients ignore).
    ///
    /// Reply `words` layout on success (`REPLY`):
    /// * `[0]` ready (0/1)
    /// * `[1]` total_requests
    /// * `[2]` total_bytes_generated
    /// * `[3]` packed: reseed_count | entropy_failures<<16 | rejected<<32 | not_ready<<48
    /// * `[4]` last_reseed_reason (enum discriminant)
    /// * `[5]` block_bytes | (reseed_block_threshold << 32)
    ///
    /// Never carries seeds, raw entropy, keys, or random output.
    pub const STATS: u64 = 0x7202;
    /// Reply carrying exactly the requested byte count in `words[0..3]`.
    pub const REPLY: u64 = 0x72FF;
    pub const ERROR: u64 = 0x72FE;
    /// Maximum bytes returned per GET (4 transiting words).
    pub const MAX_CHUNK: usize = 32;
}

/// PTY service opcodes (registered as "pty").
#[allow(non_snake_case)]
pub mod PtyMsg {
    pub const CREATE: u64 = 0x7301;
    pub const READ_MASTER: u64 = 0x7302;
    pub const WRITE_MASTER: u64 = 0x7303;
    pub const READ_SLAVE: u64 = 0x7304;
    pub const WRITE_SLAVE: u64 = 0x7305;
    pub const SET_MODE: u64 = 0x7306;
    /// Control-authorized delegation of the slave endpoint to a spawned PID.
    pub const ATTACH_SLAVE: u64 = 0x7307;
    pub const SET_WINDOW_SIZE: u64 = 0x7308;
    pub const GET_WINDOW_SIZE: u64 = 0x7309;
    pub const SET_FOREGROUND_PROCESS: u64 = 0x730A;
    pub const GET_FOREGROUND_PROCESS: u64 = 0x730B;
    pub const CLOSE_MASTER: u64 = 0x730C;
    pub const CLOSE_SLAVE: u64 = 0x730D;
    pub const CLOSE_SESSION: u64 = 0x730E;
    pub const GET_STATE: u64 = 0x730F;
    pub const REPLY: u64 = 0x73FF;
    pub const ERROR: u64 = 0x73FE;
    /// Bit 0: canonical/cooked mode.
    /// Bit 1: local echo.
    pub const FLAG_CANONICAL: u64 = 1 << 0;
    pub const FLAG_ECHO: u64 = 1 << 1;

    pub const ERR_INVALID_PTY: u64 = 1;
    pub const ERR_STALE_HANDLE: u64 = 2;
    pub const ERR_PERMISSION_DENIED: u64 = 3;
    pub const ERR_WRONG_ENDPOINT_ROLE: u64 = 4;
    pub const ERR_NO_SLOTS: u64 = 5;
    pub const ERR_WOULD_BLOCK: u64 = 6;
    pub const ERR_PEER_CLOSED: u64 = 7;
    pub const ERR_SESSION_CLOSING: u64 = 8;
    pub const ERR_INVALID_WINDOW_SIZE: u64 = 9;
    pub const ERR_BUFFER_FULL: u64 = 10;
    pub const ERR_SERVICE_UNAVAILABLE: u64 = 11;
    pub const ERR_INTERNAL: u64 = 12;

    pub const STATE_MASTER_OPEN: u64 = 1 << 0;
    pub const STATE_SLAVE_OPEN: u64 = 1 << 1;
    pub const STATE_CONTROL_OPEN: u64 = 1 << 2;
    pub const STATE_CLOSING: u64 = 1 << 3;
}

#[allow(non_snake_case)]
pub mod VfsMsg {
    pub const OPEN: u64 = 1;
    pub const READ: u64 = 2;
    pub const CLOSE: u64 = 3;
    pub const STAT: u64 = 4;
    pub const REPLY: u64 = 5;
    pub const ERROR: u64 = 6;
    pub const WRITE: u64 = 7;
    pub const MKDIR: u64 = 8;
    pub const CHMOD: u64 = 9;
    pub const CHOWN: u64 = 10;
    pub const GETPWNAM: u64 = 11; // Get user info by username
    pub const GETGRGID: u64 = 12; // Get group info by gid
    pub const GETPWUID: u64 = 13; // Get user info by uid
    pub const FSTAT: u64 = 14; // Stat an open file handle
    pub const UNLINK: u64 = 15; // Remove a file (path in words[0..3])
    pub const RENAME: u64 = 16; // Rename: src path words[0..3], dst path words[4..7]
    pub const DATA_SHARED: u64 = 31; // large read reply carries cap in caps[0]
}

/// Thumbnail daemon protocol (`sunlight-thumbd`).
///
/// Chunked-path, request/ack style IPC. Because register IPC only carries
/// 4 × 8 = 32 bytes of payload, paths longer than 24 bytes are split across
/// multiple messages. The File Manager sends PATH_CHUNK messages followed by
/// exactly one PATH_END message. thumbd ACKs each message immediately, then
/// generates the thumbnail asynchronously.
///
/// PATH_CHUNK (label 0x7411):
///   words[0..3] = 32 bytes of source path (packed, zero-padded)
///
/// PATH_END (label 0x7412):
///   words[0..2] = up to 24 remaining path bytes (zero-padded)
///   words[3]    = (path_total_len as u32) | ((size as u32) << 32)
///                 where size: 0 = SIZE_NORMAL (128 px), 1 = SIZE_LARGE (256 px)
///
/// After the last PATH_END for a given sender PID, thumbd resolves the full
/// path (assembled from chunks + tail) and enqueues thumbnail generation.
/// The File Manager polls the on-disk cache path on subsequent repaints.
///
/// Cache location: `<home>/.cache/sunlightos/thumbs/{normal,large}/<hex_key>.simg`
///   where hex_key = hex(fnv1a_64(path || le64(file_size) || u8(size)))
///
/// Thumb sizes: `SIZE_NORMAL` = 128px, `SIZE_LARGE` = 256px.
pub mod ThumbMsg {
    /// Non-final path chunk (32 bytes in words[0..3]).
    pub const PATH_CHUNK: u64 = 0x7411;
    /// Final path chunk + metadata. After this message thumbd has the full path.
    pub const PATH_END: u64 = 0x7412;
    /// Immediate ACK from thumbd — generation will happen asynchronously.
    pub const REPLY: u64 = 0x74FF;
    /// Returned if the request is malformed or the source file is missing.
    pub const ERROR: u64 = 0x74FE;

    /// Back-compat alias (single-message short path, equivalent to PATH_END
    /// with words[0..2] as path bytes and words[3] as len|size).
    pub const REQUEST: u64 = PATH_END;

    /// `size` field value in PATH_END words[3] selecting the 128×128 thumbnail.
    pub const SIZE_NORMAL: u64 = 0;
    /// `size` field value in PATH_END words[3] selecting the 256×256 thumbnail.
    pub const SIZE_LARGE: u64 = 1;

    /// Maximum supported thumbnail edge length (large).
    pub const MAX_EDGE: u32 = 256;

    /// Bytes of path payload per PATH_CHUNK message.
    pub const CHUNK_BYTES: usize = 32;
    /// Bytes of path payload in the tail portion of a PATH_END message.
    pub const END_PATH_BYTES: usize = 24;
}

/// Sunlight Graphics Protocol (SGP) opcodes for the display / compositor service.
/// Registered under nameserver name "display_server".
/// See docs/GUI/INITIALIZE_PHASE... for full spec.
pub mod sgp {
    #[allow(non_snake_case)]
    pub mod SgpMsg {
        // Client -> Display Server Requests
        pub const CREATE_WINDOW: u64 = 0xA101;
        pub const COMMIT_FRAME: u64 = 0xA102;
        pub const EVENT_POLL: u64 = 0xA103;
        /// EVENT_POLL reply words[3] low byte is the physical button mask.
        /// These upper bits describe delivery ownership without exposing any
        /// compositor object or desktop-global pointer handle.
        pub const EVENT_FLAG_FOCUSED: u64 = 1 << 8;
        pub const EVENT_FLAG_POINTER_OWNED: u64 = 1 << 9;
        pub const EVENT_FLAG_POINTER_CAPTURED: u64 = 1 << 10;
        /// Focus was acquired by the physical button transition represented in
        /// this same poll (as opposed to keyboard/window-manager focus).
        pub const EVENT_FLAG_FOCUS_PRESS: u64 = 1 << 11;
        /// The requested window ID resolved to a live compositor window. A
        /// reply without this flag is an explicit stale/wrong-window result.
        pub const EVENT_FLAG_WINDOW_VALID: u64 = 1 << 12;
        /// EVENT_POLL reply words[3] bits 16..23 contain the active workspace id.
        pub const EVENT_DESKTOP_ACTIVE_WORKSPACE_SHIFT: u32 = 16;
        pub const EVENT_DESKTOP_ACTIVE_WORKSPACE_MASK: u64 =
            0xFF << EVENT_DESKTOP_ACTIVE_WORKSPACE_SHIFT;
        /// EVENT_POLL reply words[3] bit 24 indicates that a visible maximized
        /// normal window is driving the shell's integrated top-panel mode.
        pub const EVENT_DESKTOP_INTEGRATED_PANEL: u64 = 1 << 24;
        /// Normal window lifecycle cleanup. Clients should send this before
        /// releasing their local SHM mapping so the compositor can drop its
        /// window metadata and display-side SHM ownership.
        pub const CLOSE_WINDOW: u64 = 0xA104;
        /// Back-compat alias for older callers.
        pub const DESTROY_WINDOW: u64 = CLOSE_WINDOW;
        /// Update window properties post-creation.
        /// words[0]=win_id, words[1]=config_flags, words[2]=pid|ppid<<32,
        /// words[3]=title_bytes[0..8]; SHM cap at caps[0] for full title (optional).
        pub const CONFIGURE_WINDOW: u64 = 0xA105;
        /// Client requests a cursor shape when its pointer is inside its client area.
        /// words[0] = (win_id as u32) | ((CursorShape discriminant as u32) << 32)
        pub const SET_CURSOR: u64 = 0xA106;
        /// Query compositor display metrics before creating a window.
        /// No words required.
        /// Reply:
        ///   words[0] = width | (height << 32)
        ///   words[1] = stride_bytes | (pixel_format << 32)
        ///   words[2] = scale_fp | (backend << 32)   // scale_fp: 65536 = 1.0×
        ///   words[3] = refresh_hz (0 when unknown)
        pub const GET_SCREEN_INFO: u64 = 0xA107;
        /// Show a transient desktop notification.
        /// words[0] = NotificationKind discriminant
        /// words[1] = timeout_ms
        /// caps[0]  = SHM page containing the notification payload
        pub const SHOW_NOTIFICATION: u64 = 0xA108;
        /// Update pointer motion settings at runtime (from Control Panel or similar).
        /// words[0] = sensitivity_fp (i32 cast to u64; FP_ONE=65536 → 1.0×)
        /// words[1] = acceleration_enabled (0 = off, 1 = on)
        /// Reply: SgpMsg::REPLY with words[0]=0 on success.
        pub const SET_MOUSE_SETTINGS: u64 = 0xA109;
        /// Enumerate compositor-managed windows.
        /// words[0] = window index to query (0-based, top-level order)
        /// Reply: words[0]=win_id (0 when no more windows),
        ///        words[1]=owner_pid,
        ///        words[2]=window_state,
        ///        words[3]=window_type(low byte) | rolled_up(bit 8)
        ///                | hidden(bit 9)
        ///                | active_workspace_id(bits 16..23)
        ///                | window_workspace_id(bits 24..31),
        ///        words[6..7]=first 16 bytes of the window title (LE packed).
        pub const LIST_WINDOWS: u64 = 0xA10A;
        pub const LIST_WINDOW_ROLLED_UP: u64 = 1 << 8;
        pub const LIST_WINDOW_HIDDEN: u64 = 1 << 9;
        pub const LIST_ACTIVE_WORKSPACE_SHIFT: u32 = 16;
        pub const LIST_ACTIVE_WORKSPACE_MASK: u64 = 0xFF << LIST_ACTIVE_WORKSPACE_SHIFT;
        pub const LIST_WINDOW_WORKSPACE_SHIFT: u32 = 24;
        pub const LIST_WINDOW_WORKSPACE_MASK: u64 = 0xFF << LIST_WINDOW_WORKSPACE_SHIFT;
        /// Bring the window to the front and restore it if minimized.
        /// words[0] = win_id
        pub const ACTIVATE_WINDOW: u64 = 0xA10B;
        /// Register launch-trace metadata for a child pid.
        /// words[0] = launch_id, words[1] = source, words[2] = pid, words[3] = requested_at_ms
        pub const LAUNCH_TRACE: u64 = 0xA10C;
        /// Set the active workspace (1..=4). words[0] = workspace id.
        /// Windows whose workspace_id differs become invisible and non-interactive.
        /// Desktop/Widget windows remain visible on all workspaces.
        /// Reply: SgpMsg::REPLY.
        pub const SET_WORKSPACE: u64 = 0xA10D;
        pub const GET_DISPLAY_MODE_CAPABILITIES: u64 = 0xA112;
        pub const GET_DISPLAY_MODE: u64 = 0xA113;
        pub const BEGIN_DISPLAY_MODE_CHANGE: u64 = 0xA114;
        pub const CONFIRM_DISPLAY_MODE_CHANGE: u64 = 0xA115;
        pub const REVERT_DISPLAY_MODE_CHANGE: u64 = 0xA116;
        pub const ATTACH_DISPLAY_MODE_DIALOG: u64 = 0xA117;
        pub const LOCK_ENTER: u64 = 0xA118;
        pub const LOCK_REGISTER_PRESENTER: u64 = 0xA119;
        pub const LOCK_PRESENTER_READY: u64 = 0xA11A;
        pub const LOCK_LEAVE: u64 = 0xA11B;
        pub const LOCK_STATUS: u64 = 0xA11C;

        // Session control — sent by tty_server to coordinate framebuffer ownership.
        // words[0] = 0 (reserved)
        pub const SESSION_ACTIVATE: u64 = 0xA110; // Desktop session takes framebuffer
        pub const SESSION_DEACTIVATE: u64 = 0xA111; // TTY session takes framebuffer

        // Display Server -> Client Replies
        pub const REPLY: u64 = 0xA1FF;

        // --- config_flags word bit layout (words[1] of CREATE_WINDOW / CONFIGURE_WINDOW) ---
        // bits [1:0]  window_type  (0=Normal 1=Dialog 2=Desktop 3=Widget)
        // bits [3:2]  state        (0=Normal 1=Minimized 2=Maximized 3=Fullscreen)
        // bit  [4]    border       (0=Full 1=None)
        // bit  [5]    z_index_type (0=Normal 1=OnTop)
        // bits [12:6] z_index_value (1-100; 0 → use default 50)
        // bits [14:13] show_type   (0=Floating 1=Tiled 2=Scrolling)
        // bits [16:15] group_type  (0=None 1=Stacked 2=Tabbed)
        // bits [18:17] decoration  (0=Normal 1=CompactClose 2=CompactCloseMinimize
        //                           3=HiddenOverlay)
        // bits [20:19] surface material
        //   0 = OpaqueXrgb (default)
        //   1 = reserved legacy WindowGlass — accepted for compatibility;
        //       compositor maps to opaque chrome + straight-alpha client pixels
        //       (not backdrop blur / acrylic)
        // bits [63:21] reserved
        pub mod config_flags {
            pub const WIN_TYPE_MASK: u64 = 0x3;
            pub const STATE_MASK: u64 = 0x3 << 2;
            pub const STATE_SHIFT: u64 = 2;
            pub const BORDER_NONE: u64 = 1 << 4;
            pub const Z_ON_TOP: u64 = 1 << 5;
            pub const Z_VAL_MASK: u64 = 0x7F << 6;
            pub const Z_VAL_SHIFT: u64 = 6;
            pub const SHOW_TYPE_MASK: u64 = 0x3 << 13;
            pub const SHOW_TYPE_SHIFT: u64 = 13;
            pub const GROUP_TYPE_MASK: u64 = 0x3 << 15;
            pub const GROUP_TYPE_SHIFT: u64 = 15;
            pub const DECORATION_MASK: u64 = 0x3 << 17;
            pub const DECORATION_SHIFT: u64 = 17;
            /// Explicit material field. Default remains opaque XRGB; the
            /// compositor must never infer transparency from pixel colors.
            pub const MATERIAL_MASK: u64 = 0x3 << 19;
            pub const MATERIAL_SHIFT: u64 = 19;
            /// Reserved legacy value. Kept so old clients still connect; the
            /// display service treats it as opaque chrome with straight-alpha
            /// client composition, not as a translucent glass/blur effect.
            pub const MATERIAL_WINDOW_GLASS: u64 = 0x1 << MATERIAL_SHIFT;
        }
    }
}

pub mod launch_trace;

// For convenience, also re-export at top level (some code uses SgpMsg directly)
pub use sgp::SgpMsg;

/// Notification severity used by the compositor overlay.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Info = 0,
    Warning = 1,
    Error = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationPriority {
    Low,
    Normal,
    High,
    Critical,
}

const KV_REPLY: u64 = 0x4BFF;
const KV_ERROR: u64 = 0x4BEE;
const KV_VALUE: u64 = 0x4B05;
const KV_PUT_SHM2: u64 = 0x4B08;
const KV_GET_SHM2: u64 = 0x4B09;
pub const NOTIFICATION_RECENT_KEY: &str = "notifications/index/recent";
pub const NOTIFICATION_DND_KEY: &str = "notifications/config/dnd";
const NOTIFICATION_RECENT_LIMIT: usize = 64;
const NOTIFICATION_FIELD_MAX: usize = 256;
static mut NOTIFICATION_ID_COUNTER: u64 = 1;

#[repr(C)]
struct NotificationWire {
    kind: u8,
    _pad0: [u8; 7],
    timeout_ms: u64,
    title_len: u32,
    body_len: u32,
    title: [u8; 96],
    body: [u8; 256],
}

impl NotificationWire {
    fn new(kind: NotificationKind, title: &str, body: &str, timeout_ms: u64) -> Self {
        let mut wire = Self {
            kind: kind as u8,
            _pad0: [0; 7],
            timeout_ms,
            title_len: 0,
            body_len: 0,
            title: [0; 96],
            body: [0; 256],
        };

        let title_bytes = title.as_bytes();
        let body_bytes = body.as_bytes();
        let title_len = title_bytes.len().min(wire.title.len());
        let body_len = body_bytes.len().min(wire.body.len());
        wire.title[..title_len].copy_from_slice(&title_bytes[..title_len]);
        wire.body[..body_len].copy_from_slice(&body_bytes[..body_len]);
        wire.title_len = title_len as u32;
        wire.body_len = body_len as u32;
        wire
    }
}

fn notification_priority_for_kind(kind: NotificationKind) -> NotificationPriority {
    match kind {
        NotificationKind::Info => NotificationPriority::Normal,
        NotificationKind::Warning => NotificationPriority::High,
        NotificationKind::Error => NotificationPriority::Critical,
    }
}

fn notification_priority_str(priority: NotificationPriority) -> &'static str {
    match priority {
        NotificationPriority::Low => "low",
        NotificationPriority::Normal => "normal",
        NotificationPriority::High => "high",
        NotificationPriority::Critical => "critical",
    }
}

fn notification_kv_cap() -> Option<CapabilityToken> {
    nameserver_lookup_timeout("sunlight-kv", 100)
}

pub fn notification_kv_put(key: &str, value: &[u8]) -> bool {
    if key.len() > SHM_PAGE || value.len() > SHM_PAGE {
        return false;
    }
    let Some(cap) = notification_kv_cap() else {
        return false;
    };
    let Ok((key_ptr, key_tok)) = shm_create(SHM_PAGE, 0) else {
        return false;
    };
    let Ok((value_ptr, value_tok)) = shm_create(SHM_PAGE, 0) else {
        let _ = shm_free(key_tok);
        return false;
    };
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
        core::ptr::copy_nonoverlapping(value.as_ptr(), value_ptr, value.len());
    }
    let msg = IpcMsg::with_label(KV_PUT_SHM2)
        .word(0, key.len() as u64)
        .word(1, value.len() as u64)
        .with_cap(0, key_tok)
        .with_cap(1, value_tok);
    let ok = matches!(
        ipc_call_timeout(cap, msg, 100),
        Ok(reply) if reply.label == KV_REPLY && reply.words[0] == 0
    );
    let _ = shm_free(key_tok);
    let _ = shm_free(value_tok);
    ok
}

pub fn notification_kv_get_into(key: &str, out: &mut [u8]) -> Option<usize> {
    if key.len() > SHM_PAGE {
        return None;
    }
    let cap = notification_kv_cap()?;
    let Ok((key_ptr, key_tok)) = shm_create(SHM_PAGE, 0) else {
        return None;
    };
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
    }
    let msg = IpcMsg::with_label(KV_GET_SHM2)
        .word(0, key.len() as u64)
        .with_cap(0, key_tok);
    let reply_res = ipc_call_timeout(cap, msg, 100);
    let _ = shm_free(key_tok);
    let reply = reply_res.ok()?;
    if reply.label == KV_ERROR || reply.label != KV_VALUE {
        return None;
    }
    let len = (reply.words[0] as usize).min(SHM_PAGE);
    if len == 0 {
        return Some(0);
    }
    let tok = reply.caps[0];
    if tok == CapabilityToken::INVALID {
        return None;
    }
    let ptr = match shm_map(tok) {
        Ok(ptr) => ptr,
        Err(_) => {
            let _ = shm_free(tok);
            return None;
        }
    };
    let copy_len = len.min(out.len());
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, out.as_mut_ptr(), copy_len);
    }
    let _ = shm_free(tok);
    Some(copy_len)
}

fn append_byte(out: &mut [u8], pos: &mut usize, byte: u8) {
    if *pos < out.len() {
        out[*pos] = byte;
        *pos += 1;
    }
}

fn append_bytes(out: &mut [u8], pos: &mut usize, bytes: &[u8]) {
    for &byte in bytes {
        append_byte(out, pos, byte);
    }
}

fn append_str(out: &mut [u8], pos: &mut usize, value: &str) {
    append_bytes(out, pos, value.as_bytes());
}

fn append_u64_hex(out: &mut [u8], pos: &mut usize, value: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut started = false;
    for shift in (0..16).rev() {
        let digit = ((value >> (shift * 4)) & 0xf) as usize;
        if digit != 0 || started || shift == 0 {
            started = true;
            append_byte(out, pos, HEX[digit]);
        }
    }
}

fn append_u64_dec(out: &mut [u8], pos: &mut usize, mut value: u64) {
    let mut tmp = [0u8; 20];
    let mut idx = tmp.len();
    loop {
        idx -= 1;
        tmp[idx] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    append_bytes(out, pos, &tmp[idx..]);
}

fn append_escaped_limited(out: &mut [u8], pos: &mut usize, value: &str) {
    let mut used = 0usize;
    for &byte in value.as_bytes() {
        if used >= NOTIFICATION_FIELD_MAX {
            break;
        }
        match byte {
            b'\\' => append_bytes(out, pos, b"\\\\"),
            b'\n' | b'\r' => append_bytes(out, pos, b"\\n"),
            b'=' => append_bytes(out, pos, b"\\e"),
            0x20..=0x7e => append_byte(out, pos, byte),
            _ => append_byte(out, pos, b'?'),
        }
        used += 1;
    }
}

fn append_field(out: &mut [u8], pos: &mut usize, key: &str, value: &str) {
    append_str(out, pos, key);
    append_byte(out, pos, b'=');
    append_escaped_limited(out, pos, value);
    append_byte(out, pos, b'\n');
}

fn notification_owner_key_into(owner: &str, out: &mut [u8]) -> usize {
    let mut pos = 0usize;
    for byte in owner.bytes() {
        let mapped = match byte {
            b'A'..=b'Z' => byte + 32,
            b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => byte,
            b' ' | b'\t' => b'-',
            _ => continue,
        };
        if pos < out.len() {
            out[pos] = mapped;
            pos += 1;
        }
    }
    if pos == 0 {
        let fallback = b"unknown";
        let len = fallback.len().min(out.len());
        out[..len].copy_from_slice(&fallback[..len]);
        pos = len;
    }
    pos
}

fn notification_record_key_into(owner: &str, id: &[u8], out: &mut [u8]) -> usize {
    let mut pos = 0usize;
    append_bytes(out, &mut pos, b"notifications/history/");
    let mut owner_buf = [0u8; 64];
    let owner_len = notification_owner_key_into(owner, &mut owner_buf);
    append_bytes(out, &mut pos, &owner_buf[..owner_len]);
    append_byte(out, &mut pos, b'/');
    append_bytes(out, &mut pos, id);
    pos
}

fn notification_next_id_into(out: &mut [u8]) -> usize {
    let unix = get_time_utc();
    let pid = getpid();
    let seq = unsafe {
        let current = NOTIFICATION_ID_COUNTER;
        NOTIFICATION_ID_COUNTER = NOTIFICATION_ID_COUNTER.saturating_add(1);
        current
    };
    let mut pos = 0usize;
    append_u64_hex(out, &mut pos, unix);
    append_byte(out, &mut pos, b'-');
    append_u64_hex(out, &mut pos, pid);
    append_byte(out, &mut pos, b'-');
    append_u64_hex(out, &mut pos, seq);
    pos
}

fn notification_store_raw_record(
    id: &[u8],
    sender_name: &str,
    owner: &str,
    sender_icon: Option<&str>,
    title: &str,
    body: &str,
    priority: NotificationPriority,
) {
    let mut key = [0u8; 160];
    let key_len = notification_record_key_into(owner, id, &mut key);
    let mut value = [0u8; SHM_PAGE];
    let mut pos = 0usize;
    append_bytes(&mut value, &mut pos, b"sunlight-notification-v1\n");
    append_bytes(&mut value, &mut pos, b"id=");
    append_bytes(&mut value, &mut pos, id);
    append_byte(&mut value, &mut pos, b'\n');
    append_bytes(&mut value, &mut pos, b"timestamp=");
    append_u64_dec(&mut value, &mut pos, get_time_utc());
    append_byte(&mut value, &mut pos, b'\n');
    append_bytes(&mut value, &mut pos, b"sender_pid=");
    append_u64_dec(&mut value, &mut pos, getpid());
    append_byte(&mut value, &mut pos, b'\n');
    append_field(&mut value, &mut pos, "sender_name", sender_name);
    append_field(
        &mut value,
        &mut pos,
        "sender_icon",
        sender_icon.unwrap_or(""),
    );
    append_field(&mut value, &mut pos, "owner", owner);
    append_field(&mut value, &mut pos, "title", title);
    append_field(&mut value, &mut pos, "body", body);
    append_field(
        &mut value,
        &mut pos,
        "priority",
        notification_priority_str(priority),
    );
    append_bytes(&mut value, &mut pos, b"seen=0\n");
    append_bytes(&mut value, &mut pos, b"dismissed=0\n");
    if !notification_kv_put(
        core::str::from_utf8(&key[..key_len]).unwrap_or(""),
        &value[..pos],
    ) {
        return;
    }

    let mut index = [0u8; SHM_PAGE];
    let mut index_len = notification_kv_get_into(NOTIFICATION_RECENT_KEY, &mut index).unwrap_or(0);
    let key_bytes = &key[..key_len];
    let mut new_index = [0u8; SHM_PAGE];
    let mut new_len = 0usize;
    append_bytes(&mut new_index, &mut new_len, key_bytes);
    append_byte(&mut new_index, &mut new_len, b'\n');
    let mut count = 1usize;
    let mut start = 0usize;
    while start < index_len && count < NOTIFICATION_RECENT_LIMIT {
        let mut end = start;
        while end < index_len && index[end] != b'\n' {
            end += 1;
        }
        let line = &index[start..end];
        if !line.is_empty() && line != key_bytes {
            append_bytes(&mut new_index, &mut new_len, line);
            append_byte(&mut new_index, &mut new_len, b'\n');
            count += 1;
        }
        start = end.saturating_add(1);
    }
    index_len = new_len;
    let _ = notification_kv_put(NOTIFICATION_RECENT_KEY, &new_index[..index_len]);
}

pub fn notification_dnd_enabled() -> bool {
    let mut buf = [0u8; 8];
    notification_kv_get_into(NOTIFICATION_DND_KEY, &mut buf)
        .map(|len| core::str::from_utf8(&buf[..len]).unwrap_or("").trim() == "1")
        .unwrap_or(false)
}

pub fn notification_set_dnd(enabled: bool) -> bool {
    notification_kv_put(NOTIFICATION_DND_KEY, if enabled { b"1" } else { b"0" })
}

pub fn show_notification_from(
    kind: NotificationKind,
    sender_name: &str,
    owner: &str,
    sender_icon: Option<&str>,
    title: &str,
    body: &str,
    timeout_ms: u64,
) -> bool {
    let mut id = [0u8; 48];
    let id_len = notification_next_id_into(&mut id);
    // TODO(notifications): restrict sender permissions, validate process/app manifest identity,
    // and prevent spoofed sender_name/owner values.
    notification_store_raw_record(
        &id[..id_len],
        sender_name,
        owner,
        sender_icon,
        title,
        body,
        notification_priority_for_kind(kind),
    );
    if notification_dnd_enabled() {
        return true;
    }
    show_notification_popup(kind, title, body, timeout_ms)
}

#[allow(non_snake_case)]
pub mod KbdMsg {
    pub const KEY_EVENT: u64 = 1;
}

#[allow(non_snake_case)]
pub mod MouseMsg {
    /// Raw mouse motion event: dx(i16) | dy(i16)<<16 | buttons(u8)<<32
    pub const RAW_MOTION: u64 = 0x3;
}

/// Device-independent relative pointer report shared by PS/2, USB HID, TTY,
/// and the graphical compositor. Positive X moves right and positive Y moves
/// down; the low three button bits are primary, secondary, and middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerReport {
    pub dx: i16,
    pub dy: i16,
    pub buttons: u8,
}

impl PointerReport {
    pub const fn new(dx: i16, dy: i16, buttons: u8) -> Self {
        Self {
            dx,
            dy,
            buttons: buttons & 0x07,
        }
    }

    pub const fn pack(self) -> u64 {
        (self.dx as u16 as u64) | ((self.dy as u16 as u64) << 16) | ((self.buttons as u64) << 32)
    }

    pub const fn unpack(word: u64) -> Self {
        Self {
            dx: (word & 0xffff) as u16 as i16,
            dy: ((word >> 16) & 0xffff) as u16 as i16,
            buttons: ((word >> 32) & 0x07) as u8,
        }
    }
}

#[cfg(test)]
mod pointer_report_tests {
    use super::PointerReport;

    #[test]
    fn normalized_pointer_report_round_trips_signed_axes_and_buttons() {
        for report in [
            PointerReport::new(12, -34, 0x01),
            PointerReport::new(i16::MIN, i16::MAX, 0x07),
            PointerReport::new(-1, 1, 0xff),
        ] {
            assert_eq!(PointerReport::unpack(report.pack()), report);
        }
    }
}

#[allow(non_snake_case)]
pub mod SpawnMsg {
    pub const SPAWN: u64 = 1;
    pub const REPLY: u64 = 2;
    pub const ERROR: u64 = 3;
    /// Spawn a native user shell from a one-time UAC session grant in caps[0].
    pub const SPAWN_AUTHENTICATED: u64 = 4;
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    Unlocked = 0,
    EnteringLock = 1,
    LockedWithPresenter = 2,
    LockedFallback = 3,
    RecoveringPresenter = 4,
    Authenticating = 5,
    LeavingLock = 6,
}

impl LockState {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::EnteringLock,
            2 => Self::LockedWithPresenter,
            3 => Self::LockedFallback,
            4 => Self::RecoveringPresenter,
            5 => Self::Authenticating,
            6 => Self::LeavingLock,
            _ => Self::Unlocked,
        }
    }
}

#[allow(non_snake_case)]
pub mod MezzoMsg {
    pub const SESSION_ESTABLISH: u64 = 0xB0FF;
    pub const LOCK_ACTIVATE: u64 = 0xB100;
    pub const LOCK_STATUS: u64 = 0xB101;
    pub const LOCK_RECOVER: u64 = 0xB102;
    pub const PRESENTER_HELLO: u64 = 0xB103;
    pub const PRESENTER_READY: u64 = 0xB104;
    pub const AUTHENTICATE: u64 = 0xB105;
    pub const REPLY: u64 = 0xB1FF;
    pub const ERROR: u64 = 0xB1FE;

    pub const RECOVER_SAFE: u64 = 1;
    pub const ERR_NOT_LOCKED: u64 = 1;
    pub const ERR_STALE: u64 = 2;
    pub const ERR_UNAUTHORIZED: u64 = 3;
    pub const ERR_START_FAILED: u64 = 4;
    pub const ERR_BUSY: u64 = 5;
    pub const ERR_NO_SESSION: u64 = 6;
}

pub const LOCK_CALLER_TTY_SERVICE: u64 = 1;
pub const LOCK_CALLER_AUTHENTICATED_TTY: u64 = 2;
pub const LOCK_SESSION_USERNAME_MAX: usize = 32;

/// IPC protocol between `tty_server` and a native `sshl` process.
#[allow(non_snake_case)]
pub mod ShellMsg {
    pub const KEY: u64 = 1;
    pub const OUTPUT: u64 = 2;
    pub const EXIT: u64 = 3;
    pub const DRAIN: u64 = 4;
    pub const FOREGROUND_STARTED: u64 = 5;
    pub const FOREGROUND_DONE: u64 = 6;
    /// Ask an authenticated shell to spawn `/bin/sshl<word0>` as its child.
    pub const SPAWN_TAB: u64 = 7;
    /// Successful SPAWN_TAB reply; word0 contains the child pid.
    pub const TAB_SPAWNED: u64 = 8;
    pub const ERROR: u64 = 0xff;
}

/// sunlight-sm (Storage Manager) opcodes. Registered as "sm".
/// Uses shm for payload (path + optional content) to support file sizes > inline.
#[allow(non_snake_case)]
pub mod SmMsg {
    pub const WRITE_FILE: u64 = 1;
    pub const MKDIR_ALL: u64 = 2;
    pub const REMOVE: u64 = 3;
    pub const READ_FILE: u64 = 4;
    // OP 5 batch left for future; current bite keeps simple per-op IPC.

    pub const REPLY_OK: u64 = 1;
    pub const REPLY_ERR: u64 = 0xff;

    /// Max bytes for path+content in one shm page grant.
    pub const PAGE_CAPACITY: usize = 4096;

    // Error codes returned in reply.words[0] when label==REPLY_ERR
    pub const ERR_OK: u64 = 0;
    pub const ERR_DENIED: u64 = 1;
    pub const ERR_INVALID_PATH: u64 = 2;
    pub const ERR_PAYLOAD_TOO_LARGE: u64 = 3;
    pub const ERR_NOT_FOUND: u64 = 4;
    pub const ERR_IO: u64 = 5;
    pub const ERR_UNSUPPORTED: u64 = 6;
}

pub type DriverId = u64;
pub type DeviceId = u64;

pub const HARDWARE_INVENTORY_MAX_RECORDS: usize = 2050;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HardwareBus {
    Pci = 1,
    Ps2 = 2,
    Platform = 3,
    Unknown = 255,
}

impl HardwareBus {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::Pci,
            2 => Self::Ps2,
            3 => Self::Platform,
            _ => Self::Unknown,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareState {
    Unknown = 0,
    Active = 1,
    Loaded = 2,
    ProbeFailed = 3,
    NoDriver = 4,
    Disabled = 5,
}

impl HardwareState {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::Active,
            2 => Self::Loaded,
            3 => Self::ProbeFailed,
            4 => Self::NoDriver,
            5 => Self::Disabled,
            _ => Self::Unknown,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareFailureStage {
    None = 0,
    Match = 1,
    ResourceAllocation = 2,
    ResourceMapping = 3,
    FeatureNegotiation = 4,
    QueueSetup = 5,
    DeviceInitialization = 6,
    DeviceActivation = 7,
    ServiceBinding = 8,
    Unknown = 255,
}

impl HardwareFailureStage {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Match,
            2 => Self::ResourceAllocation,
            3 => Self::ResourceMapping,
            4 => Self::FeatureNegotiation,
            5 => Self::QueueSetup,
            6 => Self::DeviceInitialization,
            7 => Self::DeviceActivation,
            8 => Self::ServiceBinding,
            _ => Self::Unknown,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareInventoryRecord {
    pub key: u64,
    pub identity: u64,
    pub subsystem: u64,
    pub matched_driver: u64,
    pub bound_driver: u64,
    pub state: u64,
    pub error_code: u64,
    pub irq: u64,
    pub bars: [u64; 6],
}

impl HardwareInventoryRecord {
    pub const fn empty() -> Self {
        Self {
            key: 0,
            identity: 0,
            subsystem: 0,
            matched_driver: 0,
            bound_driver: 0,
            state: HardwareState::Unknown as u64,
            error_code: 0,
            irq: u64::MAX,
            bars: [0; 6],
        }
    }

    pub const fn pci_key(domain: u16, bus: u8, device: u8, function: u8) -> u64 {
        (HardwareBus::Pci as u64)
            | ((domain as u64) << 8)
            | ((bus as u64) << 24)
            | ((device as u64) << 32)
            | ((function as u64) << 40)
    }

    pub const fn ps2_key(port: u8) -> u64 {
        (HardwareBus::Ps2 as u64) | ((port as u64) << 8)
    }

    pub const fn bus(self) -> HardwareBus {
        HardwareBus::from_u64(self.key & 0xff)
    }

    pub const fn pci_location(self) -> Option<(u16, u8, u8, u8)> {
        if matches!(self.bus(), HardwareBus::Pci) {
            Some((
                ((self.key >> 8) & 0xffff) as u16,
                ((self.key >> 24) & 0xff) as u8,
                ((self.key >> 32) & 0xff) as u8,
                ((self.key >> 40) & 0xff) as u8,
            ))
        } else {
            None
        }
    }

    pub const fn vendor_id(self) -> Option<u16> {
        if matches!(self.bus(), HardwareBus::Pci) {
            Some((self.identity & 0xffff) as u16)
        } else {
            None
        }
    }

    pub const fn device_id(self) -> Option<u16> {
        if matches!(self.bus(), HardwareBus::Pci) {
            Some(((self.identity >> 16) & 0xffff) as u16)
        } else {
            None
        }
    }

    pub const fn class(self) -> u8 {
        ((self.identity >> 32) & 0xff) as u8
    }

    pub const fn subclass(self) -> u8 {
        ((self.identity >> 40) & 0xff) as u8
    }

    pub const fn programming_interface(self) -> u8 {
        ((self.identity >> 48) & 0xff) as u8
    }

    pub const fn revision(self) -> u8 {
        ((self.identity >> 56) & 0xff) as u8
    }

    pub const fn hardware_state(self) -> HardwareState {
        HardwareState::from_u64(self.state & 0xff)
    }

    pub const fn failure_stage(self) -> HardwareFailureStage {
        HardwareFailureStage::from_u64((self.state >> 8) & 0xff)
    }
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    Virtio = 1,
    Keyboard = 2,
    Mouse = 3,
    Network = 4,
    Storage = 5,
    Block = 6,
    Display = 7,
    Audio = 8,
    Power = 9,
    Unknown = 255,
}

impl DriverKind {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::Virtio,
            2 => Self::Keyboard,
            3 => Self::Mouse,
            4 => Self::Network,
            5 => Self::Storage,
            6 => Self::Block,
            7 => Self::Display,
            8 => Self::Audio,
            9 => Self::Power,
            _ => Self::Unknown,
        }
    }
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverState {
    Starting = 1,
    Ready = 2,
    Blocked = 3,
    Failed = 4,
    Restarting = 5,
    Stopped = 6,
}

impl DriverState {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::Starting,
            2 => Self::Ready,
            3 => Self::Blocked,
            4 => Self::Failed,
            5 => Self::Restarting,
            6 => Self::Stopped,
            _ => Self::Failed,
        }
    }
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Input = 1,
    Network = 2,
    Block = 3,
    Display = 4,
    Audio = 5,
    Bus = 6,
    Unknown = 255,
}

impl DeviceKind {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::Input,
            2 => Self::Network,
            3 => Self::Block,
            4 => Self::Display,
            5 => Self::Audio,
            6 => Self::Bus,
            _ => Self::Unknown,
        }
    }
}

#[allow(non_snake_case)]
pub mod DriverCaps {
    pub const INPUT: u64 = 1 << 0;
    pub const KEYBOARD: u64 = 1 << 1;
    pub const POINTER: u64 = 1 << 2;
    pub const RELATIVE_MOTION: u64 = 1 << 3;
    pub const VIRTIO: u64 = 1 << 4;
    pub const BUS: u64 = 1 << 5;
    pub const NETWORK: u64 = 1 << 6;
    pub const BLOCK: u64 = 1 << 7;
    pub const STORAGE: u64 = 1 << 8;
    pub const DISPLAY: u64 = 1 << 9;
}

/// deviced v0 opcodes. Registered as "deviced".
///
/// Register IPC transports only four words, so v0 uses compact fixed records:
/// - driver names are packed into one word (8 byte short name)
/// - capabilities are a bitmask, rendered by clients
/// - longer metadata/device paths are intentionally deferred to a shm-backed v1
#[allow(non_snake_case)]
pub mod DevicedMsg {
    pub const REGISTER_DRIVER: u64 = 0xA001;
    pub const UPDATE_DRIVER_STATE: u64 = 0xA002;
    pub const TOUCH_DRIVER: u64 = 0xA003;
    pub const LIST_DRIVERS: u64 = 0xA004;
    pub const GET_DRIVER: u64 = 0xA005;
    pub const LIST_DEVICES: u64 = 0xA006;
    pub const GET_DEVICE: u64 = 0xA007;
    pub const MARK_DRIVER_FAILED: u64 = 0xA008;
    pub const UNREGISTER_DRIVER: u64 = 0xA009;
    pub const LIST_INVENTORY: u64 = 0xA101;
    pub const GET_INVENTORY: u64 = 0xA102;
    pub const GET_INVENTORY_FIELD: u64 = 0xA103;

    pub const REPLY: u64 = 0xA0FF;
    pub const INVENTORY_REPLY: u64 = 0xA1FF;
    pub const ERROR: u64 = 0xA0FE;

    pub const ERR_NOT_FOUND: u64 = 1;
    pub const ERR_FULL: u64 = 2;
    pub const ERR_BAD_REQUEST: u64 = 3;
    pub const ERR_UNSUPPORTED_FIELD: u64 = 4;

    pub const FIELD_SUBSYSTEM: u64 = 1;
    pub const FIELD_DRIVERS: u64 = 2;
    pub const FIELD_DIAGNOSTIC: u64 = 3;
    pub const FIELD_IRQ: u64 = 4;
    pub const FIELD_BAR0: u64 = 16;
    pub const FIELD_BAR5: u64 = FIELD_BAR0 + 5;
}

/// networkd v0 interface and config management.
///
/// Registered as "networkd".
/// Uses compact 4-word register IPC. Names packed as u64 (8 bytes).
/// Interface listing uses 0-based index like deviced LIST_*.
#[allow(non_snake_case)]
pub mod NetworkdMsg {
    pub const LIST_INTERFACES: u64 = 0xB001;
    pub const GET_INTERFACE: u64 = 0xB002;
    pub const ENABLE_INTERFACE: u64 = 0xB010;
    pub const DISABLE_INTERFACE: u64 = 0xB011;
    pub const SET_DHCP: u64 = 0xB012;
    pub const SET_STATIC_IPV4: u64 = 0xB013;
    pub const SET_PRIORITY: u64 = 0xB014;
    pub const SET_AUTO_CONNECT: u64 = 0xB015;
    pub const GET_DEFAULT_ROUTE: u64 = 0xB016;
    pub const REFRESH: u64 = 0xB017;
    pub const RENEW_DHCP: u64 = 0xB018; // optional v0

    pub const REPLY: u64 = 0xB0FF;
    pub const ERROR: u64 = 0xB0FE;

    pub const ERR_NOT_FOUND: u64 = 1;
    pub const ERR_BAD_REQUEST: u64 = 2;
    pub const ERR_NO_CARRIER: u64 = 3;
    pub const ERR_DEVICED_UNAVAILABLE: u64 = 4;
    pub const ERR_BACKEND_UNAVAILABLE: u64 = 5;
    pub const ERR_DEVICE_INITIALIZATION_FAILED: u64 = 6;
    pub const ERR_INVALID_STATE: u64 = 7;
    pub const ERR_PERMISSION_DENIED: u64 = 8;
    pub const ERR_UNSUPPORTED_OPERATION: u64 = 9;
    pub const ERR_IPC_FAILURE: u64 = 10;
}

/// resolved v0: DNS resolver configuration management.
/// Registered as "resolved".
/// Owns DNS server list, search domains, and generates /etc/resolv.conf view.
/// networkd hands DHCP/static DNS here.
#[allow(non_snake_case)]
pub mod ResolvedMsg {
    pub const GET_CONFIG: u64 = 0xC001;
    pub const GET_SERVER: u64 = 0xC002; // w0=index -> one server summary
    pub const SET_SERVERS: u64 = 0xC010; // w0..N packed addrs; source=Manual
    pub const ADD_SERVER: u64 = 0xC011; // w0=addr_packed, w1=source_tag
    pub const CLEAR_SERVERS: u64 = 0xC012;
    pub const USE_SYSTEM_DEFAULT: u64 = 0xC013;
    pub const UPDATE_FROM_NETWORKD: u64 = 0xC014; // w0=iface_packed, w1=source_tag, w2.. dns addrs
    pub const RENDER_RESOLV_CONF: u64 = 0xC015; // returns packed servers or text hint
    pub const RESOLVE_NAME: u64 = 0xC016; // optional: w/hostname -> ip or 0
    pub const FLUSH_CACHE: u64 = 0xC017;

    pub const REPLY: u64 = 0xC0FF;
    pub const ERROR: u64 = 0xC0FE;

    pub const ERR_NOT_FOUND: u64 = 1;
    pub const ERR_BAD_REQUEST: u64 = 2;
    pub const ERR_UNAVAILABLE: u64 = 3;
}

/// powerd v0: power profile and policy service.
/// Registered as "powerd".
/// Owns the current selected profile and computes an effective profile + derived knobs.
/// Future: battery/ACPI integration, scheduler bias, cache/prefetch/display hooks.
#[allow(non_snake_case)]
pub mod PowerdMsg {
    pub const GET_STATUS: u64 = 0xD001;
    pub const GET_PROFILE: u64 = 0xD002;
    pub const SET_PROFILE: u64 = 0xD010; // w0 = profile tag
    pub const SET_AUTO: u64 = 0xD011;
    pub const LIST_PROFILES: u64 = 0xD003; // w0 = index -> one profile summary
    pub const GET_POLICY: u64 = 0xD004;
    pub const UPDATE_CONTEXT: u64 = 0xD012; // w0.. packed context fields (best effort)

    // Optional custom policy hooks (v0 may return ERR_UNSUPPORTED)
    pub const SET_CUSTOM_POLICY: u64 = 0xD020;
    pub const GET_CUSTOM_POLICY: u64 = 0xD021;

    /// Temporary thermal upper-bound on the effective power mode.
    /// w0: severity (ThermalConstraintSeverity)
    /// w1: maximum_allowed_mode (PowerProfile tag; must be concrete, not Auto)
    /// w2: reason (ThermalConstraintReason)
    /// w3: source (ThermalConstraintSource)
    /// w4: monotonic_generation (must be > active generation to replace)
    pub const SET_THERMAL_CONSTRAINT: u64 = 0xD030;
    /// Clear a thermal constraint only when w0 generation >= active generation.
    pub const CLEAR_THERMAL_CONSTRAINT: u64 = 0xD031;
    /// Extended status: requested, effective, active constraints.
    pub const GET_POWER_POLICY_STATUS: u64 = 0xD032;

    pub const REPLY: u64 = 0xD0FF;
    pub const ERROR: u64 = 0xD0FE;

    pub const ERR_NOT_FOUND: u64 = 1;
    pub const ERR_BAD_REQUEST: u64 = 2;
    pub const ERR_UNAVAILABLE: u64 = 3;
    pub const ERR_UNSUPPORTED: u64 = 4;
    pub const ERR_STALE_GENERATION: u64 = 5;
    pub const ERR_DENIED: u64 = 6;
}

/// thermald: thermal policy manager (sensors, fan policy, thermal constraints).
/// Registered as "thermald". Does not own user power-mode selection.
#[allow(non_snake_case)]
pub mod ThermaldMsg {
    pub const GET_STATUS: u64 = 0xD201;
    pub const LIST_SENSORS: u64 = 0xD202; // w0 = index
    pub const LIST_COOLING: u64 = 0xD203; // w0 = index
    pub const GET_PROFILE: u64 = 0xD204;
    pub const SET_PROFILE: u64 = 0xD210; // w0 = CoolingProfile tag
    pub const GET_POLICY: u64 = 0xD205;
    pub const SET_VALIDATED_CUSTOM_POLICY: u64 = 0xD220; // optional; may ERR_UNSUPPORTED
    pub const RESET_SAFE_DEFAULTS: u64 = 0xD211;
    pub const FORCE_FIRMWARE_AUTO: u64 = 0xD212;
    /// Public system identity (no serial/UUID).
    pub const GET_IDENTITY: u64 = 0xD206;
    /// Service-internal / privileged lifecycle hooks (suspend/resume notification).
    pub const PREPARE_SUSPEND: u64 = 0xD230;
    pub const RESUME: u64 = 0xD231;

    pub const REPLY: u64 = 0xD2FF;
    pub const ERROR: u64 = 0xD2FE;

    pub const ERR_NOT_FOUND: u64 = 1;
    pub const ERR_BAD_REQUEST: u64 = 2;
    pub const ERR_UNAVAILABLE: u64 = 3;
    pub const ERR_UNSUPPORTED: u64 = 4;
    pub const ERR_DENIED: u64 = 5;
    pub const ERR_INVALID_CONFIG: u64 = 6;
}

/// Severity of a temporary thermal constraint published to powerd.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalConstraintSeverity {
    None = 0,
    Warm = 1,
    Hot = 2,
    Critical = 3,
}

impl ThermalConstraintSeverity {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Warm,
            2 => Self::Hot,
            3 => Self::Critical,
            _ => Self::None,
        }
    }
}

/// Why powerd is applying a thermal upper bound.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalConstraintReason {
    None = 0,
    ThermalWarm = 1,
    ThermalHot = 2,
    ThermalCritical = 3,
    SensorFailure = 4,
    FanFailure = 5,
}

impl ThermalConstraintReason {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::ThermalWarm,
            2 => Self::ThermalHot,
            3 => Self::ThermalCritical,
            4 => Self::SensorFailure,
            5 => Self::FanFailure,
            _ => Self::None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::ThermalWarm => "ThermalWarm",
            Self::ThermalHot => "ThermalHot",
            Self::ThermalCritical => "ThermalCritical",
            Self::SensorFailure => "SensorFailure",
            Self::FanFailure => "FanFailure",
        }
    }
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalConstraintSource {
    None = 0,
    Thermald = 1,
}

impl ThermalConstraintSource {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Thermald,
            _ => Self::None,
        }
    }
}

/// Overall thermal classification used by thermald / UI / CLI.
///
/// `Normal` is reported only when a controlling temperature sensor is valid
/// and fresh. Missing/stale/unsupported telemetry uses `Unavailable`.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalState {
    Normal = 0,
    Warm = 1,
    Hot = 2,
    Critical = 3,
    /// No valid controlling sensor (missing, stale, unsupported, or unknown).
    Unavailable = 4,
}

impl ThermalState {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Normal,
            1 => Self::Warm,
            2 => Self::Hot,
            3 => Self::Critical,
            4 => Self::Unavailable,
            // Unknown tags must not become Normal (that would imply a valid sensor).
            _ => Self::Unavailable,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Warm => "Warm",
            Self::Hot => "Hot",
            Self::Critical => "Critical",
            Self::Unavailable => "Unavailable",
        }
    }

    /// True when a live controlling temperature was used for classification.
    pub const fn has_valid_controlling_sensor(self) -> bool {
        matches!(
            self,
            Self::Normal | Self::Warm | Self::Hot | Self::Critical
        )
    }
}

/// Fan control mode reported by thermald.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanControlMode {
    FirmwareAuto = 0,
    Managed = 1,
    FullSpeed = 2,
    Unavailable = 3,
    MonitoringOnly = 4,
}

impl FanControlMode {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Managed,
            2 => Self::FullSpeed,
            3 => Self::Unavailable,
            4 => Self::MonitoringOnly,
            _ => Self::FirmwareAuto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirmwareAuto => "FirmwareAuto",
            Self::Managed => "Managed",
            Self::FullSpeed => "FullSpeed",
            Self::Unavailable => "Unavailable",
            Self::MonitoringOnly => "MonitoringOnly",
        }
    }
}

/// Cooling preference (thermal fan bias). Not a powerd power mode.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoolingProfile {
    Balanced = 0,
    Quiet = 1,
    Cool = 2,
    Performance = 3,
}

impl CoolingProfile {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Quiet,
            2 => Self::Cool,
            3 => Self::Performance,
            _ => Self::Balanced,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::Quiet => "Quiet",
            Self::Cool => "Cool",
            Self::Performance => "Performance",
        }
    }
}

/// Validated ThinkPad-style discrete fan levels (0 = min/off, 7 = high, 8 = full-speed).
/// Never includes a "disengaged" / unbounded mode.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FanLevel {
    Level0 = 0,
    Level1 = 1,
    Level2 = 2,
    Level3 = 3,
    Level4 = 4,
    Level5 = 5,
    Level6 = 6,
    Level7 = 7,
    FullSpeed = 8,
}

impl FanLevel {
    pub const fn from_u64(v: u64) -> Option<Self> {
        match v {
            0 => Some(Self::Level0),
            1 => Some(Self::Level1),
            2 => Some(Self::Level2),
            3 => Some(Self::Level3),
            4 => Some(Self::Level4),
            5 => Some(Self::Level5),
            6 => Some(Self::Level6),
            7 => Some(Self::Level7),
            8 => Some(Self::FullSpeed),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Managed-fan lease health as observed by thermald.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    None = 0,
    Healthy = 1,
    Expired = 2,
    Unavailable = 3,
}

impl LeaseState {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Healthy,
            2 => Self::Expired,
            3 => Self::Unavailable,
            _ => Self::None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Healthy => "Healthy",
            Self::Expired => "Expired",
            Self::Unavailable => "Unavailable",
        }
    }
}

#[allow(non_snake_case)]
pub mod ClipMsg {
    pub const SET_CLIPBOARD: u64 = 0xD101;
    pub const GET_CLIPBOARD: u64 = 0xD102;
    pub const GET_CLIPBOARD_SUMMARY: u64 = 0xD103;
    pub const LIST_CLIPBOARD_HISTORY: u64 = 0xD104;
    pub const SELECT_CLIPBOARD_HISTORY_ITEM: u64 = 0xD105;
    pub const CLEAR_CLIPBOARD: u64 = 0xD106;
    pub const CLEAR_CLIPBOARD_HISTORY: u64 = 0xD107;

    pub const REPLY: u64 = 0xD1FF;
    pub const ERROR: u64 = 0xD1FE;

    pub const SELECT_BY_INDEX: u64 = 0;
    pub const SELECT_BY_ID: u64 = 1;

    pub const ERR_BAD_REQUEST: u64 = 1;
    pub const ERR_NOT_FOUND: u64 = 2;
    pub const ERR_TOO_LARGE: u64 = 3;
    pub const ERR_UNSUPPORTED: u64 = 4;
    pub const ERR_CORRUPT: u64 = 5;
    pub const ERR_INTERNAL: u64 = 6;
}

/// Power profile selection (selected by user or Auto).
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerProfile {
    Turbo = 0,
    Performance = 1,
    Balanced = 2,
    LowPower = 3,
    Stamina = 4,
    Custom = 5,
    Auto = 6,
}

impl PowerProfile {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Turbo,
            1 => Self::Performance,
            2 => Self::Balanced,
            3 => Self::LowPower,
            4 => Self::Stamina,
            5 => Self::Custom,
            6 => Self::Auto,
            _ => Self::Balanced,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Turbo => "Turbo",
            Self::Performance => "Performance",
            Self::Balanced => "Balanced",
            Self::LowPower => "LowPower",
            Self::Stamina => "Stamina",
            Self::Custom => "Custom",
            Self::Auto => "Auto",
        }
    }
}

/// Runtime power context (best-effort; unknown fields are None / safe defaults in v0).
#[derive(Debug, Clone, Copy)]
pub struct PowerContext {
    pub on_ac: Option<bool>,
    pub battery_percent: Option<u8>,
    pub battery_present: bool,
    pub load_percent: Option<u8>,
    pub user_active: bool,
}

impl Default for PowerContext {
    fn default() -> Self {
        Self {
            on_ac: None,
            battery_percent: None,
            battery_present: false,
            load_percent: None,
            user_active: true,
        }
    }
}

/// Cache behavior hint.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Minimal = 0,
    Normal = 1,
    Aggressive = 2,
}

impl CacheMode {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Minimal,
            1 => Self::Normal,
            2 => Self::Aggressive,
            _ => Self::Normal,
        }
    }
}

/// Prefetch behavior hint.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefetchMode {
    Off = 0,
    Light = 1,
    Normal = 2,
    Aggressive = 3,
}

impl PrefetchMode {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Off,
            1 => Self::Light,
            2 => Self::Normal,
            3 => Self::Aggressive,
            _ => Self::Normal,
        }
    }
}

/// Visual/effects richness hint.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectsMode {
    Minimal = 0,
    Normal = 1,
    Rich = 2,
}

impl EffectsMode {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Minimal,
            1 => Self::Normal,
            2 => Self::Rich,
            _ => Self::Normal,
        }
    }
}

/// Scheduler bias hint for niced / future scheduler integration.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerBias {
    Battery = 0,
    Balanced = 1,
    Interactive = 2,
    Performance = 3,
}

impl SchedulerBias {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Battery,
            1 => Self::Balanced,
            2 => Self::Interactive,
            3 => Self::Performance,
            _ => Self::Balanced,
        }
    }
}

/// Derived policy exposed to consumers (cache, prefetch, scheduler, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerPolicy {
    pub selected_profile: PowerProfile,
    pub effective_profile: PowerProfile,
    pub cache_mode: CacheMode,
    pub prefetch_mode: PrefetchMode,
    pub effects_mode: EffectsMode,
    pub scheduler_bias: SchedulerBias,
    pub background_work_allowed: bool,
}

impl PowerPolicy {
    /// v0 mapping from (effective) profile to knobs.
    pub const fn from_profile(effective: PowerProfile) -> Self {
        match effective {
            PowerProfile::Turbo => Self {
                selected_profile: PowerProfile::Turbo,
                effective_profile: PowerProfile::Turbo,
                cache_mode: CacheMode::Aggressive,
                prefetch_mode: PrefetchMode::Aggressive,
                effects_mode: EffectsMode::Rich,
                scheduler_bias: SchedulerBias::Performance,
                background_work_allowed: true,
            },
            PowerProfile::Performance => Self {
                selected_profile: PowerProfile::Performance,
                effective_profile: PowerProfile::Performance,
                cache_mode: CacheMode::Aggressive,
                prefetch_mode: PrefetchMode::Normal,
                effects_mode: EffectsMode::Normal,
                scheduler_bias: SchedulerBias::Performance,
                background_work_allowed: true,
            },
            PowerProfile::Balanced => Self {
                selected_profile: PowerProfile::Balanced,
                effective_profile: PowerProfile::Balanced,
                cache_mode: CacheMode::Normal,
                prefetch_mode: PrefetchMode::Normal,
                effects_mode: EffectsMode::Normal,
                scheduler_bias: SchedulerBias::Balanced,
                background_work_allowed: true,
            },
            PowerProfile::LowPower => Self {
                selected_profile: PowerProfile::LowPower,
                effective_profile: PowerProfile::LowPower,
                cache_mode: CacheMode::Normal,
                prefetch_mode: PrefetchMode::Light,
                effects_mode: EffectsMode::Minimal,
                scheduler_bias: SchedulerBias::Balanced,
                background_work_allowed: false,
            },
            PowerProfile::Stamina => Self {
                selected_profile: PowerProfile::Stamina,
                effective_profile: PowerProfile::Stamina,
                cache_mode: CacheMode::Minimal,
                prefetch_mode: PrefetchMode::Off,
                effects_mode: EffectsMode::Minimal,
                scheduler_bias: SchedulerBias::Battery,
                background_work_allowed: false,
            },
            PowerProfile::Custom => Self {
                selected_profile: PowerProfile::Custom,
                effective_profile: PowerProfile::Custom,
                cache_mode: CacheMode::Normal,
                prefetch_mode: PrefetchMode::Normal,
                effects_mode: EffectsMode::Normal,
                scheduler_bias: SchedulerBias::Balanced,
                background_work_allowed: true,
            },
            PowerProfile::Auto => Self {
                // Auto should have been resolved to a concrete profile before calling.
                selected_profile: PowerProfile::Auto,
                effective_profile: PowerProfile::Balanced,
                cache_mode: CacheMode::Normal,
                prefetch_mode: PrefetchMode::Normal,
                effects_mode: EffectsMode::Normal,
                scheduler_bias: SchedulerBias::Balanced,
                background_work_allowed: true,
            },
        }
    }
}

/// DnsSource for resolver records (v0).
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsSource {
    Dhcp = 0,
    Static = 1,
    SystemDefault = 2,
    Vpn = 3,
    Manual = 4,
    Unknown = 5,
}

impl DnsSource {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Dhcp,
            1 => Self::Static,
            2 => Self::SystemDefault,
            3 => Self::Vpn,
            4 => Self::Manual,
            _ => Self::Unknown,
        }
    }
}

/// Compact server summary for IPC (fits in one reply).
#[derive(Debug, Clone, Copy)]
pub struct DnsServerSummary {
    pub address: [u8; 4],
    pub source: DnsSource,
    pub priority: i32,
    pub iface: u64, // packed short name or 0
    pub total: u16,
}

/// Pack a DnsServerSummary into a REPLY (similar style to IfaceSummary).
pub fn pack_dns_server_summary(s: &DnsServerSummary) -> IpcMsg {
    let src = s.source as u64;
    let prio_u = (s.priority.clamp(-32768, 32767) as i16 as u16) as u64;
    let tot = (s.total as u64) & 0xff;
    let w1 = src | (prio_u << 8) | (tot << 24);
    IpcMsg::with_label(ResolvedMsg::REPLY)
        .word(0, pack_ipv4(s.address))
        .word(1, w1)
        .word(2, s.iface)
}

/// Unpack from reply.
pub fn unpack_dns_server_summary(reply: &IpcMsg) -> Option<DnsServerSummary> {
    if reply.label != ResolvedMsg::REPLY || reply.word_count < 2 {
        return None;
    }
    let addr = unpack_ipv4(reply.words[0]);
    let w1 = reply.words[1];
    let src = DnsSource::from_u64(w1 & 0xff);
    let prio = (((w1 >> 8) & 0x7fff) as u16 as i16) as i32;
    let total = ((w1 >> 24) & 0xff) as u16;
    let iface = if reply.word_count > 2 {
        reply.words[2]
    } else {
        0
    };
    Some(DnsServerSummary {
        address: addr,
        source: src,
        priority: prio,
        iface,
        total,
    })
}

/// Interface kind reported by networkd (matches driver capabilities).
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceKind {
    Loopback = 0,
    Ethernet = 1,
    VirtioNet = 2,
    Wireless = 3,
    Tunnel = 4,
    Vmxnet3 = 5,
    Unknown = 255,
}

impl InterfaceKind {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            0 => Self::Loopback,
            1 => Self::Ethernet,
            2 => Self::VirtioNet,
            3 => Self::Wireless,
            4 => Self::Tunnel,
            5 => Self::Vmxnet3,
            _ => Self::Unknown,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Loopback => "Loopback",
            Self::Ethernet => "Ethernet",
            Self::VirtioNet => "VirtioNet",
            Self::Wireless => "Wireless",
            Self::Tunnel => "Tunnel",
            Self::Vmxnet3 => "VMXNET3",
            Self::Unknown => "Unknown",
        }
    }
}

/// Hardware backend selected by the kernel after PCI probing and successful
/// device initialization. This is the single identity carried from the driver
/// through the frame proxy into networkd/networkctl.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkBackendKind {
    VirtioNet = 1,
    Vmxnet3 = 2,
}

/// Authoritative VMXNET3 bring-up state shared by the kernel and diagnostics.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vmxnet3InitStage {
    NotProbed = 0,
    PciMatched = 1,
    BarsMapped = 2,
    RevisionSelected = 3,
    UptSelected = 4,
    MacRead = 5,
    DmaAllocated = 6,
    RingsInitialized = 7,
    DeviceActivated = 8,
    BackendInstalled = 9,
    FrameProxyRegistered = 10,
    InterfacePublished = 11,
    Failed = 12,
}

impl Vmxnet3InitStage {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::PciMatched,
            2 => Self::BarsMapped,
            3 => Self::RevisionSelected,
            4 => Self::UptSelected,
            5 => Self::MacRead,
            6 => Self::DmaAllocated,
            7 => Self::RingsInitialized,
            8 => Self::DeviceActivated,
            9 => Self::BackendInstalled,
            10 => Self::FrameProxyRegistered,
            11 => Self::InterfacePublished,
            12 => Self::Failed,
            _ => Self::NotProbed,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::NotProbed => "NotProbed",
            Self::PciMatched => "PciMatched",
            Self::BarsMapped => "BarsMapped",
            Self::RevisionSelected => "RevisionSelected",
            Self::UptSelected => "UptSelected",
            Self::MacRead => "MacRead",
            Self::DmaAllocated => "DmaAllocated",
            Self::RingsInitialized => "RingsInitialized",
            Self::DeviceActivated => "DeviceActivated",
            Self::BackendInstalled => "BackendInstalled",
            Self::FrameProxyRegistered => "FrameProxyRegistered",
            Self::InterfacePublished => "InterfacePublished",
            Self::Failed => "Failed",
        }
    }
}

impl Default for Vmxnet3InitStage {
    fn default() -> Self {
        Self::NotProbed
    }
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetBackendEvent {
    FrameProxyRegistered = 1,
    InterfacePublished = 2,
    DhcpTimeout = 3,
    DhcpLeaseInstalled = 4,
}

#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vmxnet3ErrorCode {
    None = 0,
    PciNotPresent = 1,
    BarZero = 2,
    BarIoUnsupported = 3,
    BarTypeUnsupported = 4,
    BarAddressUnusable = 5,
    BarSizeInvalid = 6,
    BarRoleIncorrect = 7,
    BarMappingFailed = 8,
    SharedDmaAllocation = 9,
    QueueDmaAllocation = 10,
    RingDmaAllocation = 11,
    TxBufferAllocation = 12,
    RxBufferAllocation = 13,
    ResetFailed = 14,
    RevisionUnsupported = 15,
    UptUnsupported = 16,
    InvalidMac = 17,
    MalformedMacRegisters = 18,
    ActivationFailed = 19,
    RxModeUpdateFailed = 20,
}

impl Vmxnet3ErrorCode {
    pub const fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::PciNotPresent,
            2 => Self::BarZero,
            3 => Self::BarIoUnsupported,
            4 => Self::BarTypeUnsupported,
            5 => Self::BarAddressUnusable,
            6 => Self::BarSizeInvalid,
            7 => Self::BarRoleIncorrect,
            8 => Self::BarMappingFailed,
            9 => Self::SharedDmaAllocation,
            10 => Self::QueueDmaAllocation,
            11 => Self::RingDmaAllocation,
            12 => Self::TxBufferAllocation,
            13 => Self::RxBufferAllocation,
            14 => Self::ResetFailed,
            15 => Self::RevisionUnsupported,
            16 => Self::UptUnsupported,
            17 => Self::InvalidMac,
            18 => Self::MalformedMacRegisters,
            19 => Self::ActivationFailed,
            20 => Self::RxModeUpdateFailed,
            _ => Self::None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PciNotPresent => "PCI device 15ad:07b0 not present",
            Self::BarZero => "zero BAR",
            Self::BarIoUnsupported => "unsupported I/O BAR",
            Self::BarTypeUnsupported => "unsupported memory BAR type",
            Self::BarAddressUnusable => "unusable BAR address",
            Self::BarSizeInvalid => "invalid BAR size",
            Self::BarRoleIncorrect => "incorrect BAR role",
            Self::BarMappingFailed => "BAR mapping failed",
            Self::SharedDmaAllocation => "driver-shared DMA allocation failed",
            Self::QueueDmaAllocation => "queue-descriptor DMA allocation failed",
            Self::RingDmaAllocation => "ring DMA allocation failed",
            Self::TxBufferAllocation => "TX buffer allocation failed",
            Self::RxBufferAllocation => "RX buffer allocation failed",
            Self::ResetFailed => "RESET_DEV failed",
            Self::RevisionUnsupported => "no supported VMXNET3 revision",
            Self::UptUnsupported => "no supported UPT version",
            Self::InvalidMac => "invalid MAC",
            Self::MalformedMacRegisters => "malformed MAC register result",
            Self::ActivationFailed => "ACTIVATE_DEV failed",
            Self::RxModeUpdateFailed => "UPDATE_RX_MODE failed",
        }
    }
}

impl NetworkBackendKind {
    pub const fn from_u64(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::VirtioNet),
            2 => Some(Self::Vmxnet3),
            _ => None,
        }
    }

    pub const fn interface_kind(self) -> InterfaceKind {
        match self {
            Self::VirtioNet => InterfaceKind::VirtioNet,
            Self::Vmxnet3 => InterfaceKind::Vmxnet3,
        }
    }

    pub const fn driver_name(self) -> &'static str {
        match self {
            Self::VirtioNet => "virtio",
            Self::Vmxnet3 => "vmxnet3",
        }
    }
}

/// Operational link state.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Unknown = 0,
    Down = 1,
    Up = 2,
    NoCarrier = 3,
    Carrier = 4,
}

impl LinkState {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Down,
            2 => Self::Up,
            3 => Self::NoCarrier,
            4 => Self::Carrier,
            _ => Self::Unknown,
        }
    }
}

/// Administrative state.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminState {
    Disabled = 0,
    Enabled = 1,
}

impl AdminState {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Enabled,
            _ => Self::Disabled,
        }
    }
}

/// IP configuration mode for an interface.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpConfigMode {
    None = 0,
    Dhcp = 1,
    Static = 2,
}

impl IpConfigMode {
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::Dhcp,
            2 => Self::Static,
            _ => Self::None,
        }
    }
}

pub type InterfaceId = u64;

/// Helper to pack an IPv4 a.b.c.d into a u64 (low 32 bits).
pub fn pack_ipv4(a: [u8; 4]) -> u64 {
    ((a[0] as u64) << 24) | ((a[1] as u64) << 16) | ((a[2] as u64) << 8) | (a[3] as u64)
}

/// Unpack low 32 bits as IPv4 [a,b,c,d].
pub fn unpack_ipv4(w: u64) -> [u8; 4] {
    [
        ((w >> 24) & 0xff) as u8,
        ((w >> 16) & 0xff) as u8,
        ((w >> 8) & 0xff) as u8,
        (w & 0xff) as u8,
    ]
}

/// Pack up to 8 bytes of a short name (e.g. "eth0", "lo") into a u64 for IPC.
pub fn pack_short_name(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut word = 0u64;
    let mut i = 0usize;
    while i < bytes.len().min(8) {
        word |= (bytes[i] as u64) << (i * 8);
        i += 1;
    }
    word
}

/// Structured reply summary for one interface (fits in 4-5 words).
/// Clients use LIST with sequential index; non-REPLY means end.
#[derive(Debug, Clone, Copy)]
pub struct IfaceSummary {
    pub id: InterfaceId,
    pub name: u64, // packed 8-byte name (or 0)
    pub kind: InterfaceKind,
    pub admin: AdminState,
    pub link: LinkState,
    pub mode: IpConfigMode,
    pub addr: [u8; 4], // 0.0.0.0 if none
    pub prefix: u8,
    pub gw: [u8; 4],
    pub dns: [u8; 4], // primary DNS or 0s; full list via future/ctl
    pub priority: i32,
    pub is_default: bool,
    pub total: u16, // total count
}

/// Pack an IfaceSummary into reply words for LIST/GET.
/// Compact wire:
///   w0=id, w1=name_packed, w2=meta, w3=addr|gw, w4=dns (optional)
pub fn pack_iface_summary(s: &IfaceSummary) -> IpcMsg {
    let prio_u = (s.priority.clamp(-32768, 32767) as i16 as u16) as u64;
    let tot = (s.total as u64) & 0xff;
    let is_def = if s.is_default { 1u64 } else { 0 };
    let w2 = (s.kind as u64)
        | ((s.admin as u64) << 8)
        | ((s.link as u64) << 16)
        | ((s.mode as u64) << 24)
        | ((s.prefix as u64) << 32)
        | (is_def << 40)
        | (prio_u << 41)
        | (tot << 56);

    let addr_p = pack_ipv4(s.addr);
    let gw_p = pack_ipv4(s.gw);
    let w3 = addr_p | (gw_p << 32);

    IpcMsg::with_label(NetworkdMsg::REPLY)
        .word(0, s.id)
        .word(1, s.name)
        .word(2, w2)
        .word(3, w3)
        .word(4, pack_ipv4(s.dns))
}

pub fn unpack_iface_summary(reply: &IpcMsg) -> Option<IfaceSummary> {
    if reply.label != NetworkdMsg::REPLY || reply.word_count < 4 {
        return None;
    }
    let id = reply.words[0];
    let name = reply.words[1];
    let w2 = reply.words[2];
    let w3 = reply.words[3];

    let kind = InterfaceKind::from_u64(w2 & 0xff);
    let admin = AdminState::from_u64((w2 >> 8) & 0xff);
    let link = LinkState::from_u64((w2 >> 16) & 0xff);
    let mode = IpConfigMode::from_u64((w2 >> 24) & 0xff);
    let prefix = ((w2 >> 32) & 0xff) as u8;
    let is_default = ((w2 >> 40) & 1) != 0;
    let prio = (((w2 >> 41) & 0x7fff) as u16 as i16) as i32;
    let total = ((w2 >> 56) & 0xff) as u16;

    let addr = unpack_ipv4(w3 & 0xffff_ffff);
    let gw = unpack_ipv4((w3 >> 32) & 0xffff_ffff);
    let dns = if reply.word_count > 4 {
        unpack_ipv4(reply.words[4])
    } else {
        [0u8; 4]
    };

    Some(IfaceSummary {
        id,
        name,
        kind,
        admin,
        link,
        mode,
        addr,
        prefix,
        gw,
        dns,
        priority: prio,
        is_default,
        total,
    })
}

/// Structured spawn request that carries both a binary path and an explicit
/// process name. Designed to fit within the register IPC budget.
///
/// Wire layout (register IPC — 4 available words via r8/r9/r10/r12):
///   words[0..3]: path, NUL-terminated, up to 32 bytes
///   words[4..5]: name hint, NUL-terminated, up to 16 bytes
///                (NOT transmitted by the register-based IPC path today;
///                 the kernel derives the name from the path basename instead.
///                 Reserved for future memory-mapped IPC channel.)
#[derive(Debug, Clone, Copy)]
pub struct SpawnRequest {
    /// Binary path, e.g. b"/sbin/timezone_service\0..."
    pub path: [u8; 32],
    /// Short process name, e.g. b"timezone_service\0..."
    pub name: [u8; 16],
    /// Requested uid for the child.
    pub uid: u32,
    /// Requested gid for the child.
    pub gid: u32,
    /// Declared capability mask for service-name lookups.
    pub service_caps: u64,
}

impl SpawnRequest {
    pub fn new(path: &str, name: &str) -> Self {
        let mut req = Self {
            path: [0; 32],
            name: [0; 16],
            uid: 0,
            gid: 0,
            service_caps: u64::MAX,
        };
        let pb = path.as_bytes();
        let plen = pb.len().min(31);
        req.path[..plen].copy_from_slice(&pb[..plen]);
        let nb = name.as_bytes();
        let nlen = nb.len().min(15);
        req.name[..nlen].copy_from_slice(&nb[..nlen]);
        req
    }

    pub fn with_identity(mut self, uid: u32, gid: u32) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    pub fn with_service_caps(mut self, service_caps: u64) -> Self {
        self.service_caps = service_caps;
        self
    }

    /// Pack into an IpcMsg. Path occupies words[0..3]; name hint in words[4..5].
    pub fn pack_into(&self, msg: &mut IpcMsg) {
        msg.label = SpawnMsg::SPAWN;
        for i in 0..4 {
            let mut w = 0u64;
            for j in 0..8usize {
                w |= (self.path[i * 8 + j] as u64) << (j * 8);
            }
            msg.words[i] = w;
        }
        for i in 0..2 {
            let mut w = 0u64;
            for j in 0..8usize {
                w |= (self.name[i * 8 + j] as u64) << (j * 8);
            }
            msg.words[4 + i] = w;
        }
        msg.caps[0] = CapabilityToken(self.uid as u64);
        msg.caps[1] = CapabilityToken((self.gid as u64) | (self.service_caps << 32));
        msg.cap_count = 2;
    }

    /// Unpack from an IpcMsg.
    pub fn unpack(msg: &IpcMsg) -> Self {
        let mut req = Self {
            path: [0; 32],
            name: [0; 16],
            uid: msg.caps[0].0 as u32,
            gid: msg.caps[1].0 as u32,
            service_caps: msg.caps[1].0 >> 32,
        };
        for i in 0..4 {
            let w = msg.words[i];
            for j in 0..8usize {
                req.path[i * 8 + j] = ((w >> (j * 8)) & 0xff) as u8;
            }
        }
        for i in 0..2 {
            let w = msg.words[4 + i];
            for j in 0..8usize {
                req.name[i * 8 + j] = ((w >> (j * 8)) & 0xff) as u8;
            }
        }
        req
    }

    pub fn path_str(&self) -> &str {
        let len = self.path.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.path[..len]).unwrap_or("")
    }

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(16);
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

/// Pack a key event into a single u64 word for IPC transport.
/// Layout: keycode(u8) | pressed(u8) << 8 | mods_byte(u8) << 16 | ascii(u8) << 24
pub fn pack_key_event(
    keycode: u8,
    pressed: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    super_key: bool,
    ascii: Option<u8>,
) -> u64 {
    let mut val = keycode as u64;
    val |= (pressed as u64) << 8;
    let mods = ((shift as u64) << 0)
        | ((ctrl as u64) << 1)
        | ((alt as u64) << 2)
        | ((super_key as u64) << 3);
    val |= mods << 16;
    val |= (ascii.unwrap_or(0) as u64) << 24;
    val
}

/// Unpack a key event from a u64 word.
pub fn unpack_key_event(val: u64) -> (u8, bool, bool, bool, bool, bool, Option<u8>) {
    let keycode = (val & 0xFF) as u8;
    let pressed = ((val >> 8) & 0xFF) != 0;
    let mods = (val >> 16) & 0xFF;
    let shift = (mods & 1) != 0;
    let ctrl = (mods & 2) != 0;
    let alt = (mods & 4) != 0;
    let super_key = (mods & 8) != 0;
    let ascii = if (val >> 24) & 0xFF != 0 {
        Some(((val >> 24) & 0xFF) as u8)
    } else {
        None
    };
    (keycode, pressed, shift, ctrl, alt, super_key, ascii)
}

/// Errors returned by IPC operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidCapability = 1,
    EndpointNotFound = 2,
    WouldBlock = 3,
    InvalidArgument = 4,
    /// `word_count` exceeds `IPC_REGISTER_WORDS` (4) — forged or malformed register value.
    InvalidWordCount = 5,
    /// `cap_count` exceeds `IPC_MAX_CAPS` (2) — forged or malformed register value.
    InvalidCapCount = 6,
    QueueFull = 7,
    DeadlineExpired = 8,
    Cancelled = 9,
    PeerClosed = 10,
}

/// Errors from shared memory grant syscalls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmError {
    OutOfMemory = 1,
    InvalidToken = 2,
    InvalidArgument = 3,
}

#[inline(always)]
unsafe fn raw_syscall(
    num: SunlightSyscall,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
    a7: u64,
) -> (u64, IpcMsg) {
    let ret: u64;
    let out_rdi: u64;
    let out_rsi: u64;
    let out_rdx: u64;
    let out_r8: u64;
    let out_r9: u64;
    let out_r10: u64;
    let out_r12: u64;
    let out_r13: u64;
    let out_r14: u64;
    // SAFETY: caller selects a valid syscall number and passes ABI-shaped arguments.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") num as u64 => ret,
            inlateout("rdi") a1 => out_rdi,
            inlateout("rsi") a2 => out_rsi,
            inlateout("rdx") a3 => out_rdx,
            inlateout("r8") a4 => out_r8,
            inlateout("r9") a5 => out_r9,
            inlateout("r10") a6 => out_r10,
            inlateout("r12") a7 => out_r12,
            lateout("r13") out_r13,
            lateout("r14") out_r14,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    let mut msg = IpcMsg::with_label(out_rdi);
    msg.badge = out_rsi;
    msg.word_count = (out_rdx & 0xffff_ffff) as u32;
    msg.cap_count = (out_rdx >> 32) as u32;
    msg.words[0] = out_r8;
    msg.words[1] = out_r9;
    msg.words[2] = out_r10;
    msg.words[3] = out_r12;
    msg.caps[0] = CapabilityToken(out_r13);
    msg.caps[1] = CapabilityToken(out_r14);
    (ret, msg)
}

#[inline(always)]
unsafe fn raw_syscall_ipc(num: SunlightSyscall, object: u64, msg: IpcMsg) -> (u64, IpcMsg) {
    let counts = msg.word_count as u64 | ((msg.cap_count as u64) << 32);
    let ret: u64;
    let out_rdi: u64;
    let out_rsi: u64;
    let out_rdx: u64;
    let out_r8: u64;
    let out_r9: u64;
    let out_r10: u64;
    let out_r12: u64;
    let out_r13: u64;
    let out_r14: u64;

    // SAFETY: fixed register IPC ABI. Only words[0..4) cross in registers:
    // r8/r9/r10/r12. r13/r14 carry capability tokens; the
    // generic syscall wrapper cannot do that because normal syscalls do not
    // pass cap registers.
    //
    // TRANSPORT LIMIT: IpcMsg has 8 logical words but this ABI only carries
    // words[0..4] (32 bytes). words[4..7] are silently dropped. Protocols
    // that pack strings into register words (e.g. KV keys at word 2) are
    // therefore limited to 16 transmitted bytes for the key. Extend via
    // shared memory (shm_alloc/shm_map) or add new SHM-key opcodes if
    // longer keys are needed.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") num as u64 => ret,
            inlateout("rdi") msg.label => out_rdi,
            inlateout("rsi") object => out_rsi,
            inlateout("rdx") counts => out_rdx,
            inlateout("r8") msg.words[0] => out_r8,
            inlateout("r9") msg.words[1] => out_r9,
            inlateout("r10") msg.words[2] => out_r10,
            inlateout("r12") msg.words[3] => out_r12,
            inlateout("r13") msg.caps[0].0 => out_r13,
            inlateout("r14") msg.caps[1].0 => out_r14,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    let mut reply = IpcMsg::with_label(out_rdi);
    reply.badge = out_rsi;
    reply.word_count = (out_rdx & 0xffff_ffff) as u32;
    reply.cap_count = (out_rdx >> 32) as u32;
    reply.words[0] = out_r8;
    reply.words[1] = out_r9;
    reply.words[2] = out_r10;
    reply.words[3] = out_r12;
    reply.caps[0] = CapabilityToken(out_r13);
    reply.caps[1] = CapabilityToken(out_r14);
    (ret, reply)
}

#[inline(always)]
fn would_block(ret: u64) -> bool {
    ret == IpcError::WouldBlock as u64
}

pub fn endpoint_create() -> EndpointId {
    // SAFETY: EndpointCreate takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::EndpointCreate, 0, 0, 0, 0, 0, 0, 0) };
    EndpointId(ret)
}

pub fn endpoint_bind(endpoint: u64) -> CapabilityToken {
    // SAFETY: EndpointBind accepts an opaque endpoint selector/token.
    let (ret, _) =
        unsafe { raw_syscall(SunlightSyscall::EndpointBind, endpoint, 0, 0, 0, 0, 0, 0) };
    CapabilityToken(ret)
}

/// Destroy an endpoint using its owner capability. All derived tokens are
/// revoked atomically with the endpoint.
pub fn endpoint_destroy(endpoint: EndpointId) -> bool {
    // SAFETY: EndpointDestroy accepts one opaque owner token.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::EndpointDestroy,
            endpoint.0,
            0,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret == 0
}

/// Nameserver-private endpoint identity/liveness query. The kernel accepts it
/// only from PID 1; other callers receive `false`.
pub fn nameserver_endpoint_is_live(cap: CapabilityToken, endpoint_id: u32) -> bool {
    // SAFETY: the syscall takes only a bearer token and numeric endpoint identity.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::NameserverEndpointValidate,
            cap.0,
            endpoint_id as u64,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret == 1
}

/// PID 1-only bounded registry diagnostic event.
pub fn nameserver_note_diagnostic(event: u64) {
    // SAFETY: the syscall accepts one bounded event code and authenticates PID 1.
    unsafe {
        raw_syscall(
            SunlightSyscall::NameserverDiagnosticEvent,
            event,
            0,
            0,
            0,
            0,
            0,
            0,
        );
    }
}

pub fn get_init_cap() -> CapabilityToken {
    endpoint_bind(INIT_NAMESERVER_ENDPOINT)
}

/// Client: send a message and block until reply.
///
/// WARNING: This is a blocking call with no timeout. It will loop
/// (yield + retry) forever until a reply arrives or the target endpoint
/// is destroyed. Use only for trusted boot-time or server-to-server flows
/// where the peer is known to be alive and responsive.
///
/// For interactive shell paths or best-effort clients (e.g. calculator
/// history talking to sunlight-kv), use `ipc_call_timeout` instead.
pub fn ipc_call(cap: CapabilityToken, msg: IpcMsg) -> IpcMsg {
    loop {
        // SAFETY: ipc_call passes a capability token and fixed register IPC message.
        let (ret, reply) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcCall, cap.0, msg) };
        if ret == IpcError::QueueFull as u64 {
            // Backpressure for the legacy infallible API: yield between retries.
            process_yield();
            continue;
        }
        if !would_block(ret) {
            return reply;
        }
        process_yield();
    }
}

/// Errors returned by `ipc_call_timeout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcCallError {
    /// The call did not complete within the provided deadline.
    Timeout,
    /// The capability token was invalid or lacked SEND rights.
    InvalidCapability,
    /// The nameserver (or target) could not resolve the endpoint.
    EndpointNotFound,
    /// Invalid argument (e.g. malformed message counts).
    InvalidArgument,
    /// The endpoint queue reached its fixed capacity; retry with backoff.
    QueueFull,
    /// The call was explicitly cancelled before a reply committed.
    Cancelled,
    /// The endpoint owner exited or destroyed the endpoint.
    PeerClosed,
    /// Other/unknown error code from the kernel.
    Unknown(u64),
}

/// Client: send a message and wait for reply up to `timeout_ms`.
///
/// This is the safe primitive for interactive/client paths. It arms one
/// absolute monotonic deadline in the kernel before issuing the call, so a
/// fully descheduled caller is woken even if the peer remains silent.
///
/// Semantics:
/// - The kernel IpcCall syscall returns WouldBlock while the operation remains
///   in flight. The scheduler deadline transition returns DeadlineExpired on
///   the next retry.
/// - On success (ret==0) the assembled reply IpcMsg is returned.
/// - Known transport errors are mapped to `IpcCallError`.
/// - Server-side semantic errors (e.g. KV_ERROR label in reply) are **not**
///   turned into IpcCallError; the caller must inspect `reply.label`.
///
/// The original `msg` is re-submitted on each retry. The kernel's pending_call
/// state machine tolerates re-submission of the same call.
pub fn ipc_call_timeout(
    cap: CapabilityToken,
    msg: IpcMsg,
    timeout_ms: u64,
) -> Result<IpcMsg, IpcCallError> {
    let Some(deadline) = monotonic_millis().checked_add(timeout_ms) else {
        return Err(IpcCallError::Timeout);
    };
    // SAFETY: IpcSetDeadline takes one absolute monotonic millisecond value.
    let (arm_ret, _) =
        unsafe { raw_syscall(SunlightSyscall::IpcSetDeadline, deadline, 0, 0, 0, 0, 0, 0) };
    if arm_ret != 0 {
        return Err(map_ipc_call_error(arm_ret));
    }
    loop {
        let (ret, reply) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcCall, cap.0, msg) };
        if !would_block(ret) {
            if ret == 0 {
                return Ok(reply);
            } else {
                return Err(map_ipc_call_error(ret));
            }
        }
        process_yield();
    }
}

fn map_ipc_call_error(ret: u64) -> IpcCallError {
    if ret == IpcError::InvalidCapability as u64 {
        IpcCallError::InvalidCapability
    } else if ret == IpcError::EndpointNotFound as u64 {
        IpcCallError::EndpointNotFound
    } else if ret == IpcError::InvalidArgument as u64
        || ret == IpcError::InvalidWordCount as u64
        || ret == IpcError::InvalidCapCount as u64
    {
        IpcCallError::InvalidArgument
    } else if ret == IpcError::QueueFull as u64 {
        IpcCallError::QueueFull
    } else if ret == IpcError::DeadlineExpired as u64 {
        IpcCallError::Timeout
    } else if ret == IpcError::Cancelled as u64 {
        IpcCallError::Cancelled
    } else if ret == IpcError::PeerClosed as u64 {
        IpcCallError::PeerClosed
    } else {
        IpcCallError::Unknown(ret)
    }
}

/// Server: block waiting for first incoming call.
pub fn ipc_recv(ep: EndpointId) -> IpcMsg {
    loop {
        // SAFETY: ipc_recv passes the endpoint owner token; kernel validates receive rights.
        let (ret, msg) =
            unsafe { raw_syscall_ipc(SunlightSyscall::IpcRecv, ep.0, IpcMsg::empty()) };
        if !would_block(ret) {
            return msg;
        }
        process_yield();
    }
}

/// Receive a message with a kernel-enforced absolute monotonic deadline.
///
/// Returns `None` if no message arrives before `timeout_ms` elapses.
pub fn ipc_recv_timeout(ep: EndpointId, timeout_ms: u64) -> Option<IpcMsg> {
    let deadline = monotonic_millis().checked_add(timeout_ms)?;
    // SAFETY: IpcSetDeadline takes one absolute monotonic millisecond value.
    let (arm_ret, _) =
        unsafe { raw_syscall(SunlightSyscall::IpcSetDeadline, deadline, 0, 0, 0, 0, 0, 0) };
    if arm_ret != 0 {
        return None;
    }
    loop {
        // SAFETY: ipc_recv_timeout passes the endpoint owner token; kernel validates receive rights.
        let (ret, msg) =
            unsafe { raw_syscall_ipc(SunlightSyscall::IpcRecv, ep.0, IpcMsg::empty()) };
        if !would_block(ret) {
            return (ret == 0).then_some(msg);
        }
        process_yield();
    }
}

pub fn ipc_reply(reply: IpcMsg) {
    // SAFETY: ipc_reply sends a fixed register IPC message to the current reply waiter.
    unsafe {
        raw_syscall_ipc(SunlightSyscall::IpcReply, 0, reply);
    }
}

/// Retain the current server call and return a generation-scoped completion
/// token. The caller must later use `ipc_complete_deferred_reply` exactly once.
pub fn ipc_defer_reply() -> Result<u64, IpcError> {
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::IpcDeferReply, 0, 0, 0, 0, 0, 0, 0) };
    if ret <= IpcError::PeerClosed as u64 {
        Err(match ret {
            value if value == IpcError::InvalidCapability as u64 => IpcError::InvalidCapability,
            value if value == IpcError::EndpointNotFound as u64 => IpcError::EndpointNotFound,
            value if value == IpcError::WouldBlock as u64 => IpcError::WouldBlock,
            value if value == IpcError::InvalidArgument as u64 => IpcError::InvalidArgument,
            value if value == IpcError::InvalidWordCount as u64 => IpcError::InvalidWordCount,
            value if value == IpcError::InvalidCapCount as u64 => IpcError::InvalidCapCount,
            value if value == IpcError::QueueFull as u64 => IpcError::QueueFull,
            value if value == IpcError::DeadlineExpired as u64 => IpcError::DeadlineExpired,
            value if value == IpcError::Cancelled as u64 => IpcError::Cancelled,
            _ => IpcError::PeerClosed,
        })
    } else {
        Ok(ret)
    }
}

/// Complete a deferred server reply. The kernel checks the token's original
/// caller and IPC generation before waking that task.
pub fn ipc_complete_deferred_reply(token: u64, reply: IpcMsg) -> Result<(), IpcError> {
    let (ret, _) =
        unsafe { raw_syscall_ipc(SunlightSyscall::IpcCompleteDeferredReply, token, reply) };
    if ret == 0 {
        Ok(())
    } else {
        Err(match ret {
            value if value == IpcError::InvalidCapability as u64 => IpcError::InvalidCapability,
            value if value == IpcError::EndpointNotFound as u64 => IpcError::EndpointNotFound,
            value if value == IpcError::WouldBlock as u64 => IpcError::WouldBlock,
            value if value == IpcError::InvalidArgument as u64 => IpcError::InvalidArgument,
            value if value == IpcError::InvalidWordCount as u64 => IpcError::InvalidWordCount,
            value if value == IpcError::InvalidCapCount as u64 => IpcError::InvalidCapCount,
            value if value == IpcError::QueueFull as u64 => IpcError::QueueFull,
            value if value == IpcError::DeadlineExpired as u64 => IpcError::DeadlineExpired,
            value if value == IpcError::Cancelled as u64 => IpcError::Cancelled,
            _ => IpcError::PeerClosed,
        })
    }
}

/// Return whether a previously deferred reply target still represents a live,
/// uncancelled caller. This lets services discard cancellation metadata without
/// sending a synthetic reply.
pub fn ipc_deferred_reply_is_live(token: u64) -> bool {
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::IpcDeferredReplyIsLive,
            token,
            0,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret == 1
}

/// Server: send reply and block for the next call.
pub fn ipc_reply_and_wait(ep: EndpointId, reply: IpcMsg) -> IpcMsg {
    loop {
        // SAFETY: ipc_reply_and_wait passes the endpoint owner token and fixed reply message.
        let (ret, msg) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcReplyWait, ep.0, reply) };
        if !would_block(ret) {
            return msg;
        }
        process_yield();
    }
}

/// Server: send reply, then make a single non-blocking attempt to receive the
/// next call. Returns `None` (instead of yield-looping forever) if no call is
/// pending yet, so callers can do periodic work (e.g. clock refresh) while
/// waiting for the next message.
pub fn ipc_reply_and_try_recv(ep: EndpointId, reply: IpcMsg) -> Option<IpcMsg> {
    // SAFETY: ipc_reply_and_try_recv passes the endpoint owner token and fixed reply message.
    let (ret, msg) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcReplyWait, ep.0, reply) };
    if would_block(ret) {
        None
    } else {
        Some(msg)
    }
}

pub fn notify_send(cap: CapabilityToken) {
    // SAFETY: notify_send passes only an opaque capability token.
    unsafe {
        raw_syscall(SunlightSyscall::IpcNotifySend, cap.0, 0, 0, 0, 0, 0, 0);
    }
}

pub fn notify_wait(ep: EndpointId) {
    loop {
        // SAFETY: notify_wait passes only an opaque endpoint token.
        let (ret, _) =
            unsafe { raw_syscall(SunlightSyscall::IpcNotifyWait, ep.0, 0, 0, 0, 0, 0, 0) };
        if !would_block(ret) {
            return;
        }
        process_yield();
    }
}

pub fn nameserver_register(name: &str, ep: EndpointId) -> bool {
    let msg = IpcMsg::with_label(InitMsg::REGISTER)
        .word(0, name_to_u64(name))
        .word(1, ep.0);
    loop {
        let init_cap = get_init_cap();
        if init_cap.0 == 0 {
            process_yield();
            continue;
        }
        let reply = ipc_call(init_cap, msg);
        if reply.label == InitMsg::GRANT && reply.words[0] == InitStatus::OK {
            return true;
        }
        return false;
    }
}

pub fn nameserver_lookup(name: &str) -> Option<CapabilityToken> {
    let init_cap = get_init_cap();
    let msg = IpcMsg::with_label(InitMsg::LOOKUP).word(0, name_to_u64(name));
    let reply = ipc_call(init_cap, msg);
    if reply.label == InitMsg::GRANT {
        Some(CapabilityToken(reply.words[0]))
    } else {
        None
    }
}

/// Nameserver lookup with a bounded timeout.
///
/// Returns `Some(cap)` only when the nameserver replies with `InitMsg::GRANT`.
/// Returns `None` on timeout, lookup failure (DENY or non-GRANT label),
/// IPC transport error, or any other failure.
///
/// This must be used by interactive shell code (e.g. calc history) so that
/// a broken or slow init/nameserver cannot freeze the shell.
pub fn nameserver_lookup_timeout(name: &str, timeout_ms: u64) -> Option<CapabilityToken> {
    let init_cap = get_init_cap();
    let msg = IpcMsg::with_label(InitMsg::LOOKUP).word(0, name_to_u64(name));
    match ipc_call_timeout(init_cap, msg, timeout_ms) {
        Ok(reply) if reply.label == InitMsg::GRANT => Some(CapabilityToken(reply.words[0])),
        _ => None,
    }
}

fn show_notification_popup(
    kind: NotificationKind,
    title: &str,
    body: &str,
    timeout_ms: u64,
) -> bool {
    let Some((payload_ptr, payload_cap)) =
        shm_create(core::mem::size_of::<NotificationWire>(), 0).ok()
    else {
        debug_log("[IPC] notification shm alloc failed\n");
        return false;
    };

    let wire = NotificationWire::new(kind, title, body, timeout_ms);
    unsafe {
        core::ptr::write(payload_ptr as *mut NotificationWire, wire);
    }

    let Some(display_ep) = nameserver_lookup_timeout("display_server", 100) else {
        let _ = shm_free(payload_cap);
        debug_log("[IPC] notification display lookup failed\n");
        return false;
    };

    let msg = IpcMsg::with_label(SgpMsg::SHOW_NOTIFICATION)
        .word(0, kind as u64)
        .word(1, timeout_ms)
        .with_cap(0, payload_cap);

    let ok = matches!(
        ipc_call_timeout(display_ep, msg, 100),
        Ok(reply) if reply.label == SgpMsg::REPLY
    );
    if !ok {
        debug_log("[IPC] notification send failed\n");
    }

    let _ = shm_free(payload_cap);
    ok
}

/// Best-effort desktop notification helper.
pub fn show_notification(kind: NotificationKind, title: &str, body: &str, timeout_ms: u64) -> bool {
    show_notification_from(kind, title, title, None, title, body, timeout_ms)
}

pub fn name_to_u64(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

pub fn debug_log(msg: &str) {
    // SAFETY: DebugLog receives a valid string pointer and bounded length.
    unsafe {
        raw_syscall(
            SunlightSyscall::DebugLog,
            msg.as_ptr() as u64,
            msg.len() as u64,
            0,
            0,
            0,
            0,
            0,
        );
    }
}

pub fn process_yield() {
    // SAFETY: ProcessYield takes no user pointers.
    unsafe {
        raw_syscall(SunlightSyscall::ProcessYield, 0, 0, 0, 0, 0, 0, 0);
    }
}

/// Push keyboard bytes into tab `tab`'s kernel stdin ring so the foreground
/// app can read them via fd0. Returns the number of bytes accepted.
pub fn tty_stdin_push(tab: u32, bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    // SAFETY: passes a read-only pointer/length describing `bytes`; the kernel
    // copies it into the tab's stdin ring before returning.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::TtyStdinPush,
            tab as u64,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
            0,
            0,
            0,
            0,
        )
    };
    ret as usize
}

/// Drain tab `tab`'s kernel stdout ring into `buf`. Returns bytes pulled.
pub fn tty_stdout_pull(tab: u32, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    // SAFETY: passes a writable pointer/length describing `buf`; the kernel
    // writes at most `buf.len()` bytes and returns the count.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::TtyStdoutPull,
            tab as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
            0,
        )
    };
    ret as usize
}

/// Non-reaping check whether `pid` is still alive (used to detect when a
/// foreground command exits).
pub fn process_is_alive(pid: u64) -> bool {
    // SAFETY: ProcessIsAlive takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::ProcessIsAlive, pid, 0, 0, 0, 0, 0, 0) };
    ret == 1
}

/// Non-blocking child wait. Returns the exit code once `pid` exits, or `None`
/// while it is still running or is not recognized as a child.
pub fn try_waitpid(pid: u64) -> Option<u64> {
    // SAFETY: WaitPid takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::WaitPid, pid, 0, 0, 0, 0, 0, 0) };
    if ret == u64::MAX || ret == u64::MAX - 1 {
        None
    } else {
        Some(ret)
    }
}

/// Blocking child wait built on `try_waitpid` + yield.
pub fn waitpid(pid: u64) -> u64 {
    loop {
        if let Some(code) = try_waitpid(pid) {
            return code;
        }
        if !process_is_alive(pid) {
            return u64::MAX;
        }
        process_yield();
    }
}

pub fn getpid() -> u64 {
    // SAFETY: GetPid takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::GetPid, 0, 0, 0, 0, 0, 0, 0) };
    ret
}

/// One conditioned secure entropy word. Callers requiring cryptographic
/// entropy must first check [`secure_entropy_ready`].
pub fn entropy_u64() -> u64 {
    // SAFETY: GetEntropy takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::GetEntropy, 0, 0, 0, 0, 0, 0, 0) };
    ret
}

/// Whether the kernel approved-entropy collector has completed.
pub fn secure_entropy_ready() -> bool {
    // SAFETY: SecureEntropyReady takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::SecureEntropyReady, 0, 0, 0, 0, 0, 0, 0) };
    ret == 1
}

/// Credentials returned by the kernel for an IPC caller. The syscall is
/// restricted to the trusted PTY service, so callers cannot use this helper
/// to inspect arbitrary processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyCallerCredentials {
    pub pid: u64,
    pub uid: u32,
    pub gid: u32,
}

pub fn pty_caller_credentials(caller_pid: u64) -> Option<PtyCallerCredentials> {
    // SAFETY: the kernel validates both the service identity and target PID.
    let (packed, reply) = unsafe {
        raw_syscall(
            SunlightSyscall::PtyGetCredentials,
            caller_pid,
            0,
            0,
            0,
            0,
            0,
            0,
        )
    };
    if packed == u64::MAX {
        return None;
    }
    Some(PtyCallerCredentials {
        pid: reply.words[0],
        uid: packed as u32,
        gid: (packed >> 32) as u32,
    })
}

pub fn consume_lock_auth_grant(
    grant: CapabilityToken,
    presenter_pid: u64,
) -> Option<(u32, u32)> {
    let (ret, msg) = unsafe {
        raw_syscall(
            SunlightSyscall::LockAuthConsume,
            grant.0,
            presenter_pid,
            0,
            0,
            0,
            0,
            0,
        )
    };
    if ret == u64::MAX {
        None
    } else {
        Some((ret as u32, msg.words[0] as u32))
    }
}

pub fn validate_lock_service_pid(pid: u64) -> bool {
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::ValidateLockServicePid,
            pid,
            0,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret == 1
}

pub fn validate_lock_caller(pid: u64, kind: u64) -> bool {
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::ValidateLockCaller,
            pid,
            kind,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret == 1
}

pub fn kill(pid: u64, sig: u32) -> bool {
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::Kill, pid, sig as u64, 0, 0, 0, 0, 0) };
    ret == 0
}

/// Set the nice value (-10..=10) for `pid` (0 = current process).
/// Wraps syscall 83 (SetNice), kernel clamps to NICE_MIN..=NICE_MAX.
pub fn set_nice(pid: u64, nice: i8) -> bool {
    // SAFETY: SetNice takes pid and nice value as plain integers, no pointers.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::SetNice,
            pid,
            nice as i64 as u64,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret != u64::MAX
}

/// Get the current nice value for `pid` (0 = current process).
/// Wraps syscall 84 (GetNice). Returns 0 on failure (best-effort).
pub fn get_nice(pid: u64) -> i8 {
    // SAFETY: GetNice takes pid as plain integer, no pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::GetNice, pid, 0, 0, 0, 0, 0, 0) };
    ret as i64 as i8
}

pub fn get_time_utc() -> u64 {
    // SAFETY: GetTimeUtc takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::GetTimeUtc, 0, 0, 0, 0, 0, 0, 0) };
    ret
}

/// Milliseconds since boot (~10 ms resolution, kernel timekeeper-derived).
/// Suitable for RTT/elapsed measurement where `get_time_utc`'s 1 s resolution
/// is too coarse.
pub fn monotonic_millis() -> u64 {
    // SAFETY: MonotonicMs takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::MonotonicMs, 0, 0, 0, 0, 0, 0, 0) };
    ret
}

pub fn hardware_inventory_record(index: usize) -> Option<(HardwareInventoryRecord, usize)> {
    let mut record = HardwareInventoryRecord::empty();
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::HardwareInventory,
            index as u64,
            &mut record as *mut HardwareInventoryRecord as u64,
            core::mem::size_of::<HardwareInventoryRecord>() as u64,
            0,
            0,
            0,
            0,
        )
    };
    if ret == u64::MAX {
        None
    } else {
        Some((record, ret as usize))
    }
}

/// Public system identity from kernel SMBIOS parse (no serial/UUID).
pub fn system_identity() -> Option<SystemIdentityRecord> {
    let mut record = SystemIdentityRecord::empty();
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::SystemIdentity,
            &mut record as *mut SystemIdentityRecord as u64,
            core::mem::size_of::<SystemIdentityRecord>() as u64,
            0,
            0,
            0,
            0,
            0,
        )
    };
    if ret == u64::MAX {
        None
    } else {
        Some(record)
    }
}

/// Thermal sensors from kernel Intel DTS backend.
/// Returns (filled_count, total_available).
pub fn thermal_sensors(out: &mut [ThermalSensorRecord]) -> Option<(usize, usize)> {
    if out.is_empty() {
        return Some((0, 0));
    }
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::ThermalSensors,
            out.as_mut_ptr() as u64,
            out.len() as u64,
            core::mem::size_of::<ThermalSensorRecord>() as u64,
            0,
            0,
            0,
            0,
        )
    };
    if ret == u64::MAX {
        None
    } else {
        let total = ret as usize;
        let filled = total.min(out.len());
        Some((filled, total))
    }
}

/// Phase 3.4: hand a raw Ethernet frame to the kernel-owned virtio-net
/// device for transmission. Returns `true` on success. Restricted to the
/// net_server process (pid 5) by the kernel.
pub fn net_tx(frame: &[u8]) -> bool {
    // SAFETY: passes a read-only pointer/length describing `frame` to the
    // kernel, which copies it into its own TX buffer before returning.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::NetTx,
            frame.as_ptr() as u64,
            frame.len() as u64,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret == 1
}

/// Phase 3.4: poll the kernel-owned virtio-net device's RX queue for one
/// frame, copying up to `buf.len()` bytes in. Returns the frame length, or
/// `0` if no frame is pending.
pub fn net_rx(buf: &mut [u8]) -> usize {
    // SAFETY: passes a writable pointer/capacity for `buf`; the kernel
    // bounds-checks and copies at most `buf.len()` bytes into it.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::NetRx,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret as usize
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NetDeviceInfo {
    pub present: bool,
    pub mac: [u8; 6],
    pub link_up: bool,
    pub backend: Option<NetworkBackendKind>,
    pub mtu: u16,
    pub state: u64,
    pub error: u64,
    pub device_resets: u64,
    pub device_activations: u64,
    pub tx_requests: u64,
    pub tx_submitted: u64,
    pub tx_completed: u64,
    pub tx_bytes: u64,
    pub tx_notifications: u64,
    pub tx_errors: u64,
    pub rx_buffers_posted: u64,
    pub rx_completed: u64,
    pub rx_delivered: u64,
    pub rx_bytes: u64,
    pub rx_dropped: u64,
    pub rx_errors: u64,
    pub tx_ring_full: u64,
    pub rx_bad_completion: u64,
    pub interrupts: u64,
    pub polls: u64,
    pub dhcp_discovers: u64,
    pub dhcp_offers: u64,
    pub dhcp_requests: u64,
    pub dhcp_acks: u64,
    pub dhcp_timeouts: u64,
    pub dhcp_leases_installed: u64,
    pub vmxnet3_stage: Vmxnet3InitStage,
    pub vmxnet3_failure_stage: Vmxnet3InitStage,
    pub vmxnet3_error_detail: u64,
}

impl NetDeviceInfo {
    pub fn publishable(&self) -> bool {
        self.present
            && self.backend.is_some()
            && self.mac != [0; 6]
            && self.mtu != 0
            && self.state == 4
            && self.error == 0
    }
}

pub fn net_device_info() -> Option<NetDeviceInfo> {
    net_device_info_with_event(None)
}

pub fn net_mark_backend_event(event: NetBackendEvent) -> Option<NetDeviceInfo> {
    net_device_info_with_event(Some(event))
}

fn net_device_info_with_event(event: Option<NetBackendEvent>) -> Option<NetDeviceInfo> {
    let mut words = [0u64; 32];
    let (result, _) = unsafe {
        raw_syscall(
            SunlightSyscall::NetInfo,
            words.as_mut_ptr() as u64,
            words.len() as u64,
            event.map(|value| value as u64).unwrap_or(0),
            0,
            0,
            0,
            0,
        )
    };
    if result != 1 {
        return None;
    }
    let bytes = words[0].to_le_bytes();
    Some(NetDeviceInfo {
        present: words[1] & 1 != 0,
        mac: [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]],
        link_up: words[1] & 2 != 0,
        backend: NetworkBackendKind::from_u64(words[2]),
        mtu: words[3] as u16,
        state: words[4],
        error: words[5],
        device_resets: words[6],
        device_activations: words[7],
        tx_requests: words[8],
        tx_submitted: words[9],
        tx_completed: words[10],
        tx_bytes: words[11],
        tx_notifications: words[12],
        tx_errors: words[13],
        rx_buffers_posted: words[14],
        rx_completed: words[15],
        rx_delivered: words[16],
        rx_bytes: words[17],
        rx_dropped: words[18],
        rx_errors: words[19],
        tx_ring_full: words[20],
        rx_bad_completion: words[21],
        interrupts: words[22],
        polls: words[23],
        dhcp_discovers: words[24],
        dhcp_offers: words[25],
        dhcp_requests: words[26],
        dhcp_acks: words[27],
        dhcp_timeouts: words[28],
        dhcp_leases_installed: words[29],
        vmxnet3_stage: Vmxnet3InitStage::from_u64(words[30]),
        vmxnet3_failure_stage: Vmxnet3InitStage::from_u64(words[31] & 0xff),
        vmxnet3_error_detail: words[31] >> 8,
    })
}

pub fn sysinfo() -> SystemInfo {
    let mut info = SystemInfo::default();
    // SAFETY: passes a pointer to a properly sized and aligned SystemInfo that
    // the kernel fills with four u64s.
    unsafe {
        raw_syscall(
            SunlightSyscall::SysInfo,
            &mut info as *mut SystemInfo as u64,
            0,
            0,
            0,
            0,
            0,
            0,
        );
    }
    info
}

/// Read the scheduler-confirmed online CPU count used by SWAP-1 policy.
pub fn swap_online_cpu_count() -> u32 {
    let (value, _) = unsafe { raw_syscall(SunlightSyscall::SwapCtl, 2, 0, 0, 0, 0, 0, 0) };
    value.max(1).min(u32::MAX as u64) as u32
}

/// Submit one immutable boot configuration. The kernel authenticates the
/// caller and independently recomputes every field from kernel-owned system
/// information before accepting it.
pub fn swap_configure(policy: &swap_policy::SwapPolicy) -> bool {
    let (value, _) = unsafe {
        raw_syscall(
            SunlightSyscall::SwapCtl,
            3,
            policy.detected_ram_bytes,
            u64::from(policy.detected_online_cpus),
            policy.total_logical_bytes,
            policy.pool_count as u64,
            policy.total_physical_budget_bytes,
            u64::from(policy.version),
        )
    };
    value == 0
}

/// Return active pool count, or zero when swap is not configured.
pub fn swap_active_pool_count() -> usize {
    let (value, _) = unsafe { raw_syscall(SunlightSyscall::SwapCtl, 4, 0, 0, 0, 0, 0, 0) };
    value as usize
}

/// Read one bounded aggregate SWAP-1 health snapshot.
pub fn swap_aggregate_diagnostics() -> Option<swap_policy::AggregateDiagnostics> {
    let mut diagnostics = swap_policy::AggregateDiagnostics::default();
    let (value, _) = unsafe {
        raw_syscall(
            SunlightSyscall::SwapCtl,
            5,
            (&mut diagnostics as *mut swap_policy::AggregateDiagnostics) as u64,
            core::mem::size_of::<swap_policy::AggregateDiagnostics>() as u64,
            0,
            0,
            0,
            0,
        )
    };
    (value == 0).then_some(diagnostics)
}

/// Read one bounded per-pool SWAP-1 health snapshot.
pub fn swap_pool_diagnostics(index: usize) -> Option<swap_policy::PoolDiagnostics> {
    let mut diagnostics = swap_policy::PoolDiagnostics::default();
    let (value, _) = unsafe {
        raw_syscall(
            SunlightSyscall::SwapCtl,
            6,
            index as u64,
            (&mut diagnostics as *mut swap_policy::PoolDiagnostics) as u64,
            core::mem::size_of::<swap_policy::PoolDiagnostics>() as u64,
            0,
            0,
            0,
        )
    };
    (value == 0).then_some(diagnostics)
}

pub struct ProcessExit;
impl ProcessExit {
    pub fn exit(code: i32) -> ! {
        // SAFETY: ProcessExit terminates the current process.
        unsafe {
            raw_syscall(SunlightSyscall::ProcessExit, code as u64, 0, 0, 0, 0, 0, 0);
        }
        loop {
            // SAFETY: hlt is safe in a non-returning fallback loop.
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }
}

pub mod process_exit {
    pub use super::ProcessExit;
}

pub const SHM_PAGE: usize = 4096;

/// Allocate a 4 KiB shared page (legacy/bulk-IPC path). For larger regions use shm_create.
pub fn shm_alloc() -> Result<(*mut u8, CapabilityToken), ShmError> {
    shm_create(SHM_PAGE, 0)
}

/// Create a shared memory object of the given size (bytes). Returns (base ptr, token).
/// The kernel will round up to whole pages. Use the returned token with shm_map.
pub fn shm_create(size: usize, flags: u64) -> Result<(*mut u8, CapabilityToken), ShmError> {
    let (ret, msg) =
        unsafe { raw_syscall(SunlightSyscall::ShmAlloc, size as u64, flags, 0, 0, 0, 0, 0) };
    if ret == 0
        || ret == u64::MAX
        || ret & (SHM_PAGE as u64 - 1) != 0
        || msg.caps[0] == CapabilityToken::INVALID
    {
        return Err(ShmError::OutOfMemory);
    }
    Ok((ret as *mut u8, msg.caps[0]))
}

/// Map a shared region (any size) into the caller's AS using a received token. Returns base ptr.
pub fn shm_map(token: CapabilityToken) -> Result<*mut u8, ShmError> {
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::ShmMap, token.0, 0, 0, 0, 0, 0, 0) };
    if ret == 0 || ret == u64::MAX || ret & (SHM_PAGE as u64 - 1) != 0 {
        return Err(ShmError::InvalidToken);
    }
    Ok(ret as *mut u8)
}

/// Unmap and (if owner) release the shared page grant.
pub fn shm_free(token: CapabilityToken) -> Result<(), ShmError> {
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::ShmFree, token.0, 0, 0, 0, 0, 0, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(ShmError::InvalidToken)
    }
}

/// Map the kernel telemetry page into this process; returns null on failure.
pub fn map_telemetry() -> *const u8 {
    // SAFETY: MapTelemetry takes no pointers and returns a virtual address in rax.
    let (addr, _) = unsafe { raw_syscall(SunlightSyscall::MapTelemetry, 0, 0, 0, 0, 0, 0, 0) };
    addr as *const u8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferInfo {
    pub width: u32,
    pub height: u32,
    pub pitch_bytes: u32,
    pub bits_per_pixel: u32,
    pub mapped_len: u64,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
}

fn decode_framebuffer_info(msg: &IpcMsg) -> Option<FramebufferInfo> {
    let packed = msg.words[0];
    let format = msg.badge;
    // The kernel packs exactly seven one-byte fields (memory model plus six
    // mask fields). Reject a widened or malformed register response instead
    // of silently narrowing it before display-side bounds checks run.
    if format & !0x00ff_ffff_ffff_ffff != 0 {
        return None;
    }
    let info = FramebufferInfo {
        width: packed as u32,
        height: (packed >> 32) as u32,
        pitch_bytes: u32::try_from(msg.words[1]).ok()?,
        bits_per_pixel: u32::try_from(msg.words[2]).ok()?,
        mapped_len: msg.words[3],
        memory_model: format as u8,
        red_mask_size: (format >> 8) as u8,
        red_mask_shift: (format >> 16) as u8,
        green_mask_size: (format >> 24) as u8,
        green_mask_shift: (format >> 32) as u8,
        blue_mask_size: (format >> 40) as u8,
        blue_mask_shift: (format >> 48) as u8,
    };
    let visible_len = u64::from(info.pitch_bytes).checked_mul(u64::from(info.height))?;
    if info.width == 0
        || info.height == 0
        || info.pitch_bytes == 0
        || info.mapped_len == 0
        || info.mapped_len < visible_len
    {
        None
    } else {
        Some(info)
    }
}

#[cfg(test)]
mod framebuffer_info_tests {
    use super::*;

    fn valid_message() -> IpcMsg {
        let mut msg = IpcMsg::empty();
        msg.words[0] = 640 | (480u64 << 32);
        msg.words[1] = 640 * 4;
        msg.words[2] = 32;
        msg.words[3] = 640 * 4 * 480;
        msg.badge = 1 | (8 << 8) | (16 << 16) | (8 << 24) | (8 << 32) | (8 << 40);
        msg
    }

    #[test]
    fn framebuffer_reply_requires_representable_complete_geometry() {
        assert!(decode_framebuffer_info(&valid_message()).is_some());

        let mut truncated = valid_message();
        truncated.words[1] = u64::from(u32::MAX) + 1;
        assert!(decode_framebuffer_info(&truncated).is_none());

        let mut short = valid_message();
        short.words[3] -= 1;
        assert!(decode_framebuffer_info(&short).is_none());

        let mut malformed_format = valid_message();
        malformed_format.badge |= 1u64 << 60;
        assert!(decode_framebuffer_info(&malformed_format).is_none());
    }
}

/// Query the immutable Limine framebuffer descriptor without mapping any pages.
pub fn framebuffer_info() -> Option<FramebufferInfo> {
    let (ok, msg) = unsafe { raw_syscall(SunlightSyscall::MapFramebuffer, 1, 0, 0, 0, 0, 0, 0) };
    if ok == 0 || ok == u64::MAX {
        return None;
    }
    decode_framebuffer_info(&msg)
}

fn map_framebuffer_backend(selector: u64) -> Option<(*mut u8, FramebufferInfo)> {
    let (va, msg) =
        unsafe { raw_syscall(SunlightSyscall::MapFramebuffer, selector, 0, 0, 0, 0, 0, 0) };
    if va == 0 || va == u64::MAX {
        return None;
    }
    Some((va as *mut u8, decode_framebuffer_info(&msg)?))
}

/// Map the original immutable Limine framebuffer.
pub fn map_limine_framebuffer() -> Option<(*mut u8, FramebufferInfo)> {
    map_framebuffer_backend(2)
}

/// Map VMware SVGA VRAM only when the SVGA backend is fully active.
pub fn map_svga_framebuffer() -> Option<(*mut u8, FramebufferInfo)> {
    map_framebuffer_backend(3)
}

/// Query the VirtIO GPU dimensions.
/// Returns Some((width, height)) if a VirtIO GPU is present and initialized.
pub fn gpu_get_info() -> Option<(u32, u32)> {
    let (ok, msg) = unsafe { raw_syscall(SunlightSyscall::GpuGetInfo, 0, 0, 0, 0, 0, 0, 0) };
    if ok == 0 {
        return None;
    }
    let packed = msg.words[0];
    let w = (packed & 0xFFFF_FFFF) as u32;
    let h = (packed >> 32) as u32;
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

/// Failure reasons for the GPU setup syscalls (120/121).
/// Keep numerically in sync with the kernel's `GPU_ERR_*` constants in
/// `kernel/src/arch/x86_64/syscall.rs`.
pub mod gpu_proxy {
    pub const ERR_NOT_DISPLAY_SERVER: u32 = 1;
    pub const ERR_BAD_ARGS: u32 = 2;
    pub const ERR_NO_DEVICE: u32 = 3;
    pub const ERR_UNMAPPED_PAGE: u32 = 4;
    pub const ERR_SG_OVERFLOW: u32 = 5;
    pub const ERR_CREATE_FAILED: u32 = 6;
    pub const ERR_ATTACH_FAILED: u32 = 7;
    pub const ERR_SCANOUT_FAILED: u32 = 8;

    /// Kernel-reported failure: reason plus a step-specific detail word
    /// (VirtIO response code, failing page index, or sg entry count).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GpuProxyError {
        pub reason: u32,
        pub detail: u32,
    }

    impl GpuProxyError {
        pub fn from_word(word: u64) -> Self {
            Self {
                reason: (word & 0xFFFF_FFFF) as u32,
                detail: (word >> 32) as u32,
            }
        }

        pub fn reason_str(&self) -> &'static str {
            match self.reason {
                ERR_NOT_DISPLAY_SERVER => "not-display-server",
                ERR_BAD_ARGS => "bad-args",
                ERR_NO_DEVICE => "no-gpu-device",
                ERR_UNMAPPED_PAGE => "unmapped-page",
                ERR_SG_OVERFLOW => "sg-overflow",
                ERR_CREATE_FAILED => "resource-create-failed",
                ERR_ATTACH_FAILED => "attach-backing-failed",
                ERR_SCANOUT_FAILED => "set-scanout-failed",
                _ => "unknown",
            }
        }
    }
}

/// Attach the display server's back_buffer pages to VirtIO GPU scanout resource 1.
/// `buf_ptr`: pointer to the start of back_buffer memory (must be page-aligned).
/// `num_pages`: number of 4KiB pages to attach.
/// On failure returns the kernel's reason + detail.
pub fn gpu_attach_backing(
    buf_ptr: *const u32,
    num_pages: usize,
) -> Result<(), gpu_proxy::GpuProxyError> {
    let (ok, msg) = unsafe {
        raw_syscall(
            SunlightSyscall::GpuAttachBacking,
            buf_ptr as u64,
            num_pages as u64,
            0,
            0,
            0,
            0,
            0,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(gpu_proxy::GpuProxyError::from_word(msg.words[0]))
    }
}

/// Wire resource 1 to scanout 0. On failure returns the kernel's reason + detail.
pub fn gpu_set_scanout() -> Result<(), gpu_proxy::GpuProxyError> {
    let (ok, msg) = unsafe { raw_syscall(SunlightSyscall::GpuSetScanout, 0, 0, 0, 0, 0, 0, 0) };
    if ok != 0 {
        Ok(())
    } else {
        Err(gpu_proxy::GpuProxyError::from_word(msg.words[0]))
    }
}

/// Issue TRANSFER_TO_HOST_2D + RESOURCE_FLUSH for a dirty rectangle.
pub fn gpu_flush(x: u32, y: u32, w: u32, h: u32) -> bool {
    let a1 = (x as u64) | ((y as u64) << 32);
    let a2 = (w as u64) | ((h as u64) << 32);
    let (ok, _) = unsafe { raw_syscall(SunlightSyscall::GpuFlush, a1, a2, 0, 0, 0, 0, 0) };
    ok != 0
}

/// Upload a new cursor image and set the hotspot.
/// `pixels`: pointer to 64×64 BGRA pixels (transparent where alpha=0).
/// `hot_x`/`hot_y`: cursor hotspot in pixels.
pub fn gpu_update_cursor(pixels: *const u32, num_pixels: usize, hot_x: u32, hot_y: u32) -> bool {
    let a3 = (hot_x as u64) | ((hot_y as u64) << 32);
    let (ok, _) = unsafe {
        raw_syscall(
            SunlightSyscall::GpuUpdateCursor,
            pixels as u64,
            num_pixels as u64,
            a3,
            0,
            0,
            0,
            0,
        )
    };
    ok != 0
}

/// Query live display metrics from `display_server`.
pub fn query_display_metrics(display_ep: CapabilityToken) -> Option<DisplayMetrics> {
    let reply = ipc_call(display_ep, IpcMsg::with_label(sgp::SgpMsg::GET_SCREEN_INFO));
    if reply.label != sgp::SgpMsg::REPLY || reply.words[0] == 0 {
        return None;
    }
    Some(DisplayMetrics::from_reply(&reply.words).clamped_for_clients())
}

pub fn query_display_mode_capabilities(
    display_ep: CapabilityToken,
) -> Option<DisplayModeCapabilities> {
    let reply = ipc_call(
        display_ep,
        IpcMsg::with_label(sgp::SgpMsg::GET_DISPLAY_MODE_CAPABILITIES),
    );
    if reply.label != sgp::SgpMsg::REPLY || reply.words[0] == 0 {
        return None;
    }
    let meta = reply.words[2];
    Some(DisplayModeCapabilities {
        backend: ScreenBackend::from_discriminant((meta & 0xff) as u8),
        current_mode: DisplayMode::from_words(reply.words[0], reply.words[1], reply.words[3]),
        management: DisplayModeManagement::from_u8(((meta >> 8) & 0xff) as u8),
        read_only_reason: DisplayModeReadOnlyReason::from_u8(((meta >> 16) & 0xff) as u8),
        mode_count: (meta >> 32) as u32,
    })
}

pub fn query_display_mode(display_ep: CapabilityToken, index: u32) -> Option<DisplayMode> {
    let reply = ipc_call(
        display_ep,
        IpcMsg::with_label(sgp::SgpMsg::GET_DISPLAY_MODE).word(0, index as u64),
    );
    if reply.label != sgp::SgpMsg::REPLY || reply.words[3] == 0 {
        return None;
    }
    Some(DisplayMode::from_words(
        reply.words[0],
        reply.words[1],
        reply.words[2],
    ))
}

pub fn begin_display_mode_change(
    display_ep: CapabilityToken,
    width: u32,
    height: u32,
    timeout_ms: u64,
) -> Option<DisplayModeTransaction> {
    let reply = ipc_call(
        display_ep,
        IpcMsg::with_label(sgp::SgpMsg::BEGIN_DISPLAY_MODE_CHANGE)
            .word(0, width as u64 | ((height as u64) << 32))
            .word(1, timeout_ms),
    );
    if reply.label != sgp::SgpMsg::REPLY || reply.words[0] == 0 {
        return None;
    }
    Some(DisplayModeTransaction {
        token: reply.words[0],
        applied_mode: DisplayMode::from_words(reply.words[1], reply.words[2], 2),
        deadline_ms: reply.words[3],
    })
}

pub fn confirm_display_mode_change(display_ep: CapabilityToken, token: u64) -> bool {
    let reply = ipc_call(
        display_ep,
        IpcMsg::with_label(sgp::SgpMsg::CONFIRM_DISPLAY_MODE_CHANGE).word(0, token),
    );
    reply.label == sgp::SgpMsg::REPLY && reply.words[0] == 1
}

pub fn revert_display_mode_change(display_ep: CapabilityToken, token: u64) -> bool {
    let reply = ipc_call(
        display_ep,
        IpcMsg::with_label(sgp::SgpMsg::REVERT_DISPLAY_MODE_CHANGE).word(0, token),
    );
    reply.label == sgp::SgpMsg::REPLY && reply.words[0] == 1
}

pub fn attach_display_mode_dialog(display_ep: CapabilityToken, token: u64, window_id: u64) -> bool {
    let reply = ipc_call(
        display_ep,
        IpcMsg::with_label(sgp::SgpMsg::ATTACH_DISPLAY_MODE_DIALOG)
            .word(0, token)
            .word(1, window_id),
    );
    reply.label == sgp::SgpMsg::REPLY && reply.words[0] == 1
}

/// Move the hardware cursor to (x, y) without changing its image.
pub fn gpu_move_cursor(x: u32, y: u32) -> bool {
    let a1 = (x as u64) | ((y as u64) << 32);
    let (ok, _) = unsafe { raw_syscall(SunlightSyscall::GpuMoveCursor, a1, 0, 0, 0, 0, 0, 0) };
    ok != 0
}

/// Information returned when the kernel VMware SVGA II driver is Active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvgaDisplayInfo {
    pub width: u32,
    pub height: u32,
    pub pitch_bytes: u32,
    pub bpp: u32,
    /// True when the Limine boot framebuffer physical base lies inside SVGA VRAM.
    pub boot_fb_in_vram: bool,
    pub max_width: u32,
    pub max_height: u32,
    /// Mapped FB capacity in bytes (may exceed the current mode for resize room).
    pub map_bytes: u64,
    pub framebuffer_offset: u32,
}

/// Result of [`svga_set_mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgaSetModeResult {
    Changed(SvgaDisplayInfo),
    Unchanged(SvgaDisplayInfo),
    Failed,
}

/// Query the VMware SVGA II backend. Returns `None` when the device is absent
/// or not fully activated (boot framebuffer remains the fallback).
pub fn svga_get_info() -> Option<SvgaDisplayInfo> {
    let (ok, msg) = unsafe { raw_syscall(SunlightSyscall::SvgaGetInfo, 0, 0, 0, 0, 0, 0, 0) };
    if ok == 0 {
        return None;
    }
    decode_svga_info(&msg.words)
}

fn decode_svga_info(words: &[u64; IPC_MAX_WORDS]) -> Option<SvgaDisplayInfo> {
    let wh = words[0];
    let pb = words[1];
    let packed = words[2];
    let width = (wh & 0xFFFF_FFFF) as u32;
    let height = (wh >> 32) as u32;
    let pitch_bytes = (pb & 0xFFFF_FFFF) as u32;
    let bpp = (pb >> 32) as u32;
    if width == 0 || height == 0 || pitch_bytes == 0 || bpp != 32 {
        return None;
    }
    let boot_fb_in_vram = (packed & 1) != 0;
    let max_width = ((packed >> 8) & 0xFF_FFFF) as u32;
    let max_height = (packed >> 32) as u32;
    let map_bytes = words[3] & 0xFFFF_FFFF;
    Some(SvgaDisplayInfo {
        width,
        height,
        pitch_bytes,
        bpp,
        boot_fb_in_vram,
        max_width: if max_width == 0 { width } else { max_width },
        max_height: if max_height == 0 { height } else { max_height },
        map_bytes,
        framebuffer_offset: (words[3] >> 32) as u32,
    })
}

/// Issue `SVGA_CMD_UPDATE` for a damage rectangle after pixels were written
/// to the visible framebuffer. Returns false if the backend is not ready or the
/// rect was rejected.
pub fn svga_update(x: u32, y: u32, w: u32, h: u32) -> bool {
    let a1 = (x as u64) | ((y as u64) << 32);
    let a2 = (w as u64) | ((h as u64) << 32);
    let (ok, _) = unsafe { raw_syscall(SunlightSyscall::SvgaUpdate, a1, a2, 0, 0, 0, 0, 0) };
    ok != 0
}

/// Request a VMware SVGA modeset using the VM resolution policy.
///
/// `host_w`/`host_h` are the desired host/window size (or current mode). The
/// kernel may upgrade below min-HD (1280×720) and will not exceed auto-max
/// (1920×1080) or device limits.
pub fn svga_set_mode(host_w: u32, host_h: u32) -> SvgaSetModeResult {
    let a1 = (host_w as u64) | ((host_h as u64) << 32);
    let (ok, msg) = unsafe { raw_syscall(SunlightSyscall::SvgaSetMode, a1, 0, 0, 0, 0, 0, 0) };
    match ok {
        1 => match decode_svga_info(&msg.words) {
            Some(info) => SvgaSetModeResult::Changed(info),
            None => SvgaSetModeResult::Failed,
        },
        2 => match decode_svga_info(&msg.words) {
            Some(info) => SvgaSetModeResult::Unchanged(info),
            None => SvgaSetModeResult::Failed,
        },
        _ => SvgaSetModeResult::Failed,
    }
}

pub fn svga_set_exact_mode(width: u32, height: u32) -> SvgaSetModeResult {
    let a1 = width as u64 | ((height as u64) << 32);
    let (ok, msg) = unsafe { raw_syscall(SunlightSyscall::SvgaSetMode, a1, 1, 0, 0, 0, 0, 0) };
    match ok {
        1 => decode_svga_info(&msg.words)
            .map(SvgaSetModeResult::Changed)
            .unwrap_or(SvgaSetModeResult::Failed),
        2 => decode_svga_info(&msg.words)
            .map(SvgaSetModeResult::Unchanged)
            .unwrap_or(SvgaSetModeResult::Failed),
        _ => SvgaSetModeResult::Failed,
    }
}

#[cfg(test)]
mod network_backend_tests {
    use super::{
        AdminState, InterfaceKind, LinkState, NetDeviceInfo, NetworkBackendKind, NetworkdMsg,
    };

    #[test]
    fn backend_metadata_produces_hardware_kind_labels() {
        assert_eq!(
            NetworkBackendKind::VirtioNet.interface_kind().label(),
            "VirtioNet"
        );
        assert_eq!(
            NetworkBackendKind::Vmxnet3.interface_kind().label(),
            "VMXNET3"
        );
        assert_eq!(InterfaceKind::from_u64(5).label(), "VMXNET3");
    }

    fn ready_info(kind: NetworkBackendKind) -> NetDeviceInfo {
        NetDeviceInfo {
            present: true,
            mac: [0x00, 0x50, 0x56, 0x12, 0x34, 0x56],
            link_up: false,
            backend: Some(kind),
            mtu: 1500,
            state: 4,
            error: 0,
            ..NetDeviceInfo::default()
        }
    }

    #[test]
    fn successful_backends_are_publishable_before_carrier_or_dhcp() {
        assert!(ready_info(NetworkBackendKind::VirtioNet).publishable());
        assert!(ready_info(NetworkBackendKind::Vmxnet3).publishable());
    }

    #[test]
    fn failed_or_incomplete_hardware_is_not_publishable() {
        let mut info = ready_info(NetworkBackendKind::Vmxnet3);
        info.state = 5;
        info.error = 5;
        assert!(!info.publishable());
        info.state = 4;
        info.error = 0;
        info.mac = [0; 6];
        assert!(!info.publishable());
    }

    #[test]
    fn admin_and_carrier_states_are_independent() {
        assert_ne!(AdminState::Disabled as u64, LinkState::NoCarrier as u64);
        assert_ne!(AdminState::Enabled as u64, LinkState::Carrier as u64);
    }

    #[test]
    fn semantic_network_errors_have_stable_codes() {
        assert_eq!(NetworkdMsg::ERR_NOT_FOUND, 1);
        assert_ne!(
            NetworkdMsg::ERR_BACKEND_UNAVAILABLE,
            NetworkdMsg::ERR_DEVICE_INITIALIZATION_FAILED
        );
        assert_ne!(
            NetworkdMsg::ERR_UNSUPPORTED_OPERATION,
            NetworkdMsg::ERR_IPC_FAILURE
        );
    }

    #[test]
    fn hardware_inventory_record_has_fixed_bounded_layout() {
        assert_eq!(core::mem::size_of::<crate::HardwareInventoryRecord>(), 112);
        assert_eq!(crate::HARDWARE_INVENTORY_MAX_RECORDS, 2050);
        let first = crate::HardwareInventoryRecord::pci_key(0, 0, 3, 0);
        let second = crate::HardwareInventoryRecord::pci_key(0, 0, 4, 0);
        assert_ne!(first, second);
    }
}
