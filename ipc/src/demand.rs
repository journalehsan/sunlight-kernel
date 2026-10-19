//! D1 activation protocol. Discovery returns metadata, never a service capability.
//! ACTIVATE requires kernel-checked ServiceLifecycle authority. Invocation still
//! requires the caller's own nameserver lookup and the service's normal checks.
use crate::{CapabilityToken, IpcMsg, ServiceCapability, SunlightSyscall};

pub const DISCOVER: u64 = 0xD100;
pub const ACTIVATE: u64 = 0xD101;
pub const HELLO: u64 = 0xD102;
pub const READY: u64 = 0xD103;
pub const METRICS: u64 = 0xD104;
/// D2 primary-service report: generation, busy work/session/resource count,
/// uncommitted persistent transactions. Only zero/zero nominates idle.
pub const IDLE_REPORT: u64 = 0xD200;
/// Administrative residency lease: activation generation, PID, process
/// generation, lifetime_ms. Does not activate an inactive service.
pub const LEASE_ACQUIRE: u64 = 0xD201;
/// Owner-only release: lease ID, activation generation.
pub const LEASE_RELEASE: u64 = 0xD202;
pub const IDLE_METRICS: u64 = 0xD203;
pub const IDLE_ACCOUNT: u64 = 0xD204;
/// Manager -> service: activation generation, monotonically increasing nonce.
/// ACK echoes both, and attests closed admission, zero accepted work/resources
/// and zero uncommitted transactions. Unknown opcodes MUST NOT be acknowledged.
pub const QUIESCE: u64 = 0xD210;
/// Only accepted behind the closed admission barrier for the exact nonce.
pub const COMMIT_EXIT: u64 = 0xD211;
pub const DRAINED: u64 = 0xD280;
pub const REAP_QUERY: u64 = 0x4432524541500001;
pub const REAP_RELEASE: u64 = 0x4432524541500002;
pub const ACCOUNT_QUERY: u64 = 0x4432524541500003;
/// Explicit manager-only spawn mode, used only for cooperative D2 units.
pub const SPAWN_TRACKED: u64 = 0xD220;
pub const LEASE_MAX_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceStatus {
    Live,
    Exiting,
    Reaped {
        private_frames: u64,
        page_tables: u64,
        free_frames: u64,
        exit_code: i32,
    },
}

/// Trusted manager only. Missing/overwritten receipts fail closed. This is
/// deliberately stronger than the live-process credential query.
pub fn instance_status(pid: u32, generation: u64) -> Option<InstanceStatus> {
    let (status, msg) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::SessionGetCredentials,
            pid as u64,
            REAP_QUERY,
            generation,
            0,
            0,
            0,
            0,
        )
    };
    match status {
        0 => Some(InstanceStatus::Live),
        1 => Some(InstanceStatus::Exiting),
        2 => Some(InstanceStatus::Reaped {
            private_frames: msg.words[0],
            page_tables: msg.words[1],
            free_frames: msg.words[2],
            exit_code: msg.words[3] as i32,
        }),
        _ => None,
    }
}

pub fn release_reap_receipt(pid: u32, generation: u64) -> bool {
    let (status, _) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::SessionGetCredentials,
            pid as u64,
            REAP_RELEASE,
            generation,
            0,
            0,
            0,
            0,
        )
    };
    status == 0
}

/// Explicit diagnostic sample: scheduler runtime ns, mapped user pages,
/// shared mapping pages and global PMM free frames. Not an RSS reclamation claim.
pub fn instance_account(pid: u32, generation: u64) -> Option<[u64; 4]> {
    let (status, msg) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::SessionGetCredentials,
            pid as u64,
            ACCOUNT_QUERY,
            generation,
            0,
            0,
            0,
            0,
        )
    };
    (status == 0).then_some([msg.words[0], msg.words[1], msg.words[2], msg.words[3]])
}
pub const OK: u64 = 0xD180;
pub const ERROR: u64 = 0xD1FF;
pub const TIMEOUT_MS: u64 = 5_000;
pub const RETRY_MS: u64 = 1_000;
pub const NOT_FOUND: u64 = 1;
pub const BUSY: u64 = 2;
pub const START_FAILED: u64 = 3;
pub const TIMED_OUT: u64 = 4;
pub const DIED: u64 = 5;
pub const STALE: u64 = 6;
pub const DEPENDENCY: u64 = 7;
pub const STOPPED: u64 = 8;
pub const EXHAUSTED: u64 = 9;
/// Explicit mode in the signal argument; legacy two-argument kill callers
/// need not initialize otherwise-unused registers.
pub const KILL_GENERATION_FLAG: u64 = 1 << 63;

pub fn activation_authorized(restrictions: Option<u64>) -> bool {
    restrictions.is_none_or(|mask| mask & ServiceCapability::ServiceLifecycle.bit() != 0)
}

pub fn error(code: u64) -> IpcMsg {
    IpcMsg::with_label(ERROR).word(0, code)
}

/// Trusted SunlightD only. Checks a SEND capability against the expected live
/// process incarnation and returns its actual endpoint ID. Uses the existing
/// nameserver validation syscall's manager mode, not userspace identity claims.
pub fn endpoint_identity(cap: CapabilityToken, pid: u32, generation: u64) -> Option<u32> {
    if pid == 0 || generation == 0 {
        return None;
    }
    let (ret, _) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::NameserverEndpointValidate,
            cap.0,
            pid as u64,
            generation,
            0,
            0,
            0,
            0,
        )
    };
    ret.checked_sub(1).and_then(|id| u32::try_from(id).ok())
}

/// Name occupies four register words. Reject truncation, embedded NUL and
/// non-ASCII rather than silently selecting a different unit.
pub fn request(op: u64, name: &str) -> Option<IpcMsg> {
    if name.is_empty() || name.len() > 31 || !name.is_ascii() || name.as_bytes().contains(&0) {
        return None;
    }
    let mut msg = IpcMsg::with_label(op);
    for (i, b) in name.bytes().enumerate() {
        msg.words[i / 8] |= (b as u64) << ((i % 8) * 8);
    }
    msg.word_count = 4;
    Some(msg)
}

/// Manager-only generation-qualified use of existing kill syscall.
pub fn kill_instance(pid: u32, generation: u64, signal: u32) -> bool {
    if generation == 0 {
        return false;
    }
    let (ret, _) = unsafe {
        crate::raw_syscall(
            SunlightSyscall::Kill,
            pid as u64,
            signal as u64 | KILL_GENERATION_FLAG,
            generation,
            0,
            0,
            0,
            0,
        )
    };
    ret == 0
}
