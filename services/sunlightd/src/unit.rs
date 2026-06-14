//! Unit file parser for .service and .socket files
//! Implements a no-alloc-friendly parser using heapless types

use heapless::{String, Vec};

pub const MAX_UNITS: usize = 32;

pub type UnitName = String<64>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceType {
    Simple,
    Oneshot,
    Notify,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RestartPolicy {
    No,
    OnFailure,
    Always,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogDest {
    Journal,
    Null,
    Inherit,
}

#[derive(Clone, Debug)]
pub struct EnvPair {
    pub key: String<64>,
    pub value: String<128>,
}

#[derive(Clone, Debug)]
pub struct ServiceUnit {
    // [Unit]
    pub description: String<128>,
    pub after: Vec<UnitName, 8>,
    pub requires: Vec<UnitName, 8>,
    pub wants: Vec<UnitName, 8>,

    // [Service]
    pub service_type: ServiceType,
    pub exec_start: String<256>,
    pub exec_start_pre: Option<String<256>>,
    pub exec_stop: Option<String<256>>,
    pub restart: RestartPolicy,
    pub restart_sec: u32,
    pub environment: Vec<EnvPair, 16>,
    pub environment_file: Option<String<128>>,
    pub user: String<32>,
    pub working_dir: Option<String<128>>,
    pub stdout: LogDest,
    pub stderr: LogDest,

    // [Install]
    pub wanted_by: Vec<UnitName, 4>,
}

impl Default for ServiceUnit {
    fn default() -> Self {
        let mut user = String::new();
        let _ = user.push_str("root");
        
        Self {
            description: String::new(),
            after: Vec::new(),
            requires: Vec::new(),
            wants: Vec::new(),
            service_type: ServiceType::Simple,
            exec_start: String::new(),
            exec_start_pre: None,
            exec_stop: None,
            restart: RestartPolicy::No,
            restart_sec: 5,
            environment: Vec::new(),
            environment_file: None,
            user,
            working_dir: None,
            stdout: LogDest::Journal,
            stderr: LogDest::Journal,
            wanted_by: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum SocketAddr {
    Unix(String<108>),
    Tcp(u16),
}

#[derive(Clone, Debug)]
pub struct SocketUnit {
    // [Unit]
    pub description: String<128>,
    pub after: Vec<UnitName, 4>,

    // [Socket]
    pub listen_stream: SocketAddr,
    pub service: UnitName,

    // [Install]
    pub wanted_by: Vec<UnitName, 4>,
}

impl Default for SocketUnit {
    fn default() -> Self {
        Self {
            description: String::new(),
            after: Vec::new(),
            listen_stream: SocketAddr::Tcp(0),
            service: String::new(),
            wanted_by: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum ParseError {
    InvalidSection,
    InvalidKey,
    ValueTooLong,
    TooManyEntries,
}

#[derive(PartialEq, Eq)]
enum Section {
    None,
    Unit,
    Service,
    Socket,
    Install,
}

fn str_to_string<const N: usize>(s: &str) -> String<N> {
    let mut result = String::new();
    let _ = result.push_str(s);
    result
}

/// Parse a .service unit file from a byte buffer
pub fn parse_service_unit(content: &[u8]) -> Result<ServiceUnit, ParseError> {
    let mut unit = ServiceUnit::default();
    let mut section = Section::None;

    let content_str = core::str::from_utf8(content).map_err(|_| ParseError::InvalidKey)?;

    for line in content_str.lines() {
        let line = line.trim();
        
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section headers
        if line.starts_with('[') && line.ends_with(']') {
            let section_name = &line[1..line.len()-1];
            section = match section_name {
                "Unit" => Section::Unit,
                "Service" => Section::Service,
                "Install" => Section::Install,
                _ => Section::None,
            };
            continue;
        }

        // Key=Value pairs
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos+1..].trim();

            match section {
                Section::Unit => parse_unit_key(&mut unit, key, value)?,
                Section::Service => parse_service_key(&mut unit, key, value)?,
                Section::Install => parse_install_key(&mut unit, key, value)?,
                Section::None | Section::Socket => {},
            }
        }
    }

    Ok(unit)
}

fn parse_unit_key(unit: &mut ServiceUnit, key: &str, value: &str) -> Result<(), ParseError> {
    match key {
        "Description" => {
            unit.description = str_to_string(value);
        }
        "After" => {
            for dep in value.split_whitespace() {
                unit.after.push(str_to_string(dep)).map_err(|_| ParseError::TooManyEntries)?;
            }
        }
        "Requires" => {
            for dep in value.split_whitespace() {
                unit.requires.push(str_to_string(dep)).map_err(|_| ParseError::TooManyEntries)?;
            }
        }
        "Wants" => {
            for dep in value.split_whitespace() {
                unit.wants.push(str_to_string(dep)).map_err(|_| ParseError::TooManyEntries)?;
            }
        }
        _ => {} // Unknown keys silently ignored
    }
    Ok(())
}

fn parse_service_key(unit: &mut ServiceUnit, key: &str, value: &str) -> Result<(), ParseError> {
    match key {
        "Type" => {
            unit.service_type = match value {
                "simple" => ServiceType::Simple,
                "oneshot" => ServiceType::Oneshot,
                "notify" => ServiceType::Notify,
                _ => ServiceType::Simple,
            };
        }
        "ExecStart" => {
            unit.exec_start = str_to_string(value);
        }
        "ExecStartPre" => {
            unit.exec_start_pre = Some(str_to_string(value));
        }
        "ExecStop" => {
            unit.exec_stop = Some(str_to_string(value));
        }
        "Restart" => {
            unit.restart = match value {
                "always" => RestartPolicy::Always,
                "on-failure" => RestartPolicy::OnFailure,
                _ => RestartPolicy::No,
            };
        }
        "RestartSec" => {
            if let Ok(sec) = value.parse::<u32>() {
                unit.restart_sec = sec;
            }
        }
        "Environment" => {
            if let Some(eq_pos) = value.find('=') {
                let env_key = &value[..eq_pos];
                let env_val = &value[eq_pos+1..];
                unit.environment.push(EnvPair {
                    key: str_to_string(env_key),
                    value: str_to_string(env_val),
                }).map_err(|_| ParseError::TooManyEntries)?;
            }
        }
        "EnvironmentFile" => {
            unit.environment_file = Some(str_to_string(value));
        }
        "User" => {
            unit.user = str_to_string(value);
        }
        "WorkingDirectory" => {
            unit.working_dir = Some(str_to_string(value));
        }
        "StandardOutput" => {
            unit.stdout = match value {
                "journal" => LogDest::Journal,
                "null" => LogDest::Null,
                "inherit" => LogDest::Inherit,
                _ => LogDest::Journal,
            };
        }
        "StandardError" => {
            unit.stderr = match value {
                "journal" => LogDest::Journal,
                "null" => LogDest::Null,
                "inherit" => LogDest::Inherit,
                _ => LogDest::Journal,
            };
        }
        _ => {}
    }
    Ok(())
}

fn parse_install_key(unit: &mut ServiceUnit, key: &str, value: &str) -> Result<(), ParseError> {
    match key {
        "WantedBy" => {
            for target in value.split_whitespace() {
                unit.wanted_by.push(str_to_string(target)).map_err(|_| ParseError::TooManyEntries)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Parse a .socket unit file from a byte buffer
pub fn parse_socket_unit(content: &[u8]) -> Result<SocketUnit, ParseError> {
    let mut unit = SocketUnit::default();
    let mut section = Section::None;

    let content_str = core::str::from_utf8(content).map_err(|_| ParseError::InvalidKey)?;

    for line in content_str.lines() {
        let line = line.trim();
        
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            let section_name = &line[1..line.len()-1];
            section = match section_name {
                "Unit" => Section::Unit,
                "Socket" => Section::Socket,
                "Install" => Section::Install,
                _ => Section::None,
            };
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos+1..].trim();

            match section {
                Section::Unit => parse_socket_unit_key(&mut unit, key, value)?,
                Section::Socket => parse_socket_key(&mut unit, key, value)?,
                Section::Install => parse_socket_install_key(&mut unit, key, value)?,
                Section::None | Section::Service => {},
            }
        }
    }

    Ok(unit)
}

fn parse_socket_unit_key(unit: &mut SocketUnit, key: &str, value: &str) -> Result<(), ParseError> {
    match key {
        "Description" => {
            unit.description = str_to_string(value);
        }
        "After" => {
            for dep in value.split_whitespace() {
                unit.after.push(str_to_string(dep)).map_err(|_| ParseError::TooManyEntries)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_socket_key(unit: &mut SocketUnit, key: &str, value: &str) -> Result<(), ParseError> {
    match key {
        "ListenStream" => {
            // Try to parse as port number first
            if let Ok(port) = value.parse::<u16>() {
                unit.listen_stream = SocketAddr::Tcp(port);
            } else {
                // Otherwise treat as Unix socket path
                unit.listen_stream = SocketAddr::Unix(str_to_string(value));
            }
        }
        "Service" => {
            unit.service = str_to_string(value);
        }
        _ => {}
    }
    Ok(())
}

fn parse_socket_install_key(unit: &mut SocketUnit, key: &str, value: &str) -> Result<(), ParseError> {
    match key {
        "WantedBy" => {
            for target in value.split_whitespace() {
                unit.wanted_by.push(str_to_string(target)).map_err(|_| ParseError::TooManyEntries)?;
            }
        }
        _ => {}
    }
    Ok(())
}
