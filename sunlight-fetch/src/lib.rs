//! sunlight-fetch: Lightweight chunked HTTP downloader for SunlightOS
//!
//! Architecture:
//!   cli.rs       — Argument parsing, no allocations where possible
//!   ipc.rs       — NetOp IPC extensions for HTTP requests
//!   downloader.rs — Chunked download engine with Range fallback
//!   progress.rs  — Single-line ANSI TUI progress bar
//!
//! Security: All operations require explicit capabilities.
//! No ambient authority. Network via net_server IPC only.

#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod cli;
pub mod downloader;
pub mod error;
pub mod http;
pub mod ipc;
pub mod progress;

pub use error::{FetchError, FetchResult};
