//! Shared app launcher used by `sun-exec` and GUI launch entry points.
//!
//! This intentionally stays policy-light: it resolves an app id or command,
//! emits timing trace points, then calls the existing spawn syscall wrapper.
//! TODO: nice_d profile lookup.
//! TODO: VIP launch flag.
//! TODO: scheduler bypass lanes.

use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis,
};

use crate::{self as libc, Errno, MAX_ARGS, MAX_PATH};

const MAX_ARG_LEN: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchError {
    AppNotFound,
    InvalidCommand,
    SpawnFailed(Errno),
    PermissionDenied,
    DisplayUnavailable,
    TooManyArgs,
    ArgTooLong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchResult {
    pub pid: u64,
    pub path_len: usize,
    pub path: [u8; MAX_PATH],
}

impl LaunchResult {
    pub fn path(&self) -> &[u8] {
        &self.path[..self.path_len]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchRequest<'a> {
    pub trace: LaunchTrace,
    pub source: LaunchSource,
    pub command: &'a [u8],
    pub args: &'a [&'a [u8]],
    pub require_display: bool,
}

impl<'a> LaunchRequest<'a> {
    pub fn new(trace: LaunchTrace, source: LaunchSource, command: &'a [u8]) -> Self {
        Self {
            trace,
            source,
            command,
            args: &[],
            require_display: true,
        }
    }
}

pub fn launch(request: LaunchRequest<'_>) -> Result<LaunchResult, LaunchError> {
    let subject_buf = Subject::new(request.command);
    let subject = subject_buf.as_str();
    launch_trace::log_phase_now(request.trace, subject, "request_received", None);

    validate_command(request.command)?;
    if request.args.len() + 2 > MAX_ARGS {
        log_error(request.trace, subject, "too_many_args");
        return Err(LaunchError::TooManyArgs);
    }

    if request.require_display && sunlight_ipc::nameserver_lookup("display_server").is_none() {
        log_error(request.trace, subject, "display_unavailable");
        return Err(LaunchError::DisplayUnavailable);
    }

    let resolved = match resolve(request.command) {
        Some(resolved) => resolved,
        None => {
            log_error(request.trace, subject, "app_not_found");
            return Err(LaunchError::AppNotFound);
        }
    };
    launch_trace::log_phase_now(request.trace, resolved.subject(), "app_resolved", None);

    if !is_executable(resolved.path()) {
        log_error(request.trace, resolved.subject(), "permission_denied");
        return Err(LaunchError::PermissionDenied);
    }

    let mut trace_arg = [0u8; 64];
    let trace_arg_len = launch_trace::format_launch_arg(request.trace, &mut trace_arg).unwrap_or(0);
    let mut argv: [&[u8]; MAX_ARGS] = [&[]; MAX_ARGS];
    let mut argc = 0usize;
    argv[argc] = resolved.argv0();
    argc += 1;
    for arg in request.args {
        if arg.len() > MAX_ARG_LEN || arg.contains(&0) {
            log_error(request.trace, resolved.subject(), "invalid_command");
            return Err(LaunchError::ArgTooLong);
        }
        argv[argc] = arg;
        argc += 1;
    }
    if trace_arg_len != 0 && argc < MAX_ARGS {
        argv[argc] = &trace_arg[..trace_arg_len];
        argc += 1;
    }

    launch_trace::log_phase_now(request.trace, resolved.subject(), "spawn_entered", None);
    match libc::spawn(resolved.path(), &argv[..argc], None) {
        Ok(pid) => {
            launch_trace::log_phase_now(
                request.trace,
                resolved.subject(),
                "spawn_returned",
                Some(pid),
            );
            launch_trace::log_phase_now(
                request.trace,
                resolved.subject(),
                "process_created",
                Some(pid),
            );
            Ok(LaunchResult {
                pid,
                path_len: resolved.path_len,
                path: resolved.path,
            })
        }
        Err(err) => {
            log_error(request.trace, resolved.subject(), "spawn_failed");
            Err(LaunchError::SpawnFailed(err))
        }
    }
}

pub fn launch_from_words(
    trace: LaunchTrace,
    source: LaunchSource,
    words: &[&[u8]],
    require_display: bool,
) -> Result<LaunchResult, LaunchError> {
    if words.is_empty() || words[0].is_empty() {
        return Err(LaunchError::InvalidCommand);
    }
    launch(LaunchRequest {
        trace,
        source,
        command: words[0],
        args: &words[1..],
        require_display,
    })
}

pub fn split_words<'a>(input: &'a [u8], out: &mut [&'a [u8]]) -> Result<usize, LaunchError> {
    let mut count = 0usize;
    let mut pos = 0usize;
    while pos < input.len() {
        while pos < input.len() && input[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= input.len() {
            break;
        }
        if count >= out.len() {
            return Err(LaunchError::TooManyArgs);
        }
        let start = pos;
        while pos < input.len() && !input[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos - start > MAX_ARG_LEN {
            return Err(LaunchError::ArgTooLong);
        }
        out[count] = &input[start..pos];
        count += 1;
    }
    Ok(count)
}

fn resolve(command: &[u8]) -> Option<ResolvedApp> {
    if let Some(path) = map_app_id(command) {
        return ResolvedApp::from_path(path);
    }
    if command.first() == Some(&b'/') {
        return ResolvedApp::from_path(command);
    }

    let mut path = [0u8; MAX_PATH];
    for prefix in [b"/bin/".as_slice(), b"/usr/bin/".as_slice()] {
        let len = prefix.len().checked_add(command.len())?;
        if len > path.len() {
            return None;
        }
        path[..prefix.len()].copy_from_slice(prefix);
        path[prefix.len()..len].copy_from_slice(command);
        if libc::stat(&path[..len]).is_ok() {
            return ResolvedApp::from_path(&path[..len]);
        }
    }
    None
}

fn map_app_id(command: &[u8]) -> Option<&'static [u8]> {
    match command {
        b"calculator" | b"calc" => Some(b"/bin/calculator"),
        b"terminal" | b"term" | b"sunlight-terminal" => Some(b"/bin/sunlight-terminal"),
        b"settings" | b"control-panel" | b"preferences" => Some(b"/bin/control-panel"),
        b"files" | b"file-manager" | b"sunlight-files" => Some(b"/bin/sunlight-files"),
        b"tasks" | b"task-manager" | b"sunlight-tasks" => Some(b"/bin/sunlight-tasks"),
        b"eyes" => Some(b"/bin/eyes"),
        b"bench" | b"sunbench" | b"sunlight-bench" => Some(b"/bin/sunbench"),
        b"sunlight-edit" | b"sunlight-text" | b"edit" | b"text-editor" => {
            Some(b"/bin/sunlight-edit")
        }
        b"light-lens" | b"photos" | b"photo-viewer" => Some(b"/bin/light-lens"),
        b"calendar" | b"sunlight-calendar" => Some(b"/bin/sunlight-calendar"),
        b"rappid-rabbit" | b"rabbit" => Some(b"/bin/rappid-rabbit"),
        b"sun-open" => Some(b"/bin/sun-open"),
        _ => None,
    }
}

fn validate_command(command: &[u8]) -> Result<(), LaunchError> {
    if command.is_empty() || command.len() >= MAX_PATH || command.contains(&0) {
        return Err(LaunchError::InvalidCommand);
    }
    if command.iter().any(|b| b.is_ascii_control()) {
        return Err(LaunchError::InvalidCommand);
    }
    Ok(())
}

fn is_executable(path: &[u8]) -> bool {
    let Ok(stat) = libc::stat(path) else {
        return false;
    };
    stat.mode & 0o111 != 0
}

fn log_error(trace: LaunchTrace, subject: &str, error: &str) {
    launch_trace::log_phase_now(trace, subject, error, None);
    debug_log("sun-exec error=");
    debug_log(error);
    debug_log("\n");
}

struct ResolvedApp {
    path: [u8; MAX_PATH],
    path_len: usize,
}

impl ResolvedApp {
    fn from_path(path: &[u8]) -> Option<Self> {
        if path.is_empty() || path.len() > MAX_PATH {
            return None;
        }
        let mut out = [0u8; MAX_PATH];
        out[..path.len()].copy_from_slice(path);
        if libc::stat(path).is_err() {
            return None;
        }
        Some(Self {
            path: out,
            path_len: path.len(),
        })
    }

    fn path(&self) -> &[u8] {
        &self.path[..self.path_len]
    }

    fn argv0(&self) -> &[u8] {
        self.path()
    }

    fn subject(&self) -> &str {
        core::str::from_utf8(self.path()).unwrap_or("app")
    }
}

struct Subject {
    buf: [u8; 96],
    len: usize,
}

impl Subject {
    fn new(command: &[u8]) -> Self {
        let mut out = Self {
            buf: [0; 96],
            len: 0,
        };
        let prefix = b"app=";
        out.buf[..prefix.len()].copy_from_slice(prefix);
        out.len = prefix.len();
        let take = command.len().min(out.buf.len().saturating_sub(out.len));
        out.buf[out.len..out.len + take].copy_from_slice(&command[..take]);
        out.len += take;
        out
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("app")
    }
}

pub fn next_cli_trace(source: LaunchSource) -> LaunchTrace {
    LaunchTrace::new(monotonic_millis(), source, monotonic_millis())
}
