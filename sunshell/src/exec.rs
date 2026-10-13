use std::fs::File;
use std::io;
use std::process::{Command, ExitStatus, Stdio};

use crate::parser::{AstNode, RedirectMode, Redirection, SimpleCommand};
use crate::shellenv::ShellEnv;

#[derive(Debug)]
pub enum ExecError {
    NotFound(String),
    Io(io::Error),
}
impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(cmd) => write!(f, "{cmd}: command not found"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl From<io::Error> for ExecError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub trait Executor {
    fn run(&self, argv: &[&str]) -> Result<i32, ExecError>;
}
pub struct PosixExecutor;
impl Executor for PosixExecutor {
    fn run(&self, argv: &[&str]) -> Result<i32, ExecError> {
        let (cmd, args) = argv.split_first().expect("argv must be non-empty");
        let status = Command::new(cmd).args(args).status().map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                ExecError::NotFound((*cmd).to_string())
            } else {
                ExecError::Io(e)
            }
        })?;
        Ok(exit_code(status))
    }
}

enum IoTarget {
    Inherit,
    File(File),
}
impl IoTarget {
    fn into_stdio(self) -> io::Result<Stdio> {
        Ok(match self {
            Self::Inherit => Stdio::inherit(),
            Self::File(file) => Stdio::from(file),
        })
    }
    fn duplicate(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Inherit => Self::Inherit,
            Self::File(file) => Self::File(file.try_clone()?),
        })
    }
}

/// Execute a parsed command tree. Every stage is started before any stage is
/// waited on, so kernel pipe capacity never forces the shell to buffer output.
pub fn execute_ast(node: &AstNode, env: &ShellEnv) -> Result<i32, ExecError> {
    let commands: Vec<&SimpleCommand> = match node {
        AstNode::Command(command) => vec![command],
        AstNode::Pipeline(commands) => commands.iter().collect(),
    };
    let mut children: Vec<std::process::Child> = Vec::with_capacity(commands.len());
    let mut pipes = Vec::with_capacity(commands.len().saturating_sub(1));
    for _ in 1..commands.len() {
        pipes.push(create_pipe()?);
    }
    macro_rules! abort_pipeline {
        ($error:expr) => {{
            drop(pipes);
            for child in &mut children {
                let _ = child.kill();
            }
            for child in &mut children {
                let _ = child.wait();
            }
            return Err(ExecError::Io($error));
        }};
    }
    for (index, simple) in commands.iter().enumerate() {
        let expanded: Vec<String> = simple
            .argv
            .iter()
            .map(|arg| env.expand_token(arg))
            .collect();
        if expanded.is_empty() {
            return Err(ExecError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty command",
            )));
        }
        let last = index + 1 == commands.len();
        let mut io_map = [IoTarget::Inherit, IoTarget::Inherit, IoTarget::Inherit];
        if index > 0 {
            io_map[0] = match pipes[index - 1].0.try_clone() {
                Ok(file) => IoTarget::File(file),
                Err(e) => abort_pipeline!(e),
            };
        }
        if !last {
            io_map[1] = match pipes[index].1.try_clone() {
                Ok(file) => IoTarget::File(file),
                Err(e) => abort_pipeline!(e),
            };
        }
        for redirection in &simple.redirections {
            match redirection {
                Redirection::Open { fd, mode, path } => {
                    let file = match mode {
                        RedirectMode::Read => match File::open(path) {
                            Ok(file) => file,
                            Err(e) => abort_pipeline!(e),
                        },
                        RedirectMode::Truncate => match File::options()
                            .write(true)
                            .create(true)
                            .truncate(true)
                            .open(path)
                        {
                            Ok(file) => file,
                            Err(e) => abort_pipeline!(e),
                        },
                        RedirectMode::Append => match File::options()
                            .write(true)
                            .create(true)
                            .append(true)
                            .open(path)
                        {
                            Ok(file) => file,
                            Err(e) => abort_pipeline!(e),
                        },
                    };
                    io_map[*fd as usize] = IoTarget::File(file);
                }
                Redirection::Dup { fd, source } => {
                    io_map[*fd as usize] = match io_map[*source as usize].duplicate() {
                        Ok(target) => target,
                        Err(e) => abort_pipeline!(e),
                    }
                }
            }
        }
        let mut child_cmd = if expanded[0] == "cd" {
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg("cd -- \"$@\"")
                .arg("sshl-cd")
                .args(&expanded[1..]);
            command
        } else {
            let mut command = Command::new(&expanded[0]);
            command.args(&expanded[1..]);
            command
        };
        let [stdin, stdout, stderr] = io_map;
        child_cmd.stdin(match stdin.into_stdio() {
            Ok(io) => io,
            Err(e) => abort_pipeline!(e),
        });
        child_cmd.stdout(match stdout.into_stdio() {
            Ok(io) => io,
            Err(e) => abort_pipeline!(e),
        });
        child_cmd.stderr(match stderr.into_stdio() {
            Ok(io) => io,
            Err(e) => abort_pipeline!(e),
        });
        match child_cmd.spawn() {
            Ok(child) => {
                children.push(child);
            }
            Err(e) => {
                drop(pipes);
                for child in &mut children {
                    let _ = child.kill();
                }
                for child in &mut children {
                    let _ = child.wait();
                }
                return if e.kind() == io::ErrorKind::NotFound {
                    Err(ExecError::NotFound(expanded[0].clone()))
                } else {
                    Err(ExecError::Io(e))
                };
            }
        }
    }
    // Close the parent's endpoint copies before waiting, so EOF and EPIPE are
    // driven solely by the process descriptors that actually use each end.
    drop(pipes);
    let mut final_status = 0;
    let child_count = children.len();
    for (index, child) in children.iter_mut().enumerate() {
        let status = child.wait()?;
        if index + 1 == child_count {
            final_status = exit_code(status);
        }
    }
    Ok(final_status)
}

#[cfg(unix)]
fn create_pipe() -> io::Result<(File, File)> {
    use std::os::fd::FromRawFd;
    unsafe extern "C" {
        fn pipe2(fds: *mut i32, flags: i32) -> i32;
    }
    let mut fds = [-1; 2];
    // Linux O_CLOEXEC: child stages inherit only the endpoints explicitly
    // assigned by this executor, so unrelated and unused ends cannot hold EOF.
    if unsafe { pipe2(fds.as_mut_ptr(), 0x80000) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful pipe() returns two fresh owned descriptors.
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}

#[cfg(not(unix))]
fn create_pipe() -> io::Result<(File, File)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "host pipelines require Unix pipes",
    ))
}

fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        return 128 + status.signal().unwrap_or(1);
    }
    #[cfg(not(unix))]
    {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_line_checked;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> String {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sshl-phase1-{label}-{}-{stamp}",
                std::process::id()
            ))
            .display()
            .to_string()
    }
    fn run(line: &str) -> i32 {
        let node = parse_line_checked(line).unwrap().unwrap();
        execute_ast(&node, &ShellEnv::new()).unwrap()
    }

    #[test]
    fn two_and_three_stage_streaming_pipelines() {
        assert_eq!(run("printf hello | tr a-z A-Z"), 0);
        assert_eq!(run("printf 'one\\ntwo\\n' | cat | wc -l"), 0);
    }

    #[test]
    fn streams_more_than_pipe_capacity_and_handles_early_close() {
        let count = temp_path("count");
        assert_eq!(
            run(&format!("head -c 1048576 /dev/zero | wc -c > {count}")),
            0
        );
        assert_eq!(std::fs::read_to_string(&count).unwrap().trim(), "1048576");
        let _ = std::fs::remove_file(&count);
        let output = temp_path("early");
        assert_eq!(run(&format!("yes x | head -c 1 > {output}")), 0);
        assert_eq!(std::fs::metadata(&output).unwrap().len(), 1);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn input_append_truncate_stderr_and_ordered_duplication() {
        let file = temp_path("redir");
        let errors = temp_path("stderr");
        assert_eq!(run(&format!("printf old > {file}")), 0);
        assert_eq!(run(&format!("printf new > {file}")), 0);
        assert_eq!(run(&format!("printf '+app' >> {file}")), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new+app");
        assert_eq!(run(&format!("cat < {file} | wc -c > {errors}")), 0);
        assert_eq!(std::fs::read_to_string(&errors).unwrap().trim(), "7");
        let stderr_only = temp_path("stderr-only");
        assert_eq!(run(&format!("sh -c 'printf err >&2' 2> {stderr_only}")), 0);
        assert_eq!(std::fs::read_to_string(&stderr_only).unwrap(), "err");
        let merged = temp_path("merged");
        assert_eq!(
            run(&format!(
                "sh -c 'printf out; printf err >&2' > {merged} 2>&1"
            )),
            0
        );
        assert_eq!(std::fs::read_to_string(&merged).unwrap(), "outerr");
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_file(errors);
        let _ = std::fs::remove_file(stderr_only);
        let _ = std::fs::remove_file(merged);
    }

    #[test]
    fn reports_final_status_missing_commands_and_failed_middle_stage() {
        assert_eq!(run("sh -c 'exit 3' | cat"), 0);
        let node = parse_line_checked("printf x | definitely-not-an-sshl-command | cat")
            .unwrap()
            .unwrap();
        assert!(matches!(
            execute_ast(&node, &ShellEnv::new()),
            Err(ExecError::NotFound(_))
        ));
        assert_eq!(run("sh -c 'exit 9'"), 9);
    }

    #[test]
    fn builtin_pipeline_runs_in_a_child_environment() {
        let before = std::env::current_dir().unwrap();
        assert_eq!(run("cd /tmp | cat"), 0);
        assert_eq!(std::env::current_dir().unwrap(), before);
        assert_eq!(run("pwd | cat"), 0);
    }

    #[test]
    fn repeated_runs_and_redirection_open_failure_are_clean() {
        for _ in 0..25 {
            assert_eq!(run("printf x | cat | wc -c"), 0);
        }
        let node = parse_line_checked("printf x > /definitely-missing-dir/sshl-out")
            .unwrap()
            .unwrap();
        assert!(matches!(
            execute_ast(&node, &ShellEnv::new()),
            Err(ExecError::Io(_))
        ));
    }
}
