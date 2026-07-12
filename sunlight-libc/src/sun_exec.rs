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
const MAX_MANIFEST_BYTES: usize = 8192;
const MAX_BUNDLE_PATH: usize = MAX_PATH - 14;
const CHRONOS_RUNTIME_PATH: &[u8] = b"/bin/sunlight-chronos";

/// Runtime selected by a validated external application bundle.  Native
/// applications keep using the established executable resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeKind {
    Native,
    Chronos,
    Helios,
}

/// Validated external launch metadata.  It deliberately contains only
/// guest-facing metadata and bounded byte slices; capabilities stay owned by
/// the native process that opens the scoped roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationLaunchRequest<'a> {
    pub app_id: &'a [u8],
    pub display_name: &'a [u8],
    pub runtime: RuntimeKind,
    pub entry: &'a [u8],
    pub bundle_root: &'a [u8],
    pub documents_read_write: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchError {
    AppNotFound,
    InvalidCommand,
    SpawnFailed(Errno),
    PermissionDenied,
    DisplayUnavailable,
    TooManyArgs,
    ArgTooLong,
    InvalidBundle,
    UnsupportedBundleFormat,
    UnsupportedRuntime,
    MissingEntry,
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

    if is_bundle_path(request.command) {
        return launch_chronos_bundle(request);
    }
    if is_com_path(request.command) {
        return launch_direct_com(request);
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

fn launch_chronos_bundle(request: LaunchRequest<'_>) -> Result<LaunchResult, LaunchError> {
    let bundle = normalize_bundle_path(request.command)?;
    let bundle = &bundle[..request.command.len()];
    let manifest = read_bundle_manifest(bundle)?;
    let descriptor = parse_chronos_manifest(bundle, &manifest[..])?;
    if request.require_display && sunlight_ipc::nameserver_lookup("display_server").is_none() {
        return Err(LaunchError::DisplayUnavailable);
    }

    let mut entry = [0u8; MAX_PATH];
    let entry_len = join_bundle_program(&mut entry, bundle, descriptor.entry)?;
    let mut document_root = [0u8; MAX_PATH];
    let document_len = user_documents_path(&mut document_root)?;
    let mut args: [&[u8]; MAX_ARGS - 1] = [&[]; MAX_ARGS - 1];
    args[0] = b"--chronos-bundle";
    args[1] = &bundle[..];
    args[2] = b"--chronos-entry";
    args[3] = &entry[..entry_len];
    args[4] = b"--chronos-app-id";
    args[5] = descriptor.app_id;
    args[6] = b"--chronos-title";
    args[7] = descriptor.display_name;
    // The document scope policy is sent as one extra bounded argument when
    // writable.  Chronos defaults to no D: access if it is absent.
    let document_args: [&[u8]; 2] = if descriptor.documents_read_write {
        [b"--chronos-documents", &document_root[..document_len]]
    } else {
        [b"", b""]
    };
    launch_runtime(
        request.trace,
        &args,
        if descriptor.documents_read_write {
            &document_args
        } else {
            &[]
        },
    )
}

fn launch_direct_com(request: LaunchRequest<'_>) -> Result<LaunchResult, LaunchError> {
    let program = normalize_existing_path(request.command)?;
    let program = &program[..request.command.len()];
    if request.require_display && sunlight_ipc::nameserver_lookup("display_server").is_none() {
        return Err(LaunchError::DisplayUnavailable);
    }
    let title = b"Chronos - Sunlight DOS Terminal";
    let app_id = filename(program).unwrap_or(b"dos-program");
    let mut args: [&[u8]; 8] = [&[]; 8];
    args[0] = b"--chronos-direct";
    args[1] = program;
    args[2] = b"--chronos-app-id";
    args[3] = app_id;
    args[4] = b"--chronos-title";
    args[5] = title;
    let mut argc = 6usize;
    for argument in request.args {
        if argument.len() > MAX_ARG_LEN || argument.contains(&0) || argc >= args.len() {
            return Err(LaunchError::ArgTooLong);
        }
        args[argc] = argument;
        argc += 1;
    }
    launch_runtime(request.trace, &args[..argc], &[])
}

fn launch_runtime(
    trace: LaunchTrace,
    chronos_args: &[&[u8]],
    extra_args: &[&[u8]],
) -> Result<LaunchResult, LaunchError> {
    let mut trace_arg = [0u8; 64];
    let trace_arg_len = launch_trace::format_launch_arg(trace, &mut trace_arg).unwrap_or(0);
    if chronos_args.len() + extra_args.len() + 2 > MAX_ARGS {
        return Err(LaunchError::TooManyArgs);
    }
    let mut argv: [&[u8]; MAX_ARGS] = [&[]; MAX_ARGS];
    let mut argc = 0;
    argv[argc] = CHRONOS_RUNTIME_PATH;
    argc += 1;
    for value in chronos_args.iter().chain(extra_args.iter()) {
        argv[argc] = value;
        argc += 1;
    }
    if trace_arg_len != 0 && argc < MAX_ARGS {
        argv[argc] = &trace_arg[..trace_arg_len];
        argc += 1;
    }
    let pid =
        libc::spawn(CHRONOS_RUNTIME_PATH, &argv[..argc], None).map_err(LaunchError::SpawnFailed)?;
    let mut path = [0; MAX_PATH];
    path[..CHRONOS_RUNTIME_PATH.len()].copy_from_slice(CHRONOS_RUNTIME_PATH);
    Ok(LaunchResult {
        pid,
        path_len: CHRONOS_RUNTIME_PATH.len(),
        path,
    })
}

#[derive(Clone, Copy)]
struct ChronosManifest<'a> {
    app_id: &'a [u8],
    display_name: &'a [u8],
    entry: &'a [u8],
    documents_read_write: bool,
}

fn parse_chronos_manifest<'a>(
    bundle_root: &[u8],
    source: &'a [u8],
) -> Result<ChronosManifest<'a>, LaunchError> {
    let format = toml_value(source, b"bundle", b"format").ok_or(LaunchError::InvalidBundle)?;
    if format != b"1" {
        return Err(LaunchError::UnsupportedBundleFormat);
    }
    let runtime = toml_value(source, b"runtime", b"type").ok_or(LaunchError::InvalidBundle)?;
    if runtime != b"chronos" {
        return Err(LaunchError::UnsupportedRuntime);
    }
    let app_id = toml_value(source, b"application", b"id").ok_or(LaunchError::InvalidBundle)?;
    let display_name =
        toml_value(source, b"application", b"name").ok_or(LaunchError::InvalidBundle)?;
    let entry = toml_value(source, b"entry", b"executable").ok_or(LaunchError::MissingEntry)?;
    let icon = toml_value(source, b"application", b"icon").ok_or(LaunchError::InvalidBundle)?;
    if !valid_app_id(app_id)
        || display_name.is_empty()
        || !valid_bundle_relative(icon)
        || !valid_dos_entry(entry)
    {
        return Err(LaunchError::InvalidBundle);
    }
    let mut entry_path = [0u8; MAX_PATH];
    let entry_len = join_bundle_program(&mut entry_path, bundle_root, entry)?;
    if libc::stat(&entry_path[..entry_len]).is_err() {
        return Err(LaunchError::MissingEntry);
    }
    let mut icon_path = [0u8; MAX_PATH];
    let icon_len = join_bundle_file(&mut icon_path, bundle_root, icon)?;
    if libc::stat(&icon_path[..icon_len]).is_err() {
        return Err(LaunchError::InvalidBundle);
    }
    let document_permission = toml_value(source, b"permissions", b"documents").unwrap_or(b"none");
    let documents_read_write = match document_permission {
        b"none" | b"read-only" => false,
        b"read-write" => true,
        _ => return Err(LaunchError::InvalidBundle),
    };
    Ok(ChronosManifest {
        app_id,
        display_name,
        entry,
        documents_read_write,
    })
}

fn toml_value<'a>(source: &'a [u8], wanted_section: &[u8], wanted_key: &[u8]) -> Option<&'a [u8]> {
    let mut section = &[][..];
    for raw_line in source.split(|byte| *byte == b'\n') {
        let line = trim(raw_line);
        if line.starts_with(b"[") && line.ends_with(b"]") {
            section = &line[1..line.len() - 1];
            continue;
        }
        if section != wanted_section || line.starts_with(b"#") {
            continue;
        }
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (key, value) = (&line[..separator], &line[separator + 1..]);
        if trim(key) != wanted_key {
            continue;
        }
        let value = trim(value);
        if value.len() >= 2 && value[0] == b'"' && value[value.len() - 1] == b'"' {
            return Some(&value[1..value.len() - 1]);
        }
        if value.iter().all(|byte| byte.is_ascii_digit()) {
            return Some(value);
        }
        return None;
    }
    None
}

fn read_bundle_manifest(bundle: &[u8]) -> Result<[u8; MAX_MANIFEST_BYTES], LaunchError> {
    let mut path = [0u8; MAX_PATH];
    let len = join_path(&mut path, bundle, b"Manifest.toml").ok_or(LaunchError::InvalidBundle)?;
    let fd = libc::open(&path[..len]).map_err(|_| LaunchError::InvalidBundle)?;
    let mut out = [0u8; MAX_MANIFEST_BYTES];
    let count = libc::read(fd, &mut out).map_err(|_| LaunchError::InvalidBundle)?;
    let _ = libc::close(fd);
    if count == 0 || count == out.len() {
        return Err(LaunchError::InvalidBundle);
    }
    out[count] = 0;
    Ok(out)
}

fn normalize_bundle_path(command: &[u8]) -> Result<[u8; MAX_PATH], LaunchError> {
    let path = normalize_existing_path(command)?;
    if !is_bundle_path(&path) {
        return Err(LaunchError::InvalidBundle);
    }
    Ok(path)
}

fn normalize_existing_path(command: &[u8]) -> Result<[u8; MAX_PATH], LaunchError> {
    let mut path = [0u8; MAX_PATH];
    let value = if command.first() == Some(&b'/') {
        command
    } else {
        return Err(LaunchError::InvalidCommand);
    };
    if value.len() >= MAX_PATH || value.contains(&0) || libc::stat(value).is_err() {
        return Err(LaunchError::AppNotFound);
    }
    path[..value.len()].copy_from_slice(value);
    Ok(path)
}

fn is_bundle_path(path: &[u8]) -> bool {
    path.ends_with(b".sunapp")
}

fn is_com_path(path: &[u8]) -> bool {
    path.len() >= 4 && path[path.len() - 4..].eq_ignore_ascii_case(b".com")
}

fn valid_bundle_relative(value: &[u8]) -> bool {
    !value.is_empty()
        && !value.starts_with(b"/")
        && !value.contains(&b'\\')
        && !value
            .split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b"..")
}

fn valid_dos_entry(value: &[u8]) -> bool {
    value.len() > 3
        && value[1] == b':'
        && value[0].eq_ignore_ascii_case(&b'C')
        && value[2..].starts_with(b"\\")
        && !value.contains(&b'/')
        && !value
            .split(|byte| *byte == b'\\')
            .any(|part| part == b".." || part == b".")
}

fn valid_app_id(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn join_bundle_program(
    out: &mut [u8; MAX_PATH],
    bundle: &[u8],
    dos_entry: &[u8],
) -> Result<usize, LaunchError> {
    let mut program = [0u8; MAX_PATH];
    let program_len =
        join_path(&mut program, bundle, b"Program").ok_or(LaunchError::InvalidBundle)?;
    join_bundle_file(out, &program[..program_len], &dos_entry[3..])
}

fn join_bundle_file(
    out: &mut [u8; MAX_PATH],
    bundle: &[u8],
    relative: &[u8],
) -> Result<usize, LaunchError> {
    let mut transformed = [0u8; MAX_BUNDLE_PATH];
    let mut len = 0usize;
    for byte in relative {
        if *byte == b'\\' {
            transformed[len] = b'/';
        } else {
            transformed[len] = *byte;
        }
        len += 1;
    }
    join_path(out, bundle, &transformed[..len]).ok_or(LaunchError::InvalidBundle)
}

fn join_path(out: &mut [u8; MAX_PATH], left: &[u8], right: &[u8]) -> Option<usize> {
    let separator = (!left.ends_with(b"/")) as usize;
    let total = left
        .len()
        .checked_add(separator)?
        .checked_add(right.len())?;
    if total >= out.len() {
        return None;
    }
    out[..left.len()].copy_from_slice(left);
    if separator != 0 {
        out[left.len()] = b'/';
    }
    out[left.len() + separator..total].copy_from_slice(right);
    Some(total)
}

fn user_documents_path(out: &mut [u8; MAX_PATH]) -> Result<usize, LaunchError> {
    let home = crate::env::getenv_bytes(b"HOME").unwrap_or(b"/root");
    join_path(out, home, b"Documents").ok_or(LaunchError::InvalidBundle)
}

fn trim(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'\r')
    {
        value = &value[..value.len() - 1];
    }
    value
}

fn filename(path: &[u8]) -> Option<&[u8]> {
    path.rsplit(|byte| *byte == b'/')
        .next()
        .filter(|part| !part.is_empty())
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
        b"chronos" | b"sunlight-chronos" => Some(b"/bin/sunlight-chronos"),
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
        b"sunlight-api-lab" | b"api-lab" => Some(b"/bin/sunlight-api-lab"),
        b"emoji-picker" | b"emoji" | b"picker" => Some(b"/bin/emoji-picker"),
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

#[cfg(test)]
mod tests {
    use super::map_app_id;

    #[test]
    fn chronos_aliases_resolve_to_the_native_application() {
        assert_eq!(
            map_app_id(b"chronos"),
            Some(b"/bin/sunlight-chronos".as_slice())
        );
        assert_eq!(
            map_app_id(b"sunlight-chronos"),
            Some(b"/bin/sunlight-chronos".as_slice())
        );
    }
}
