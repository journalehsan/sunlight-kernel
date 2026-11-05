//! Strict configuration loading for the future `sunlight-sshd` service.
//!
//! This is deliberately not a general TOML parser.  It supports the documented
//! Phase 0.11 subset only: UTF-8, comments, whitespace, single-line quoted
//! strings, decimal integers, `true`/`false`, and top-level assignments.
//! Tables, arrays, inline tables, dotted keys, multiline strings, hexadecimal
//! integers, and interpolation are rejected rather than partially parsed.

use core::fmt;
use core::num::NonZeroU16;
use core::time::Duration;

use crate::{self as libc, Errno, Fd, Stat, FT_DIR, FT_FILE, MAX_PATH};

pub const SSH_CONFIG_PATH: &str = "/etc/sunlight/ssh.toml";
pub const SSH_CONFIG_DIRECTORY: &str = "/etc/sunlight";
pub const SSH_HOST_KEY_DIRECTORY: &str = "/etc/sunlight/";
pub const MAX_SSH_CONFIG_BYTES: usize = 16 * 1024;
pub const MAX_SSH_PATH_BYTES: usize = MAX_PATH - 1;
pub const MAX_AUTH_ATTEMPTS_HARD_LIMIT: u16 = 10;
pub const MAX_SSH_CONNECTIONS_HARD_LIMIT: u16 = 8;
pub const MAX_SESSIONS_PER_CONNECTION_SUPPORTED: u16 = 1;
pub const MAX_LOGIN_TIMEOUT_SECONDS_HARD_LIMIT: u16 = 300;
pub const SSH_PTY_CAPACITY: u16 = 16;
pub const SSH_PTY_RESERVE: u16 = 8;
pub const SSH_TCP_STREAM_CAPACITY: u16 = 128;
pub const SSH_WAIT_SET_CAPACITY: u16 = 32;

const REQUIRED_FIELD_MASK: u8 = 0xff;
const CONFIG_FILE_TYPE_BITS: u16 = 0o100_000;
const DIRECTORY_TYPE_BITS: u16 = 0o040_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshConfigField {
    ListenAddress,
    Port,
    HostKeyFile,
    PasswordAuthentication,
    MaxAuthAttempts,
    MaxConnections,
    MaxSessionsPerConnection,
    LoginTimeoutSeconds,
}

impl SshConfigField {
    const fn bit(self) -> u8 {
        1 << self.index()
    }

    const fn index(self) -> usize {
        match self {
            Self::ListenAddress => 0,
            Self::Port => 1,
            Self::HostKeyFile => 2,
            Self::PasswordAuthentication => 3,
            Self::MaxAuthAttempts => 4,
            Self::MaxConnections => 5,
            Self::MaxSessionsPerConnection => 6,
            Self::LoginTimeoutSeconds => 7,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::ListenAddress => "listen_address",
            Self::Port => "port",
            Self::HostKeyFile => "host_key_file",
            Self::PasswordAuthentication => "password_authentication",
            Self::MaxAuthAttempts => "max_auth_attempts",
            Self::MaxConnections => "max_connections",
            Self::MaxSessionsPerConnection => "max_sessions_per_connection",
            Self::LoginTimeoutSeconds => "login_timeout_seconds",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigErrorKind {
    FileMissing,
    FileTooLarge,
    FileReadFailed,
    InvalidUtf8,
    EmptyFile,
    UnexpectedFileType,
    InsecureFileOwner,
    InsecureFileMode,
    InsecureParentDirectory,
    SyntaxError,
    UnsupportedSyntax,
    DuplicateField,
    UnknownField,
    MissingField,
    WrongType,
    InvalidIpv4Address,
    UnsupportedAddress,
    InvalidPort,
    InvalidPath,
    ValueTooSmall,
    ValueTooLarge,
    UnsupportedValue,
    CrossFieldConflict,
    ResourceLimitExceeded,
    InternalError,
}

/// Bounded diagnostic data.  It never stores a raw configuration value or the
/// configuration text, so future secret-valued fields cannot be dumped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub kind: ConfigErrorKind,
    pub line: Option<u16>,
    pub column: Option<u16>,
    pub field: Option<SshConfigField>,
    pub first_definition_line: Option<u16>,
    pub missing_fields: u8,
    name: [u8; 64],
    name_len: u8,
}

impl ConfigError {
    const fn new(kind: ConfigErrorKind) -> Self {
        Self {
            kind,
            line: None,
            column: None,
            field: None,
            first_definition_line: None,
            missing_fields: 0,
            name: [0; 64],
            name_len: 0,
        }
    }

    fn at(mut self, line: usize, column: usize) -> Self {
        self.line = Some(line.min(u16::MAX as usize) as u16);
        self.column = Some(column.min(u16::MAX as usize) as u16);
        self
    }

    fn field(mut self, field: SshConfigField) -> Self {
        self.field = Some(field);
        self
    }

    fn name(mut self, name: &str) -> Self {
        let len = name.len().min(self.name.len());
        self.name[..len].copy_from_slice(&name.as_bytes()[..len]);
        self.name_len = len as u8;
        self
    }

    pub fn unknown_name(&self) -> Option<&str> {
        core::str::from_utf8(&self.name[..self.name_len as usize]).ok()
    }

    pub const fn missing(&self, field: SshConfigField) -> bool {
        self.missing_fields & field.bit() != 0
    }

    /// Render a bounded administrator diagnostic without retaining or printing
    /// the raw configuration buffer.
    pub fn write_diagnostic<W: fmt::Write>(&self, path: &str, out: &mut W) -> fmt::Result {
        write!(out, "{path}")?;
        if let Some(line) = self.line {
            write!(out, ":{line}")?;
            if let Some(column) = self.column {
                write!(out, ":{column}")?;
            }
        }
        write!(out, ": ")?;
        if let Some(field) = self.field {
            write!(out, "field `{}`: ", field.name())?;
        } else if matches!(self.kind, ConfigErrorKind::UnknownField) {
            if let Some(name) = self.unknown_name() {
                write!(out, "unknown field `{name}`: ")?;
            }
        }
        match self.kind {
            ConfigErrorKind::DuplicateField => write!(
                out,
                "duplicate definition; first defined at line {}",
                self.first_definition_line.unwrap_or(0)
            ),
            ConfigErrorKind::UnknownField => {
                write!(out, "rejected; service enablement is managed by sunlightd")
            }
            ConfigErrorKind::MissingField => write!(out, "required fields are missing"),
            ConfigErrorKind::WrongType => write!(out, "wrong TOML type"),
            ConfigErrorKind::InvalidIpv4Address => write!(out, "expected numeric IPv4"),
            ConfigErrorKind::UnsupportedAddress => write!(out, "unsupported listener address"),
            ConfigErrorKind::InvalidPort => write!(out, "expected integer 1..=65535"),
            ConfigErrorKind::ValueTooSmall => write!(out, "value is below the supported minimum"),
            ConfigErrorKind::ValueTooLarge => write!(out, "value exceeds the supported maximum"),
            ConfigErrorKind::UnsupportedValue => write!(out, "value is unsupported"),
            ConfigErrorKind::InvalidPath => write!(out, "invalid or out-of-policy path"),
            ConfigErrorKind::ResourceLimitExceeded => write!(out, "resource budget is unavailable"),
            ConfigErrorKind::CrossFieldConflict => write!(out, "configuration values conflict"),
            ConfigErrorKind::FileMissing => write!(out, "configuration file is missing"),
            ConfigErrorKind::FileTooLarge => write!(out, "configuration file exceeds 16 KiB"),
            ConfigErrorKind::FileReadFailed => {
                write!(out, "configuration file read or close failed")
            }
            ConfigErrorKind::InvalidUtf8 => write!(out, "configuration file is not valid UTF-8"),
            ConfigErrorKind::EmptyFile => write!(out, "configuration file is empty"),
            ConfigErrorKind::UnexpectedFileType => {
                write!(out, "configuration is not a regular file")
            }
            ConfigErrorKind::InsecureFileOwner => write!(out, "configuration is not root-owned"),
            ConfigErrorKind::InsecureFileMode => write!(out, "configuration has insecure metadata"),
            ConfigErrorKind::InsecureParentDirectory => {
                write!(out, "configuration parent is insecure")
            }
            ConfigErrorKind::SyntaxError => write!(out, "invalid supported TOML syntax"),
            ConfigErrorKind::UnsupportedSyntax => write!(out, "unsupported TOML syntax"),
            ConfigErrorKind::InternalError => write!(out, "internal configuration policy error"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileMetadata {
    pub size: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
    pub file_type: u8,
    pub nlinks: u32,
}

impl From<Stat> for FileMetadata {
    fn from(stat: Stat) -> Self {
        Self {
            size: stat.size,
            uid: stat.uid,
            gid: stat.gid,
            mode: stat.mode,
            file_type: stat.file_type,
            nlinks: stat.nlinks,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigSourceError {
    Missing,
    Again,
    Failed,
}

/// Abstracted file access keeps metadata and complete-read behavior testable.
pub trait ConfigFileSource {
    type Handle: Copy;

    fn stat(&mut self, path: &str) -> Result<FileMetadata, ConfigSourceError>;
    fn open(&mut self, path: &str) -> Result<Self::Handle, ConfigSourceError>;
    fn fstat(&mut self, handle: Self::Handle) -> Result<FileMetadata, ConfigSourceError>;
    fn read(&mut self, handle: Self::Handle, out: &mut [u8]) -> Result<usize, ConfigSourceError>;
    fn close(&mut self, handle: Self::Handle) -> Result<(), ConfigSourceError>;
}

pub struct SystemConfigSource;

impl ConfigFileSource for SystemConfigSource {
    type Handle = Fd;

    fn stat(&mut self, path: &str) -> Result<FileMetadata, ConfigSourceError> {
        // The current syscall ABI has no errno details.  A path-stat failure is
        // reported as missing; future errno-rich VFS support can refine this.
        libc::stat(path.as_bytes())
            .map(FileMetadata::from)
            .map_err(|_| ConfigSourceError::Missing)
    }

    fn open(&mut self, path: &str) -> Result<Self::Handle, ConfigSourceError> {
        libc::open(path.as_bytes()).map_err(|_| ConfigSourceError::Failed)
    }

    fn fstat(&mut self, handle: Self::Handle) -> Result<FileMetadata, ConfigSourceError> {
        libc::fstat(handle)
            .map(FileMetadata::from)
            .map_err(|_| ConfigSourceError::Failed)
    }

    fn read(&mut self, handle: Self::Handle, out: &mut [u8]) -> Result<usize, ConfigSourceError> {
        libc::read(handle, out).map_err(|error| match error {
            Errno::Again => ConfigSourceError::Again,
            _ => ConfigSourceError::Failed,
        })
    }

    fn close(&mut self, handle: Self::Handle) -> Result<(), ConfigSourceError> {
        libc::close(handle).map_err(|_| ConfigSourceError::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundedText<const N: usize> {
    bytes: [u8; N],
    len: u16,
}

impl<const N: usize> BoundedText<N> {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        if value.len() > N || value.len() > u16::MAX as usize {
            return None;
        }
        let mut result = Self::empty();
        result.bytes[..value.len()].copy_from_slice(value.as_bytes());
        result.len = value.len() as u16;
        Some(result)
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

/// Decoded file values before semantic validation.  This must not cross into
/// network, PTY, authentication, or host-key code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawSshConfig {
    pub listen_address: BoundedText<15>,
    pub port: i64,
    pub host_key_file: BoundedText<MAX_SSH_PATH_BYTES>,
    pub password_authentication: bool,
    pub max_auth_attempts: i64,
    pub max_connections: i64,
    pub max_sessions_per_connection: i64,
    pub login_timeout_seconds: i64,
    locations: [SourceLocation; 8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceLocation {
    line: u16,
    column: u16,
}

impl SourceLocation {
    const UNKNOWN: Self = Self { line: 0, column: 0 };
}

impl RawSshConfig {
    fn error(&self, kind: ConfigErrorKind, field: SshConfigField) -> ConfigError {
        let location = self.locations[field.index()];
        let error = ConfigError::new(kind).field(field);
        if location.line == 0 {
            error
        } else {
            error.at(location.line as usize, location.column as usize)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidatedAbsolutePath {
    value: BoundedText<MAX_SSH_PATH_BYTES>,
}

impl ValidatedAbsolutePath {
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidatedSshConfig {
    pub listen_address: [u8; 4],
    pub port: NonZeroU16,
    pub host_key_file: ValidatedAbsolutePath,
    pub password_authentication: bool,
    pub max_auth_attempts: NonZeroU16,
    pub max_connections: NonZeroU16,
    pub max_sessions_per_connection: NonZeroU16,
    pub login_timeout: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SshRuntimeLimits {
    pub tcp_stream_capacity: u16,
    pub tcp_stream_reserve: u16,
    pub wait_set_capacity: u16,
    pub pty_capacity: u16,
    pub pty_reserve: u16,
}

impl Default for SshRuntimeLimits {
    fn default() -> Self {
        Self {
            tcp_stream_capacity: SSH_TCP_STREAM_CAPACITY,
            tcp_stream_reserve: 16,
            wait_set_capacity: SSH_WAIT_SET_CAPACITY,
            pty_capacity: SSH_PTY_CAPACITY,
            pty_reserve: SSH_PTY_RESERVE,
        }
    }
}

/// Production entry point.  It consumes no heap and uses one bounded 16 KiB
/// stack buffer, which a future service may replace with caller-owned storage.
pub fn load_and_validate_ssh_config(
    path: &str,
    limits: &SshRuntimeLimits,
) -> Result<ValidatedSshConfig, ConfigError> {
    let mut source = SystemConfigSource;
    let mut buffer = [0u8; MAX_SSH_CONFIG_BYTES];
    load_and_validate_ssh_config_from(&mut source, path, limits, &mut buffer)
}

pub fn load_and_validate_ssh_config_from<S: ConfigFileSource>(
    source: &mut S,
    path: &str,
    limits: &SshRuntimeLimits,
    buffer: &mut [u8],
) -> Result<ValidatedSshConfig, ConfigError> {
    if buffer.len() < MAX_SSH_CONFIG_BYTES {
        return Err(ConfigError::new(ConfigErrorKind::InternalError));
    }
    validate_absolute_path(path).map_err(|_| ConfigError::new(ConfigErrorKind::InvalidPath))?;
    validate_parent(source, path)?;
    let metadata = source
        .stat(path)
        .map_err(|_| ConfigError::new(ConfigErrorKind::FileMissing))?;
    validate_file_metadata(metadata)?;
    let handle = source
        .open(path)
        .map_err(|_| ConfigError::new(ConfigErrorKind::FileReadFailed))?;
    let result = (|| {
        let opened_metadata = source
            .fstat(handle)
            .map_err(|_| ConfigError::new(ConfigErrorKind::FileReadFailed))?;
        validate_file_metadata(opened_metadata)?;
        let length = usize::try_from(opened_metadata.size)
            .map_err(|_| ConfigError::new(ConfigErrorKind::FileTooLarge))?;
        if length == 0 {
            return Err(ConfigError::new(ConfigErrorKind::EmptyFile));
        }
        if length > MAX_SSH_CONFIG_BYTES {
            return Err(ConfigError::new(ConfigErrorKind::FileTooLarge));
        }
        let mut read_total = 0;
        while read_total < length {
            match source.read(handle, &mut buffer[read_total..length]) {
                Ok(0) => return Err(ConfigError::new(ConfigErrorKind::FileReadFailed)),
                Ok(read) if read <= length - read_total => read_total += read,
                Ok(_) => return Err(ConfigError::new(ConfigErrorKind::FileReadFailed)),
                Err(ConfigSourceError::Again) => continue,
                Err(_) => return Err(ConfigError::new(ConfigErrorKind::FileReadFailed)),
            }
        }
        let mut extra = [0u8; 1];
        loop {
            match source.read(handle, &mut extra) {
                Ok(0) => break,
                Ok(_) => return Err(ConfigError::new(ConfigErrorKind::FileTooLarge)),
                Err(ConfigSourceError::Again) => continue,
                Err(_) => return Err(ConfigError::new(ConfigErrorKind::FileReadFailed)),
            }
        }
        let text = core::str::from_utf8(&buffer[..length])
            .map_err(|_| ConfigError::new(ConfigErrorKind::InvalidUtf8))?;
        validate_raw_ssh_config(parse_raw_ssh_config(text)?, path, limits)
    })();
    match source.close(handle) {
        Err(_) => Err(ConfigError::new(ConfigErrorKind::FileReadFailed)),
        Ok(()) => result,
    }
}

pub fn parse_raw_ssh_config(text: &str) -> Result<RawSshConfig, ConfigError> {
    if text.is_empty() {
        return Err(ConfigError::new(ConfigErrorKind::EmptyFile));
    }
    let mut builder = RawBuilder::empty();
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            return Err(ConfigError::new(ConfigErrorKind::UnknownField)
                .at(line_number, 1)
                .name(line));
        }
        let Some(separator) = line.find('=') else {
            return Err(ConfigError::new(ConfigErrorKind::SyntaxError).at(line_number, 1));
        };
        let key = line[..separator].trim();
        let value = line[separator + 1..].trim();
        if key.is_empty() || !key.bytes().all(is_key_byte) {
            return Err(ConfigError::new(ConfigErrorKind::SyntaxError)
                .at(line_number, 1)
                .name(key));
        }
        let field = field_from_name(key).ok_or_else(|| {
            ConfigError::new(ConfigErrorKind::UnknownField)
                .at(line_number, 1)
                .name(key)
        })?;
        if builder.seen & field.bit() != 0 {
            return Err(ConfigError {
                first_definition_line: Some(builder.lines[field.index()]),
                ..ConfigError::new(ConfigErrorKind::DuplicateField)
                    .at(line_number, 1)
                    .field(field)
                    .name(field.name())
            });
        }
        builder.seen |= field.bit();
        builder.lines[field.index()] = line_number.min(u16::MAX as usize) as u16;
        builder.locations[field.index()] = SourceLocation {
            line: line_number.min(u16::MAX as usize) as u16,
            column: (separator + 2).min(u16::MAX as usize) as u16,
        };
        builder.set(field, value, line_number, separator + 2)?;
    }
    builder.finish()
}

pub fn validate_raw_ssh_config(
    raw: RawSshConfig,
    config_path: &str,
    limits: &SshRuntimeLimits,
) -> Result<ValidatedSshConfig, ConfigError> {
    validate_runtime_limits(*limits)?;
    let listen_address = validate_ipv4(raw.listen_address.as_str())
        .map_err(|error| raw.error(error.kind, SshConfigField::ListenAddress))?;
    let port = bounded_nonzero(
        raw.port,
        1,
        u16::MAX as i64,
        SshConfigField::Port,
        ConfigErrorKind::InvalidPort,
        ConfigErrorKind::InvalidPort,
    )
    .map_err(|error| raw.error(error.kind, SshConfigField::Port))?;
    let max_auth_attempts = bounded_nonzero(
        raw.max_auth_attempts,
        1,
        MAX_AUTH_ATTEMPTS_HARD_LIMIT as i64,
        SshConfigField::MaxAuthAttempts,
        ConfigErrorKind::ValueTooSmall,
        ConfigErrorKind::ValueTooLarge,
    )
    .map_err(|error| raw.error(error.kind, SshConfigField::MaxAuthAttempts))?;
    let max_connections = bounded_nonzero(
        raw.max_connections,
        1,
        MAX_SSH_CONNECTIONS_HARD_LIMIT as i64,
        SshConfigField::MaxConnections,
        ConfigErrorKind::ValueTooSmall,
        ConfigErrorKind::ValueTooLarge,
    )
    .map_err(|error| raw.error(error.kind, SshConfigField::MaxConnections))?;
    let max_sessions_per_connection = bounded_nonzero(
        raw.max_sessions_per_connection,
        1,
        MAX_SESSIONS_PER_CONNECTION_SUPPORTED as i64,
        SshConfigField::MaxSessionsPerConnection,
        ConfigErrorKind::ValueTooSmall,
        ConfigErrorKind::UnsupportedValue,
    )
    .map_err(|error| raw.error(error.kind, SshConfigField::MaxSessionsPerConnection))?;
    let timeout = bounded_nonzero(
        raw.login_timeout_seconds,
        1,
        MAX_LOGIN_TIMEOUT_SECONDS_HARD_LIMIT as i64,
        SshConfigField::LoginTimeoutSeconds,
        ConfigErrorKind::ValueTooSmall,
        ConfigErrorKind::ValueTooLarge,
    )
    .map_err(|error| raw.error(error.kind, SshConfigField::LoginTimeoutSeconds))?;
    let host_key_file = validate_host_key_path(raw.host_key_file, config_path)
        .map_err(|error| raw.error(error.kind, SshConfigField::HostKeyFile))?;
    if max_connections.get() > limits.tcp_stream_capacity - limits.tcp_stream_reserve
        || max_connections.get().saturating_add(1) > limits.wait_set_capacity
    {
        return Err(raw.error(
            ConfigErrorKind::ResourceLimitExceeded,
            SshConfigField::MaxConnections,
        ));
    }
    let total_sessions = max_connections
        .get()
        .checked_mul(max_sessions_per_connection.get())
        .ok_or_else(|| {
            raw.error(
                ConfigErrorKind::CrossFieldConflict,
                SshConfigField::MaxConnections,
            )
        })?;
    if total_sessions > limits.pty_capacity - limits.pty_reserve {
        return Err(raw.error(
            ConfigErrorKind::ResourceLimitExceeded,
            SshConfigField::MaxSessionsPerConnection,
        ));
    }
    Ok(ValidatedSshConfig {
        listen_address,
        port,
        host_key_file,
        password_authentication: raw.password_authentication,
        max_auth_attempts,
        max_connections,
        max_sessions_per_connection,
        login_timeout: Duration::from_secs(u64::from(timeout.get())),
    })
}

/// Future daemon dependencies accept only `ValidatedSshConfig`, structurally
/// preventing raw parsed values from reaching host-key or listener operations.
pub trait ValidatedSshStartup {
    type Error;
    fn load_or_create_host_key(&mut self, config: &ValidatedSshConfig) -> Result<(), Self::Error>;
    fn create_and_bind_listener(&mut self, config: &ValidatedSshConfig) -> Result<(), Self::Error>;
    fn publish_ready(&mut self, config: &ValidatedSshConfig) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshStartupError<E> {
    Configuration(ConfigError),
    Dependency(E),
}

pub fn validate_then_start_ssh<S: ConfigFileSource, D: ValidatedSshStartup>(
    source: &mut S,
    path: &str,
    limits: &SshRuntimeLimits,
    buffer: &mut [u8],
    dependencies: &mut D,
) -> Result<ValidatedSshConfig, SshStartupError<D::Error>> {
    let config = load_and_validate_ssh_config_from(source, path, limits, buffer)
        .map_err(SshStartupError::Configuration)?;
    dependencies
        .load_or_create_host_key(&config)
        .map_err(SshStartupError::Dependency)?;
    dependencies
        .create_and_bind_listener(&config)
        .map_err(SshStartupError::Dependency)?;
    dependencies
        .publish_ready(&config)
        .map_err(SshStartupError::Dependency)?;
    Ok(config)
}

struct RawBuilder {
    listen_address: Option<BoundedText<15>>,
    port: Option<i64>,
    host_key_file: Option<BoundedText<MAX_SSH_PATH_BYTES>>,
    password_authentication: Option<bool>,
    max_auth_attempts: Option<i64>,
    max_connections: Option<i64>,
    max_sessions_per_connection: Option<i64>,
    login_timeout_seconds: Option<i64>,
    seen: u8,
    lines: [u16; 8],
    locations: [SourceLocation; 8],
}

impl RawBuilder {
    const fn empty() -> Self {
        Self {
            listen_address: None,
            port: None,
            host_key_file: None,
            password_authentication: None,
            max_auth_attempts: None,
            max_connections: None,
            max_sessions_per_connection: None,
            login_timeout_seconds: None,
            seen: 0,
            lines: [0; 8],
            locations: [SourceLocation::UNKNOWN; 8],
        }
    }

    fn set(
        &mut self,
        field: SshConfigField,
        value: &str,
        line: usize,
        column: usize,
    ) -> Result<(), ConfigError> {
        let error = |kind| ConfigError::new(kind).at(line, column).field(field);
        match field {
            SshConfigField::ListenAddress => {
                self.listen_address = Some(parse_string(value, error)?)
            }
            SshConfigField::Port => self.port = Some(parse_integer(value, error)?),
            SshConfigField::HostKeyFile => self.host_key_file = Some(parse_string(value, error)?),
            SshConfigField::PasswordAuthentication => {
                self.password_authentication = Some(parse_boolean(value, error)?)
            }
            SshConfigField::MaxAuthAttempts => {
                self.max_auth_attempts = Some(parse_integer(value, error)?)
            }
            SshConfigField::MaxConnections => {
                self.max_connections = Some(parse_integer(value, error)?)
            }
            SshConfigField::MaxSessionsPerConnection => {
                self.max_sessions_per_connection = Some(parse_integer(value, error)?)
            }
            SshConfigField::LoginTimeoutSeconds => {
                self.login_timeout_seconds = Some(parse_integer(value, error)?)
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<RawSshConfig, ConfigError> {
        if self.seen != REQUIRED_FIELD_MASK {
            return Err(ConfigError {
                missing_fields: REQUIRED_FIELD_MASK & !self.seen,
                ..ConfigError::new(ConfigErrorKind::MissingField)
            });
        }
        Ok(RawSshConfig {
            listen_address: self.listen_address.unwrap_or(BoundedText::empty()),
            port: self.port.unwrap_or(0),
            host_key_file: self.host_key_file.unwrap_or(BoundedText::empty()),
            password_authentication: self.password_authentication.unwrap_or(false),
            max_auth_attempts: self.max_auth_attempts.unwrap_or(0),
            max_connections: self.max_connections.unwrap_or(0),
            max_sessions_per_connection: self.max_sessions_per_connection.unwrap_or(0),
            login_timeout_seconds: self.login_timeout_seconds.unwrap_or(0),
            locations: self.locations,
        })
    }
}

fn validate_parent<S: ConfigFileSource>(source: &mut S, path: &str) -> Result<(), ConfigError> {
    let parent = match path.rfind('/') {
        Some(0) => "/",
        Some(index) => &path[..index],
        None => return Err(ConfigError::new(ConfigErrorKind::InvalidPath)),
    };
    let metadata = source
        .stat(parent)
        .map_err(|_| ConfigError::new(ConfigErrorKind::InsecureParentDirectory))?;
    if metadata.file_type != FT_DIR
        || metadata.mode & 0o170_000 != DIRECTORY_TYPE_BITS
        || metadata.uid != 0
        || metadata.gid != 0
        || metadata.mode & 0o022 != 0
    {
        return Err(ConfigError::new(ConfigErrorKind::InsecureParentDirectory));
    }
    Ok(())
}

fn validate_file_metadata(metadata: FileMetadata) -> Result<(), ConfigError> {
    if metadata.file_type != FT_FILE || metadata.mode & 0o170_000 != CONFIG_FILE_TYPE_BITS {
        return Err(ConfigError::new(ConfigErrorKind::UnexpectedFileType));
    }
    if metadata.uid != 0 {
        return Err(ConfigError::new(ConfigErrorKind::InsecureFileOwner));
    }
    if metadata.nlinks != 1 || metadata.mode & 0o022 != 0 {
        return Err(ConfigError::new(ConfigErrorKind::InsecureFileMode));
    }
    if metadata.size > MAX_SSH_CONFIG_BYTES as u64 {
        return Err(ConfigError::new(ConfigErrorKind::FileTooLarge));
    }
    Ok(())
}

fn validate_runtime_limits(limits: SshRuntimeLimits) -> Result<(), ConfigError> {
    if limits.tcp_stream_capacity == 0
        || limits.tcp_stream_reserve >= limits.tcp_stream_capacity
        || limits.wait_set_capacity < 2
        || limits.pty_capacity == 0
        || limits.pty_reserve >= limits.pty_capacity
    {
        return Err(ConfigError::new(ConfigErrorKind::InternalError));
    }
    Ok(())
}

fn validate_ipv4(value: &str) -> Result<[u8; 4], ConfigError> {
    if value.is_empty() || value.trim() != value {
        return Err(ConfigError::new(ConfigErrorKind::InvalidIpv4Address)
            .field(SshConfigField::ListenAddress));
    }
    let mut parts = value.split('.');
    let mut octets = [0u8; 4];
    for octet in &mut octets {
        let part = parts.next().ok_or_else(|| {
            ConfigError::new(ConfigErrorKind::InvalidIpv4Address)
                .field(SshConfigField::ListenAddress)
        })?;
        if part.is_empty()
            || part.len() > 3
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return Err(ConfigError::new(ConfigErrorKind::InvalidIpv4Address)
                .field(SshConfigField::ListenAddress));
        }
        *octet = part.parse::<u8>().map_err(|_| {
            ConfigError::new(ConfigErrorKind::InvalidIpv4Address)
                .field(SshConfigField::ListenAddress)
        })?;
    }
    if parts.next().is_some() {
        return Err(ConfigError::new(ConfigErrorKind::InvalidIpv4Address)
            .field(SshConfigField::ListenAddress));
    }
    if (224..=239).contains(&octets[0]) || octets == [255, 255, 255, 255] {
        return Err(ConfigError::new(ConfigErrorKind::UnsupportedAddress)
            .field(SshConfigField::ListenAddress));
    }
    Ok(octets)
}

fn validate_host_key_path(
    value: BoundedText<MAX_SSH_PATH_BYTES>,
    config_path: &str,
) -> Result<ValidatedAbsolutePath, ConfigError> {
    let path = value.as_str();
    validate_absolute_path(path).map_err(|_| {
        ConfigError::new(ConfigErrorKind::InvalidPath).field(SshConfigField::HostKeyFile)
    })?;
    if !path.starts_with(SSH_HOST_KEY_DIRECTORY)
        || path == SSH_HOST_KEY_DIRECTORY
        || path == config_path
        || path[SSH_HOST_KEY_DIRECTORY.len()..].contains('/')
    {
        return Err(
            ConfigError::new(ConfigErrorKind::InvalidPath).field(SshConfigField::HostKeyFile)
        );
    }
    Ok(ValidatedAbsolutePath { value })
}

fn validate_absolute_path(path: &str) -> Result<(), ()> {
    if path.is_empty()
        || path.len() > MAX_SSH_PATH_BYTES
        || !path.starts_with('/')
        || path.ends_with('/')
        || path.as_bytes().contains(&0)
        || path.split('/').any(|part| part == "." || part == "..")
    {
        Err(())
    } else {
        Ok(())
    }
}

fn bounded_nonzero(
    value: i64,
    minimum: i64,
    maximum: i64,
    field: SshConfigField,
    too_small: ConfigErrorKind,
    too_large: ConfigErrorKind,
) -> Result<NonZeroU16, ConfigError> {
    if value < minimum {
        return Err(ConfigError::new(too_small).field(field));
    }
    if value > maximum {
        return Err(ConfigError::new(too_large).field(field));
    }
    NonZeroU16::new(value as u16).ok_or_else(|| ConfigError::new(too_small).field(field))
}

fn parse_string<const N: usize>(
    value: &str,
    error: impl Fn(ConfigErrorKind) -> ConfigError,
) -> Result<BoundedText<N>, ConfigError> {
    if !value.starts_with('"') {
        return Err(error(type_or_syntax(value)));
    }
    if value.len() < 2 || !value.ends_with('"') {
        return Err(error(ConfigErrorKind::SyntaxError));
    }
    let inner = &value[1..value.len() - 1];
    let mut out = [0u8; N];
    let mut output_len = 0;
    let bytes = inner.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' || byte < 0x20 {
            return Err(error(ConfigErrorKind::SyntaxError));
        }
        let decoded = if byte == b'\\' {
            index += 1;
            match bytes.get(index).copied() {
                Some(b'"') => b'"',
                Some(b'\\') => b'\\',
                Some(b'n') => b'\n',
                Some(b'r') => b'\r',
                Some(b't') => b'\t',
                Some(_) => return Err(error(ConfigErrorKind::UnsupportedSyntax)),
                None => return Err(error(ConfigErrorKind::SyntaxError)),
            }
        } else {
            byte
        };
        if output_len == N {
            return Err(error(ConfigErrorKind::ValueTooLarge));
        }
        out[output_len] = decoded;
        output_len += 1;
        index += 1;
    }
    let decoded = core::str::from_utf8(&out[..output_len])
        .map_err(|_| error(ConfigErrorKind::InvalidUtf8))?;
    BoundedText::from_str(decoded).ok_or_else(|| error(ConfigErrorKind::ValueTooLarge))
}

fn parse_integer(
    value: &str,
    error: impl Fn(ConfigErrorKind) -> ConfigError,
) -> Result<i64, ConfigError> {
    if value.starts_with('"') || matches!(value, "true" | "false") {
        return Err(error(ConfigErrorKind::WrongType));
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty()
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return Err(error(type_or_syntax(value)));
    }
    value
        .parse::<i64>()
        .map_err(|_| error(ConfigErrorKind::ValueTooLarge))
}

fn parse_boolean(
    value: &str,
    error: impl Fn(ConfigErrorKind) -> ConfigError,
) -> Result<bool, ConfigError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(error(ConfigErrorKind::WrongType)),
    }
}

fn type_or_syntax(value: &str) -> ConfigErrorKind {
    if value.starts_with('[') || value.starts_with('{') {
        ConfigErrorKind::UnsupportedSyntax
    } else {
        ConfigErrorKind::WrongType
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in line.as_bytes().iter().enumerate() {
        if in_string && escaped {
            escaped = false;
            continue;
        }
        match *byte {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'#' if !in_string => return &line[..index],
            _ => {}
        }
    }
    line
}

fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn field_from_name(name: &str) -> Option<SshConfigField> {
    match name {
        "listen_address" => Some(SshConfigField::ListenAddress),
        "port" => Some(SshConfigField::Port),
        "host_key_file" => Some(SshConfigField::HostKeyFile),
        "password_authentication" => Some(SshConfigField::PasswordAuthentication),
        "max_auth_attempts" => Some(SshConfigField::MaxAuthAttempts),
        "max_connections" => Some(SshConfigField::MaxConnections),
        "max_sessions_per_connection" => Some(SshConfigField::MaxSessionsPerConnection),
        "login_timeout_seconds" => Some(SshConfigField::LoginTimeoutSeconds),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"# comment
listen_address = "0.0.0.0"
port = 22
host_key_file = "/etc/sunlight/ssh_host_ed25519_key"
password_authentication = true
max_auth_attempts = 3
max_connections = 8
max_sessions_per_connection = 1
login_timeout_seconds = 30
"#;

    #[derive(Clone, Copy)]
    struct FakeSource {
        bytes: &'static [u8],
        file: FileMetadata,
        parent: FileMetadata,
        present: bool,
        cursor: usize,
        closed: bool,
    }

    impl FakeSource {
        fn valid(bytes: &'static [u8]) -> Self {
            Self {
                bytes,
                file: FileMetadata {
                    size: bytes.len() as u64,
                    uid: 0,
                    gid: 0,
                    mode: CONFIG_FILE_TYPE_BITS | 0o644,
                    file_type: FT_FILE,
                    nlinks: 1,
                },
                parent: FileMetadata {
                    size: 0,
                    uid: 0,
                    gid: 0,
                    mode: DIRECTORY_TYPE_BITS | 0o755,
                    file_type: FT_DIR,
                    nlinks: 2,
                },
                present: true,
                cursor: 0,
                closed: false,
            }
        }
    }

    impl ConfigFileSource for FakeSource {
        type Handle = u8;

        fn stat(&mut self, path: &str) -> Result<FileMetadata, ConfigSourceError> {
            match path {
                SSH_CONFIG_DIRECTORY => Ok(self.parent),
                SSH_CONFIG_PATH if self.present => Ok(self.file),
                _ => Err(ConfigSourceError::Missing),
            }
        }

        fn open(&mut self, _: &str) -> Result<Self::Handle, ConfigSourceError> {
            if self.present {
                Ok(1)
            } else {
                Err(ConfigSourceError::Missing)
            }
        }

        fn fstat(&mut self, _: Self::Handle) -> Result<FileMetadata, ConfigSourceError> {
            Ok(self.file)
        }

        fn read(&mut self, _: Self::Handle, out: &mut [u8]) -> Result<usize, ConfigSourceError> {
            if self.cursor == self.bytes.len() {
                return Ok(0);
            }
            let count = (self.bytes.len() - self.cursor).min(out.len()).min(3);
            out[..count].copy_from_slice(&self.bytes[self.cursor..self.cursor + count]);
            self.cursor += count;
            Ok(count)
        }

        fn close(&mut self, _: Self::Handle) -> Result<(), ConfigSourceError> {
            self.closed = true;
            Ok(())
        }
    }

    fn validate(text: &str) -> Result<ValidatedSshConfig, ConfigError> {
        validate_raw_ssh_config(
            parse_raw_ssh_config(text)?,
            SSH_CONFIG_PATH,
            &SshRuntimeLimits::default(),
        )
    }

    #[test]
    fn parses_and_validates_documented_example() {
        let config = validate(VALID).unwrap();
        assert_eq!(config.listen_address, [0, 0, 0, 0]);
        assert_eq!(config.port.get(), 22);
        assert_eq!(
            config.host_key_file.as_str(),
            "/etc/sunlight/ssh_host_ed25519_key"
        );
        assert!(config.password_authentication);
        assert_eq!(config.max_connections.get(), 8);
        assert_eq!(config.login_timeout.as_secs(), 30);
    }

    #[test]
    fn loader_reads_complete_file_and_closes() {
        let mut source = FakeSource::valid(VALID.as_bytes());
        let mut buffer = [0; MAX_SSH_CONFIG_BYTES];
        assert_eq!(
            load_and_validate_ssh_config_from(
                &mut source,
                SSH_CONFIG_PATH,
                &SshRuntimeLimits::default(),
                &mut buffer,
            )
            .unwrap()
            .port
            .get(),
            22
        );
        assert!(source.closed);
    }

    #[test]
    fn missing_empty_large_and_insecure_metadata_fail() {
        let mut source = FakeSource::valid(VALID.as_bytes());
        source.present = false;
        let mut buffer = [0; MAX_SSH_CONFIG_BYTES];
        assert_eq!(
            load_and_validate_ssh_config_from(
                &mut source,
                SSH_CONFIG_PATH,
                &SshRuntimeLimits::default(),
                &mut buffer
            )
            .unwrap_err()
            .kind,
            ConfigErrorKind::FileMissing
        );
        let mut source = FakeSource::valid(b"");
        source.file.size = 0;
        assert_eq!(
            load_and_validate_ssh_config_from(
                &mut source,
                SSH_CONFIG_PATH,
                &SshRuntimeLimits::default(),
                &mut buffer
            )
            .unwrap_err()
            .kind,
            ConfigErrorKind::EmptyFile
        );
        let mut source = FakeSource::valid(VALID.as_bytes());
        source.file.mode = CONFIG_FILE_TYPE_BITS | 0o666;
        assert_eq!(
            load_and_validate_ssh_config_from(
                &mut source,
                SSH_CONFIG_PATH,
                &SshRuntimeLimits::default(),
                &mut buffer
            )
            .unwrap_err()
            .kind,
            ConfigErrorKind::InsecureFileMode
        );
        let mut source = FakeSource::valid(VALID.as_bytes());
        source.parent.mode = DIRECTORY_TYPE_BITS | 0o777;
        assert_eq!(
            load_and_validate_ssh_config_from(
                &mut source,
                SSH_CONFIG_PATH,
                &SshRuntimeLimits::default(),
                &mut buffer
            )
            .unwrap_err()
            .kind,
            ConfigErrorKind::InsecureParentDirectory
        );
    }

    #[test]
    fn rejects_duplicate_unknown_missing_types_and_unsupported_syntax() {
        let duplicate = r#"listen_address = "0.0.0.0"
port = 22
host_key_file = "/etc/sunlight/ssh_host_ed25519_key"
password_authentication = true
max_auth_attempts = 3
max_connections = 8
max_sessions_per_connection = 1
login_timeout_seconds = 30
port = 2222
"#;
        let error = parse_raw_ssh_config(duplicate).unwrap_err();
        assert_eq!(error.kind, ConfigErrorKind::DuplicateField);
        assert_eq!(error.field, Some(SshConfigField::Port));
        assert_eq!(error.first_definition_line, Some(2));
        assert_eq!(error.line, Some(9));
        let error = parse_raw_ssh_config("enabled = true\n").unwrap_err();
        assert_eq!(error.kind, ConfigErrorKind::UnknownField);
        assert_eq!(error.unknown_name(), Some("enabled"));
        let error = parse_raw_ssh_config("port = 22\n").unwrap_err();
        assert!(error.missing(SshConfigField::ListenAddress));
        assert_eq!(
            parse_raw_ssh_config(&VALID.replace("port = 22", "port = \"22\""))
                .unwrap_err()
                .kind,
            ConfigErrorKind::WrongType
        );
        assert_eq!(
            parse_raw_ssh_config(&VALID.replace("max_connections = 8", "max_connections = []"))
                .unwrap_err()
                .kind,
            ConfigErrorKind::UnsupportedSyntax
        );
    }

    #[test]
    fn validates_values_and_resource_relationships() {
        assert_eq!(
            validate(&VALID.replace("port = 22", "port = 0"))
                .unwrap_err()
                .kind,
            ConfigErrorKind::InvalidPort
        );
        assert_eq!(
            validate(&VALID.replace("0.0.0.0", "::1")).unwrap_err().kind,
            ConfigErrorKind::InvalidIpv4Address
        );
        assert_eq!(
            validate(&VALID.replace("0.0.0.0", "255.255.255.255"))
                .unwrap_err()
                .kind,
            ConfigErrorKind::UnsupportedAddress
        );
        assert_eq!(
            validate(&VALID.replace("/etc/sunlight/ssh_host_ed25519_key", "/tmp/key"))
                .unwrap_err()
                .kind,
            ConfigErrorKind::InvalidPath
        );
        assert_eq!(
            validate(&VALID.replace("max_auth_attempts = 3", "max_auth_attempts = 11"))
                .unwrap_err()
                .kind,
            ConfigErrorKind::ValueTooLarge
        );
        assert_eq!(
            validate(&VALID.replace(
                "max_sessions_per_connection = 1",
                "max_sessions_per_connection = 2"
            ))
            .unwrap_err()
            .kind,
            ConfigErrorKind::UnsupportedValue
        );
        let raw = parse_raw_ssh_config(VALID).unwrap();
        let small_pty_budget = SshRuntimeLimits {
            pty_capacity: 8,
            pty_reserve: 4,
            ..SshRuntimeLimits::default()
        };
        assert_eq!(
            validate_raw_ssh_config(raw, SSH_CONFIG_PATH, &small_pty_budget)
                .unwrap_err()
                .kind,
            ConfigErrorKind::ResourceLimitExceeded
        );
    }

    #[test]
    fn startup_guard_blocks_all_side_effects_on_invalid_configuration() {
        struct Dependencies(u8);
        impl ValidatedSshStartup for Dependencies {
            type Error = ();
            fn load_or_create_host_key(&mut self, _: &ValidatedSshConfig) -> Result<(), ()> {
                self.0 += 1;
                Ok(())
            }
            fn create_and_bind_listener(&mut self, _: &ValidatedSshConfig) -> Result<(), ()> {
                self.0 += 1;
                Ok(())
            }
            fn publish_ready(&mut self, _: &ValidatedSshConfig) -> Result<(), ()> {
                self.0 += 1;
                Ok(())
            }
        }
        let mut source = FakeSource::valid(b"port = 0\n");
        let mut dependencies = Dependencies(0);
        let mut buffer = [0; MAX_SSH_CONFIG_BYTES];
        assert!(matches!(
            validate_then_start_ssh(
                &mut source,
                SSH_CONFIG_PATH,
                &SshRuntimeLimits::default(),
                &mut buffer,
                &mut dependencies,
            ),
            Err(SshStartupError::Configuration(_))
        ));
        assert_eq!(dependencies.0, 0);
    }
}
