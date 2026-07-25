//! Host-testable sunlightd core: unit parsing, dependency graph, and
//! lifecycle state machine. Process/spawn syscalls stay in the binary.

#![no_std]

extern crate alloc;

pub mod graph;
pub mod ipc;
pub mod journal;
pub mod socket_act;
pub mod supervisor;
pub mod unit;
