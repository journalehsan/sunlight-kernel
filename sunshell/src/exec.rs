use std::io;

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
