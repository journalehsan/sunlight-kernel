// Descriptor-based noninteractive platform adapter. No terminal registration.
use super::*;
use alloc::{format, string::String, vec::Vec};
use sunlight_libc as libc;
use sunshell::{
    parser::{AstNode, RedirectMode},
    script::{Error, Flow, Runtime, ShellExecutionContext},
    script_parser::MAX_SOURCE,
    shellenv::ShellEnv,
};

pub(super) struct NativeRuntime {
    shell: Shell,
    captures: sunshell::capture::Captures,
    owned_scope: u64,
}
impl NativeRuntime {
    pub(super) fn new() -> Self {
        Self {
            owned_scope: 0,
            shell: Shell::new(),
            captures: sunshell::capture::Captures::default(),
        }
    }
}
impl Drop for NativeRuntime {
    fn drop(&mut self) {
        if self.owned_scope != 0 {
            let _ = sunlight_ipc::foreground(2, self.owned_scope);
        }
    }
}
fn error(e: libc::Errno) -> Error {
    Error::new(1, format!("native I/O: {e:?}"))
}
fn path(name: &str, context: &ShellExecutionContext) -> Result<String, Error> {
    normalize_path(&context.cwd, name).ok_or_else(|| Error::new(1, "invalid path"))
}
fn checkpoint() -> Result<(), Error> {
    let signal = sunlight_ipc::foreground(1, 0);
    if signal == 0 {
        return Ok(());
    }
    let mut e = Error::new(
        if signal == 126 {
            2
        } else {
            128 + signal as i32
        },
        "foreground execution cancelled",
    );
    e.expansion_fatal = true;
    Err(e)
}
impl Runtime for NativeRuntime {
    fn begin_execution(&mut self) -> Result<(), Error> {
        if self.owned_scope == 0 {
            let scope = sunlight_ipc::foreground(0, 0);
            if scope != u64::MAX {
                self.owned_scope = scope;
            }
        }
        checkpoint()
    }
    fn end_execution(&mut self) {
        if self.owned_scope != 0 {
            let _ = sunlight_ipc::foreground(2, self.owned_scope);
            self.owned_scope = 0;
        }
    }
    fn checkpoint(&mut self) -> Result<(), Error> {
        checkpoint()
    }
    fn begin_capture(&mut self) -> Result<i32, Error> {
        self.begin_execution()?;
        self.captures.begin()
    }
    fn end_capture(&mut self, fd: i32) -> Result<Vec<u8>, Error> {
        self.captures.end(fd)
    }
    fn external(
        &mut self,
        command: &AstNode<String>,
        context: &ShellExecutionContext,
        stage_environments: Option<&[ShellEnv]>,
    ) -> Result<i32, Error> {
        self.begin_execution()?;
        if stage_environments.is_none() {
            context.env.export_size().map_err(|e| Error::new(2, e))?;
        }
        let mut mapped = context.io;
        let mut channels = Vec::new();
        let mut writers = Vec::new();
        channels
            .try_reserve(3)
            .map_err(|_| Error::fatal("out of memory preparing capture pipes"))?;
        writers
            .try_reserve(3)
            .map_err(|_| Error::fatal("out of memory preparing capture pipes"))?;
        for fd in &mut mapped {
            if *fd >= 0 {
                continue;
            }
            if let Some((_, _, writer)) = channels.iter().find(|(sink, _, _)| sink == fd) {
                *fd = *writer;
                continue;
            }
            let (reader, writer) = libc::pipe().map_err(error)?;
            channels.push((*fd, OwnedFd(reader), writer.0 as i32));
            writers.push(OwnedFd(writer));
            *fd = writer.0 as i32;
        }
        self.shell.cwd = context.cwd.clone();
        self.shell.env = context.env.clone();
        self.shell.script_io = Some(mapped);
        let mut old = [0u8; libc::MAX_PATH];
        let len = libc::getcwd(&mut old)
            .map_err(|e| Error::new(1, format!("getcwd before spawn: {e:?}")))?;
        libc::chdir(context.cwd.as_bytes())
            .map_err(|e| Error::new(1, format!("chdir before spawn: {e:?}")))?;
        let commands = match command {
            AstNode::Command(command) => core::slice::from_ref(command),
            AstNode::Pipeline(commands) => commands.as_slice(),
        };
        let result = self
            .shell
            .launch_pipeline_environments(commands, stage_environments);
        drop(writers);
        let mut capture_error = None;
        let status = if let Err(message) = result {
            let status = self
                .shell
                .env
                .get("?")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            // Restore cwd even if diagnostic delivery fails.
            let _ = self.write(context.io[2], message.as_bytes());
            status
        } else {
            self.shell.fg_pid = None;
            let pids = core::mem::take(&mut self.shell.fg_pids);
            if let Err(e) = drain(&mut self.captures, channels) {
                capture_error = Some(e);
                // Failure authority is implicit kernel-installed membership;
                // no PID or caller-selected foreign scope crosses the ABI.
                let _ = sunlight_ipc::foreground(8, 0);
            }
            let mut status = 0;
            for pid in pids {
                status = libc::waitpid(pid).map(|s| s as i32).unwrap_or(1);
            }
            status
        };
        libc::chdir(&old[..len]).map_err(error)?;
        if let Some(mut e) = capture_error {
            e.expansion_fatal = true;
            return Err(e);
        }
        checkpoint()?;
        Ok(status)
    }
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<(), Error> {
        if fd < 0 {
            return self.captures.append(fd, bytes);
        }
        libc::write_all(libc::Fd(fd as u32), bytes)
            .map_err(|e| Error::new(1, format!("write fd {fd}: {e:?}")))
    }
    fn open(
        &mut self,
        name: &str,
        mode: RedirectMode,
        context: &ShellExecutionContext,
    ) -> Result<i32, Error> {
        let flags = match mode {
            RedirectMode::Read => libc::O_RDONLY,
            RedirectMode::Truncate => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            RedirectMode::Append => libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
        };
        libc::open_with_flags_mode(path(name, context)?.as_bytes(), flags, 0o644)
            .map(|fd| fd.0 as i32)
            .map_err(error)
    }
    fn close(&mut self, fd: i32) {
        let _ = libc::close(libc::Fd(fd as u32));
    }
    fn directory(&mut self, name: &str, context: &ShellExecutionContext) -> Result<String, Error> {
        let name = path(name, context)?;
        if !stat_is_dir(&name) {
            return Err(Error::new(1, "cd: no such directory"));
        }
        Ok(name)
    }
    fn source(&mut self, name: &str, context: &ShellExecutionContext) -> Result<String, Error> {
        let fd = libc::open(path(name, context)?.as_bytes())
            .map_err(|e| Error::new(1, format!("open script {name}: {e:?}")))?;
        let result = (|| {
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = libc::read(fd, &mut buf)
                    .map_err(|e| Error::new(1, format!("read script {name}: {e:?}")))?;
                if n == 0 {
                    break;
                }
                if bytes.len() + n > MAX_SOURCE {
                    return Err(Error::new(2, "script exceeds 64 KiB limit"));
                }
                bytes
                    .try_reserve(n)
                    .map_err(|_| Error::new(2, "out of memory reading script"))?;
                bytes.extend_from_slice(&buf[..n]);
            }
            String::from_utf8(bytes).map_err(|_| Error::new(2, "script is not UTF-8"))
        })();
        let _ = libc::close(fd);
        result
    }
}
struct OwnedFd(libc::Fd);
impl Drop for OwnedFd {
    fn drop(&mut self) {
        let _ = libc::close(self.0);
    }
}
fn drain(
    captures: &mut sunshell::capture::Captures,
    mut channels: Vec<(i32, OwnedFd, i32)>,
) -> Result<(), Error> {
    let mut buffer = [0; 4096];
    while !channels.is_empty() {
        checkpoint()?;
        let mut fds = [libc::PollFd {
            fd: -1,
            events: 1,
            revents: 0,
        }; 3];
        for (slot, (_, reader, _)) in fds.iter_mut().zip(&channels) {
            slot.fd = reader.0 .0 as i32;
        }
        let ready = libc::poll(&mut fds[..channels.len()], -1);
        checkpoint()?;
        ready.map_err(error)?;
        for index in (0..channels.len()).rev() {
            if fds[index].revents == 0 {
                continue;
            }
            match libc::read(channels[index].1 .0, &mut buffer) {
                Ok(0) => {
                    channels.swap_remove(index);
                }
                Ok(n) => captures.append(channels[index].0, &buffer[..n])?,
                Err(libc::Errno::Again) => {}
                Err(e) => return Err(error(e)),
            }
        }
    }
    Ok(())
}
pub(super) fn context() -> ShellExecutionContext {
    let mut env = sunshell::shellenv::ShellEnv::new();
    for i in 0..sunlight_ipc::spawn_env::MAX_ENTRIES {
        let ptr = unsafe { libc::env::entry(i) };
        if ptr.is_null() {
            break;
        }
        let len = unsafe { libc::crt0::cstr_len(ptr, sunlight_ipc::spawn_env::MAX_ENTRY_BYTES) };
        if let Ok(entry) = core::str::from_utf8(unsafe { core::slice::from_raw_parts(ptr, len) }) {
            if let Some((key, value)) = entry.split_once('=') {
                if sunshell::expansion::valid_name(key) {
                    env.set(key, value);
                    env.export(key);
                }
            }
        }
    }
    env.pid = libc::getpid();
    let mut cwd = [0; libc::MAX_PATH];
    let cwd = libc::getcwd(&mut cwd)
        .ok()
        .and_then(|n| core::str::from_utf8(&cwd[..n]).ok())
        .unwrap_or("/");
    ShellExecutionContext::new(env, String::from(cwd))
}
pub(super) fn start(raw: &[*const u8]) -> ! {
    let mut args = Vec::new();
    for ptr in raw {
        let len = unsafe { libc::crt0::cstr_len(*ptr, 1024) };
        let Ok(arg) = core::str::from_utf8(unsafe { core::slice::from_raw_parts(*ptr, len) })
        else {
            libc::exit(2);
        };
        args.push(String::from(arg));
    }
    let mut context = context();
    let mut runtime = NativeRuntime::new();
    let source = if args[1] == "-c" {
        if args.len() < 3 {
            let _ = runtime.write(2, b"sshl: -c requires a command\n");
            libc::exit(2);
        }
        context.env.arg0 = args.get(3).cloned().unwrap_or_else(|| args[0].clone());
        context.env.positional = args.get(4..).unwrap_or_default().to_vec();
        Ok(args[2].clone())
    } else {
        context.env.arg0 = args[1].clone();
        context.env.positional = args[2..].to_vec();
        runtime.source(&args[1], &context)
    };
    let result =
        source.and_then(|source| sunshell::script::execute(&source, &mut context, &mut runtime));
    let status = match result {
        Ok(Flow::Status(s) | Flow::Exit(s)) => s,
        Ok(_) => 2,
        Err(e) => {
            let _ = runtime.write(2, format!("sshl: {}\n", e.message).as_bytes());
            e.status
        }
    };
    libc::exit(status.rem_euclid(256) as u64)
}
