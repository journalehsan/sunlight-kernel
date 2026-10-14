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
pub struct HostRuntime;
fn path(path: &str, context: &ShellExecutionContext) -> PathBuf {
    Path::new(&context.cwd).join(path)
}
fn io_error(e: std::io::Error) -> Error {
    Error::new(1, e.to_string())
}
impl Runtime for HostRuntime {
    fn external(
        &mut self,
        command: &AstNode<String>,
        context: &ShellExecutionContext,
        stage_environments: Option<&[ShellEnv]>,
    ) -> Result<i32, Error> {
        if stage_environments.is_none() {
            context.env.export_size().map_err(|e| Error::new(2, e))?;
        }
        match crate::exec::execute_with_environments(command, Some(context), stage_environments) {
            Ok(status) => Ok(status),
            Err(e) => {
                self.write(context.io[2], format!("sshl: {e}\n").as_bytes())?;
                Ok(e.status())
            }
        }
    }
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<(), Error> {
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
        let result = execute(source, &mut context, &mut HostRuntime);
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
        let failure = execute("/usr/bin/true", &mut context, &mut HostRuntime).unwrap_err();
        assert_eq!(failure.status, 2);
        assert!(failure.message.contains("environment"));
        assert_eq!(
            execute("unset BIG; /usr/bin/true", &mut context, &mut HostRuntime).unwrap(),
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
            "echo $(true)",
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
}
