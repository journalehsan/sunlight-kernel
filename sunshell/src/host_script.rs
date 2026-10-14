//! Host platform adapter; language semantics remain in the no_std library.
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{BorrowedFd, FromRawFd, IntoRawFd},
    path::{Path, PathBuf},
};
use sunshell::{
    parser::{AstNode, RedirectMode},
    script::{Error, Runtime, ShellExecutionContext},
    script_parser::MAX_SOURCE,
    shellenv::ShellEnv,
};
#[derive(Default)]
pub struct HostRuntime {
    captures: sunshell::capture::Captures,
}
fn path(path: &str, context: &ShellExecutionContext) -> PathBuf {
    Path::new(&context.cwd).join(path)
}
fn io_error(e: std::io::Error) -> Error {
    Error::new(1, e.to_string())
}
impl Runtime for HostRuntime {
    fn begin_capture(&mut self) -> Result<i32, Error> {
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
            use std::os::fd::AsRawFd;
            let (reader, writer) = crate::exec::create_pipe().map_err(io_error)?;
            let writer_fd = writer.as_raw_fd();
            channels.push((*fd, reader, writer_fd));
            writers.push(writer);
            *fd = writer_fd;
        }
        let mut capture_error = None;
        let result = crate::exec::execute_streaming(
            command,
            Some(context),
            stage_environments,
            Some(mapped),
            || {
                drop(writers);
                let result = drain(&mut self.captures, channels);
                if let Err(e) = result {
                    capture_error = Some(e);
                    return Err(std::io::Error::other("capture failed"));
                }
                Ok(())
            },
        );
        if let Some(mut error) = capture_error {
            error.expansion_fatal = true;
            return Err(error);
        }
        match result {
            Ok(status) => Ok(status),
            Err(e) => {
                self.write(context.io[2], format!("sshl: {e}\n").as_bytes())?;
                Ok(e.status())
            }
        }
    }

    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<(), Error> {
        if fd < 0 {
            return self.captures.append(fd, bytes);
        }
        let owned = unsafe { BorrowedFd::borrow_raw(fd) }
            .try_clone_to_owned()
            .map_err(io_error)?;
        File::from(owned).write_all(bytes).map_err(io_error)
    }
    fn open(
        &mut self,
        name: &str,
        mode: RedirectMode,
        context: &ShellExecutionContext,
    ) -> Result<i32, Error> {
        let name = path(name, context);
        let file = match mode {
            RedirectMode::Read => File::open(name),
            RedirectMode::Truncate => File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(name),
            RedirectMode::Append => File::options()
                .write(true)
                .create(true)
                .append(true)
                .open(name),
        }
        .map_err(io_error)?;
        Ok(file.into_raw_fd())
    }
    fn close(&mut self, fd: i32) {
        drop(unsafe { File::from_raw_fd(fd) });
    }
    fn directory(&mut self, name: &str, context: &ShellExecutionContext) -> Result<String, Error> {
        let resolved = std::fs::canonicalize(path(name, context)).map_err(io_error)?;
        if !resolved.is_dir() {
            return Err(Error::new(1, "cd: not a directory"));
        }
        Ok(resolved.to_string_lossy().into_owned())
    }
    fn source(&mut self, name: &str, context: &ShellExecutionContext) -> Result<String, Error> {
        let mut file = File::open(path(name, context)).map_err(io_error)?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_SOURCE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > MAX_SOURCE {
            return Err(Error::new(2, "script exceeds 64 KiB limit"));
        }
        String::from_utf8(bytes).map_err(|_| Error::new(2, "script is not UTF-8"))
    }
}

/// At most three mapped sinks are active for one external pipeline. Poll
/// services each ready reader once per pass, including EOF/HUP, without a
/// wait-before-drain cycle or a thread copying interpreter state.
fn drain(
    captures: &mut sunshell::capture::Captures,
    mut channels: Vec<(i32, File, i32)>,
) -> Result<(), Error> {
    use std::os::fd::AsRawFd;
    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    unsafe extern "C" {
        fn poll(fds: *mut PollFd, count: usize, timeout: i32) -> i32;
    }
    let mut buffer = [0; 4096];
    while !channels.is_empty() {
        let mut fds = [const {
            PollFd {
                fd: -1,
                events: 1,
                revents: 0,
            }
        }; 3];
        for (slot, (_, reader, _)) in fds.iter_mut().zip(&channels) {
            slot.fd = reader.as_raw_fd();
        }
        if unsafe { poll(fds.as_mut_ptr(), channels.len(), -1) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(io_error(error));
        }
        for index in (0..channels.len()).rev() {
            if fds[index].revents == 0 {
                continue;
            }
            match channels[index].1.read(&mut buffer) {
                Ok(0) => {
                    channels.swap_remove(index);
                }
                Ok(n) => captures.append(channels[index].0, &buffer[..n])?,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(e) => return Err(io_error(e)),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use sunshell::{
        script::{execute, Flow},
        shellenv::ShellEnv,
    };
    static ID: AtomicUsize = AtomicUsize::new(0);
    fn run(source: &str, args: &[&str]) -> (Result<Flow, Error>, String, ShellExecutionContext) {
        let path = std::env::temp_dir().join(format!(
            "sshl-script-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let output = File::create(&path).unwrap();
        let mut env = ShellEnv::new();
        env.set("PATH", "/bin:/usr/bin");
        env.export("PATH");
        env.positional = args.iter().map(|s| s.to_string()).collect();
        env.arg0 = "script.sh".into();
        let mut context = ShellExecutionContext::new(env, "/".into());
        context.io[1] = output.as_raw_fd();
        let result = execute(source, &mut context, &mut HostRuntime::default());
        drop(output);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        (result, text, context)
    }
    #[test]
    fn nested_control_and_short_circuit() {
        let (result, output, _) = run("if false; then echo bad; elif true; then if ! false; then echo yes; fi; fi\nfalse && echo bad || echo alternative\nfor a in one two; do for b in x y; do echo $a:$b; done; done", &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "yes\nalternative\none:x\none:y\ntwo:x\ntwo:y\n");
    }
    #[test]
    fn functions_parameters_return_and_isolation() {
        let (result, output, context) = run("f() { echo \"$0:$#:$1\"; set -- changed; inner; return 7; echo bad; }; inner() { echo inner:$#; }; f arg; echo status:$?:$1; value=parent; (value=child; cd /tmp; echo $value); echo $value; pwd", &["original"]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(
            output,
            "script.sh:1:arg\ninner:0\nstatus:7:original\nchild\nparent\n/\n"
        );
        assert_eq!(context.cwd, "/");
    }
    #[test]
    fn loop_control_and_case_patterns() {
        let (result, output, _) = run("ready=no; until test \"$ready\" = yes; do ready=yes; done; while true; do for x in a b; do echo $x; break 2; done; done; for x in one two; do continue; echo bad; done; case abc in 'a*') echo bad;; a?c|xyz) echo case;; *) echo bad;; esac", &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "a\ncase\n");
    }
    #[test]
    fn quoted_at_empty_arguments_and_ifs() {
        let (result, output, _) = run("for x in \"$@\"; do echo \"<$x>\"; done; IFS=:; value=a::b; for x in $value; do echo \"[$x]\"; done; set --; for x in \"$@\"; do echo bad; done; echo \"$#\"", &["a b", "", "c"]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "<a b>\n<>\n<c>\n[a]\n[]\n[b]\n0\n");
    }
    #[test]
    fn descriptors_and_pipeline_status() {
        let path = format!("/tmp/sshl-compound-{}", std::process::id());
        let (result, output, _) = run(&format!("{{ echo group; echo two; }} > {path}; cat < {path} | cat; false | true; echo $?; (echo sub) >> {path}; cat {path}"), &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "group\ntwo\n0\ngroup\ntwo\nsub\n");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn exported_variables_and_prefix_scope() {
        let (result, output, context) = run("value=local; export EXPORTED=child; /usr/bin/printenv EXPORTED; value=temporary echo $value; echo $value; readonly LOCK=ok", &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "child\nlocal\nlocal\n");
        assert!(context.env.is_readonly("LOCK"));
        assert!(run("readonly X=one; X=two", &[]).0.is_err());
        assert_eq!(
            run("missing-sshl-command || true", &[]).0.unwrap(),
            Flow::Status(0)
        );
    }
    #[test]
    fn environment_handoff_unset_and_special_assignments() {
        let (result, output, context) = run("NAME=Sunlight; export NAME; sh -c 'echo \"$NAME\"'; NAME=temporary sh -c 'echo \"$NAME\"'; echo \"$NAME\"; LOCAL=hidden; sh -c 'echo \"<$LOCAL>\"'; unset NAME; sh -c 'echo \"<$NAME>\"'; NAME=permanent :; echo \"$NAME\"; NAME=changed export NAME; sh -c 'echo \"$NAME\"'", &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(
            output,
            "Sunlight\ntemporary\nSunlight\n<>\n<>\npermanent\nchanged\n"
        );
        assert_eq!(context.env.get("NAME"), Some("changed"));
        assert_eq!(context.env.get("LOCAL"), Some("hidden"));
    }
    #[test]
    fn pipeline_assignments_have_independent_environments() {
        let (result, output, context) = run("NAME=parent; export NAME; NAME=left sh -c 'echo \"$NAME\"' | NAME=right sh -c 'echo \"$NAME\"; cat'; echo \"$NAME\"", &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "right\nleft\nparent\n");
        assert_eq!(context.env.get("NAME"), Some("parent"));
        let too_many = vec!["true"; 17].join(" | ");
        assert!(run(&too_many, &[])
            .0
            .unwrap_err()
            .message
            .contains("16 stages"));
    }
    #[test]
    fn temporary_assignments_restore_attributes_and_survive_launch_errors() {
        let (result, output, context) = run("NAME=local; NAME=temporary missing-sshl-command || :; echo \"$NAME\"; sh -c 'echo \"<$NAME>\"'; NAME=temp readonly OTHER=fixed; echo \"$NAME:$OTHER\"; export UNASSIGNED; sh -c '/usr/bin/printenv UNASSIGNED'; echo $?", &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "local\n<>\ntemp:fixed\n1\n");
        assert!(context.env.is_readonly("OTHER"));
        assert_eq!(context.env.get("UNASSIGNED"), None);
        assert!(run("readonly NAME=locked; NAME=bad sh -c 'echo bad'", &[])
            .0
            .is_err());
    }
    #[test]
    fn environment_limit_failure_leaves_context_usable() {
        let mut env = ShellEnv::new();
        env.assign("BIG", &"x".repeat(4096)).unwrap();
        env.export("BIG");
        let mut context = ShellExecutionContext::new(env, "/".into());
        let failure =
            execute("/usr/bin/true", &mut context, &mut HostRuntime::default()).unwrap_err();
        assert_eq!(failure.status, 2);
        assert!(failure.message.contains("environment"));
        assert_eq!(
            execute(
                "unset BIG; /usr/bin/true",
                &mut context,
                &mut HostRuntime::default()
            )
            .unwrap(),
            Flow::Status(0)
        );
    }
    #[test]
    fn resource_limits_and_errors() {
        for script in [
            "f() { f; }; f",
            "return 1",
            "break",
            "echo *.rs",
            "echo ${X:-bad}",
            "f() { :; }; f | cat",
            "echo $((1+2))",
        ] {
            assert!(run(script, &[]).0.is_err(), "{script}");
        }
        for _ in 0..50 {
            assert_eq!(
                run("echo x | cat | cat; (x=child); :", &[]).0.unwrap(),
                Flow::Status(0)
            );
        }
    }

    #[test]
    fn command_substitution_isolation_status_and_no_second_expansion() {
        let (result, output, context) = run("NAME=parent; f() { NAME=child; cd /tmp; echo '$NAME'; return 7; }; v=$(f); echo \"$?:$v:$NAME\"; pwd; v=$(printf 'a\\nb\\n\\n'); echo \"<$v>\"; echo \"$(echo \"$(echo nested)\")\"", &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "7:$NAME:parent\n/\n<a\nb>\nnested\n");
        assert_eq!(context.env.get("NAME"), Some("parent"));
        assert_eq!(context.cwd, "/");
    }

    #[test]
    fn command_substitution_failures_release_capture_and_allow_recovery() {
        let mut context = ShellExecutionContext::new(ShellEnv::new(), "/".into());
        let mut runtime = HostRuntime::default();
        for source in [
            "v=$(head -c 1048576 /dev/zero | cat)",
            "v=$(printf '\\000')",
            "v=$(printf '\\377')",
            "v=$(head -c 16385 /dev/zero | tr '\\000' x)",
        ] {
            let error = execute(source, &mut context, &mut runtime).unwrap_err();
            assert_eq!(error.status, 2, "{source}");
            assert_eq!(
                execute("v=$(echo recovered)", &mut context, &mut runtime).unwrap(),
                Flow::Status(0)
            );
            assert_eq!(context.env.get("v"), Some("recovered"));
        }
        let mut source = String::from("echo x");
        for _ in 0..17 {
            source = format!("echo \"$({source})\"");
        }
        assert!(
            execute(&source, &mut context, &mut runtime)
                .unwrap_err()
                .expansion_fatal
        );
        assert_eq!(
            execute("v=$(true)", &mut context, &mut runtime).unwrap(),
            Flow::Status(0)
        );
        let producer = "head -c 98304 /dev/zero | tr '\\000' '\\n'";
        let mut source = producer.to_string();
        for _ in 0..2 {
            source = format!("{producer}; echo \"$({source})\"");
        }
        let error = execute(&format!("v=$({source})"), &mut context, &mut runtime).unwrap_err();
        assert!(error.expansion_fatal && error.message.contains("256 KiB total"));
        assert_eq!(
            execute("v=$(echo recovered)", &mut context, &mut runtime).unwrap(),
            Flow::Status(0)
        );
        assert_eq!(context.env.get("v"), Some("recovered"));
    }

    #[test]
    fn command_substitution_streams_nested_stdout_and_stderr() {
        let source = "v=$({ echo \"$(sh -c 'head -c 8192 /dev/zero | tr \"\\000\" y >&2; head -c 8192 /dev/zero | tr \"\\000\" x')\"; } 2>&1); echo \"$v\"";
        let (result, output, _) = run(source, &[]);
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(
            output,
            format!("{}{}\n", "y".repeat(8192), "x".repeat(8192))
        );
    }

    #[test]
    fn substitution_errors_follow_active_compound_redirections() {
        let path = format!("/tmp/sshl-sub-error-{}", std::process::id());
        let (result, output, _) = run(
            &format!("v=$({{ echo bad < /no-such-sshl-file; }} 2> {path}); echo \"$?:$v\""),
            &[],
        );
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert_eq!(output, "1:\n");
        assert!(!std::fs::read(&path).unwrap().is_empty());
        std::fs::remove_file(&path).unwrap();
        let (result, output, _) = run(
            "v=$({ echo bad < /no-such-sshl-file; } 2>&1); echo \"$v\"",
            &[],
        );
        assert_eq!(result.unwrap(), Flow::Status(0));
        assert!(output.starts_with("sshl: "));
        assert!(!output.contains("bad\n"));
    }
}
