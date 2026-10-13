// Descriptor-based noninteractive platform adapter. No terminal registration.
use super::*;
use alloc::{format, string::String, vec::Vec};
use sunlight_libc as libc;
use sunshell::{
    parser::{AstNode, RedirectMode},
    script::{Error, Flow, Runtime, ShellExecutionContext},
    script_parser::MAX_SOURCE,
};

pub(super) struct NativeRuntime {
    shell: Shell,
    inherited_env: sunshell::shellenv::ShellEnv,
}
impl NativeRuntime {
    pub(super) fn new() -> Self {
        Self {
            shell: Shell::new(),
            inherited_env: context().env,
        }
    }
}
fn error(e: libc::Errno) -> Error {
    Error::new(1, format!("native I/O: {e:?}"))
}
fn path(name: &str, context: &ShellExecutionContext) -> Result<String, Error> {
    normalize_path(&context.cwd, name).ok_or_else(|| Error::new(1, "invalid path"))
}
impl Runtime for NativeRuntime {
    fn external(
        &mut self,
        command: &AstNode<String>,
        context: &ShellExecutionContext,
    ) -> Result<i32, Error> {
        if !context.env.exported().eq(self.inherited_env.exported()) {
            return Err(Error::new(
                2,
                "native spawn cannot pass changed shell exports; inherited environment only",
            ));
        }
        self.shell.cwd = context.cwd.clone();
        self.shell.env = context.env.clone();
        self.shell.script_io = Some(context.io);
        let mut old = [0u8; libc::MAX_PATH];
        let len = libc::getcwd(&mut old)
            .map_err(|e| Error::new(1, format!("getcwd before spawn: {e:?}")))?;
        libc::chdir(context.cwd.as_bytes())
            .map_err(|e| Error::new(1, format!("chdir before spawn: {e:?}")))?;
        let commands = match command {
            AstNode::Command(command) => core::slice::from_ref(command),
            AstNode::Pipeline(commands) => commands.as_slice(),
        };
        let result = self.shell.launch_pipeline(commands);
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
            let mut status = 0;
            for pid in pids {
                status = libc::waitpid(pid).map(|s| s as i32).unwrap_or(1);
            }
            status
        };
        libc::chdir(&old[..len]).map_err(error)?;
        Ok(status)
    }
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<(), Error> {
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
pub(super) fn context() -> ShellExecutionContext {
    let mut env = sunshell::shellenv::ShellEnv::new();
    env.load_defaults(libc::getuid() as u32, "");
    for i in 0..128 {
        let ptr = unsafe { libc::env::entry(i) };
        if ptr.is_null() {
            break;
        }
        let len = unsafe { libc::crt0::cstr_len(ptr, 16384) };
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
