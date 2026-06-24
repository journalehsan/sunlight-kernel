use std::io;
use std::process::{Child, Command, Stdio};

use crate::parser::AstNode;
use crate::shellenv::ShellEnv;

#[derive(Debug)]
pub enum ExecError {
    NotFound(String),
    Io(io::Error),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NotFound(cmd) => write!(f, "{cmd}: command not found"),
            ExecError::Io(e) => write!(f, "{e}"),
        }
    }
}

pub trait Executor {
    fn run(&self, argv: &[&str]) -> Result<i32, ExecError>;
}

/// v0.1/v0.2 — delegates to std::process.
pub struct PosixExecutor;

impl Executor for PosixExecutor {
    fn run(&self, argv: &[&str]) -> Result<i32, ExecError> {
        let (cmd, args) = argv.split_first().expect("argv must be non-empty");

        let status = std::process::Command::new(cmd)
            .args(args)
            .status()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    ExecError::NotFound(cmd.to_string())
                } else {
                    ExecError::Io(e)
                }
            })?;

        Ok(status.code().unwrap_or(1))
    }
}

/// Execute an AST (single Command or Pipeline).
/// Expands $VAR / ${VAR} using the provided ShellEnv immediately before spawn.
pub fn execute_ast(node: &AstNode, env: &ShellEnv) -> Result<i32, ExecError> {
    match node {
        AstNode::Command(args) => {
            if args.is_empty() {
                return Ok(0);
            }
            let expanded_args: Vec<String> = args.iter().map(|arg| env.expand_token(arg)).collect();

            let mut cmd = Command::new(&expanded_args[0]);
            cmd.args(&expanded_args[1..]);

            let mut child = cmd.spawn().map_err(ExecError::Io)?;
            Ok(child.wait().map_err(ExecError::Io)?.code().unwrap_or(1))
        }
        AstNode::Pipeline(commands) => {
            let mut prev_process: Option<Child> = None;
            let mut last_status = 0;

            let mut iter = commands.iter().peekable();

            while let Some(AstNode::Command(args)) = iter.next() {
                if args.is_empty() {
                    continue;
                }
                let is_last = iter.peek().is_none();
                let expanded_args: Vec<String> =
                    args.iter().map(|arg| env.expand_token(arg)).collect();

                let mut cmd = Command::new(&expanded_args[0]);
                cmd.args(&expanded_args[1..]);

                // Plumb stdin from the previous command's stdout
                if let Some(mut prev) = prev_process.take() {
                    if let Some(stdout) = prev.stdout.take() {
                        cmd.stdin(stdout);
                    }
                }

                // Plumb stdout to the next command (unless last)
                if !is_last {
                    cmd.stdout(Stdio::piped());
                }

                let child = cmd.spawn().map_err(ExecError::Io)?;

                if is_last {
                    last_status = child
                        .wait_with_output()
                        .map_err(ExecError::Io)?
                        .status
                        .code()
                        .unwrap_or(1);
                } else {
                    prev_process = Some(child);
                }
            }
            Ok(last_status)
        }
    }
}
