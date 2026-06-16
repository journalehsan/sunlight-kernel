//! CLI implementation for sunlight-kvctl.
//! Subcommands: put, get, delete, scan.
//!
//! Connects to the daemon over the same Unix socket used by the server
//! (SUNLIGHT_KV_SOCKET or /tmp/sunlight/kv.sock).

use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use sunlight_kv::ipc::{Request, Response};

/// Parsed command line action.
#[derive(Debug, Clone)]
pub enum Command {
    Put { key: String, value: String },
    Get { key: String },
    Delete { key: String },
    Scan { prefix: String },
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("invalid arguments: {0}")]
    Usage(String),

    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("serialization: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("daemon error: {0}")]
    Daemon(String),

    #[error("not found")]
    NotFound,

    #[error("permission denied")]
    PermissionDenied,
}

pub struct Client {
    socket_path: PathBuf,
}

impl Client {
    pub fn new() -> Self {
        let sock = env::var("SUNLIGHT_KV_SOCKET")
            .unwrap_or_else(|_| "/tmp/sunlight/kv.sock".to_string());
        Self {
            socket_path: sock.into(),
        }
    }

    #[allow(dead_code)]
    pub fn with_socket<P: Into<PathBuf>>(p: P) -> Self {
        Self { socket_path: p.into() }
    }

    fn connect(&self) -> io::Result<UnixStream> {
        UnixStream::connect(&self.socket_path)
    }

    /// Send one request and receive the matching response.
    pub fn call(&self, req: Request) -> Result<Response, CliError> {
        let mut stream = self.connect()?;
        send_frame(&mut stream, &req)?;
        let resp = recv_frame(&mut stream)?;
        Ok(resp)
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

/// Execute a high-level command against the daemon and return human-friendly result.
pub fn execute(cmd: Command) -> Result<String, CliError> {
    let client = Client::new();

    match cmd {
        Command::Put { key, value } => {
            let req = Request::KV_PUT {
                key,
                value: value.into_bytes(),
            };
            match client.call(req)? {
                Response::OK => Ok("OK".to_string()),
                Response::PERMISSION_DENIED => Err(CliError::PermissionDenied),
                Response::ERROR(e) => Err(CliError::Daemon(e)),
                other => Err(CliError::Daemon(format!("unexpected response: {:?}", other))),
            }
        }
        Command::Get { key } => {
            let req = Request::KV_GET { key: key.clone() };
            match client.call(req)? {
                Response::VALUE(v) => {
                    // Print raw bytes as lossy UTF-8 for human consumption.
                    let s = String::from_utf8_lossy(&v);
                    Ok(s.into_owned())
                }
                Response::NOT_FOUND => Err(CliError::NotFound),
                Response::PERMISSION_DENIED => Err(CliError::PermissionDenied),
                Response::ERROR(e) => Err(CliError::Daemon(e)),
                other => Err(CliError::Daemon(format!("unexpected response: {:?}", other))),
            }
        }
        Command::Delete { key } => {
            let req = Request::KV_DELETE { key };
            match client.call(req)? {
                Response::OK => Ok("OK".to_string()),
                Response::NOT_FOUND => Err(CliError::NotFound),
                Response::PERMISSION_DENIED => Err(CliError::PermissionDenied),
                Response::ERROR(e) => Err(CliError::Daemon(e)),
                other => Err(CliError::Daemon(format!("unexpected: {:?}", other))),
            }
        }
        Command::Scan { prefix } => {
            let req = Request::KV_SCAN { prefix };
            match client.call(req)? {
                Response::SCAN_RESULT(keys) => {
                    if keys.is_empty() {
                        Ok("(no keys)".to_string())
                    } else {
                        Ok(keys.join("\n"))
                    }
                }
                Response::ERROR(e) => Err(CliError::Daemon(e)),
                other => Err(CliError::Daemon(format!("unexpected: {:?}", other))),
            }
        }
    }
}

// -------------------------------------------------------------------------
// Small framing duplicated from daemon for independence (no internal re-export coupling).
// -------------------------------------------------------------------------

fn send_frame<T: serde::Serialize>(stream: &mut UnixStream, val: &T) -> io::Result<()> {
    let bytes = bincode::serialize(val)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

fn recv_frame<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> io::Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    if len > 16 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let v: T = bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(v)
}

// -------------------------------------------------------------------------
// Argument parsing (very small, no external deps).
// -------------------------------------------------------------------------

pub fn parse_args(args: &[String]) -> Result<Command, CliError> {
    if args.is_empty() {
        return Err(CliError::Usage(help_text()));
    }

    match args[0].as_str() {
        "put" | "p" => {
            if args.len() != 3 {
                return Err(CliError::Usage(
                    "put requires exactly two arguments: KEY VALUE".into(),
                ));
            }
            Ok(Command::Put {
                key: args[1].clone(),
                value: args[2].clone(),
            })
        }
        "get" | "g" => {
            if args.len() != 2 {
                return Err(CliError::Usage("get requires exactly one argument: KEY".into()));
            }
            Ok(Command::Get { key: args[1].clone() })
        }
        "delete" | "del" | "d" | "rm" => {
            if args.len() != 2 {
                return Err(CliError::Usage(
                    "delete requires exactly one argument: KEY".into(),
                ));
            }
            Ok(Command::Delete { key: args[1].clone() })
        }
        "scan" | "s" | "ls" => {
            let prefix = if args.len() >= 2 {
                args[1].clone()
            } else {
                String::new()
            };
            Ok(Command::Scan { prefix })
        }
        "help" | "-h" | "--help" => Err(CliError::Usage(help_text())),
        other => Err(CliError::Usage(format!("unknown command: {}", other))),
    }
}

fn help_text() -> String {
    "\
sunlight-kvctl - client for sunlight-kv daemon

Usage:
  sunlight-kvctl put KEY VALUE
  sunlight-kvctl get KEY
  sunlight-kvctl delete KEY
  sunlight-kvctl scan [PREFIX]

Environment:
  SUNLIGHT_KV_SOCKET   Path to the daemon Unix socket (default: /tmp/sunlight/kv.sock)
".to_string()
}
