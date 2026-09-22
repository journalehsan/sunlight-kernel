//! Read-only account presentation and narrowly scoped own-password IPC.
//! Account data is returned by UAC; session identity comes from sessiond.
#[cfg(target_os = "none")]
use crate::nameserver_lookup_timeout;
use crate::{
    ipc_call_timeout, shm_create, shm_free, CapabilityToken, IpcMsg, SessionMsg, SessionState,
    SESSION_ENDPOINT,
};

pub const SNAPSHOT: u64 = 0xC200;
pub const CHANGE_OWN_PASSWORD: u64 = 0xC201;
pub const REPLY: u64 = 0xC2FF;
pub const ERROR: u64 = 0xC2FE;
pub const VERSION: u32 = 1;
pub const MAX_USERS: usize = 16;
pub const MAX_GROUPS: usize = 32;
pub const SECRET_LIMIT: usize = 128;
pub const SHM_BYTES: usize = 4096;
pub const TIMEOUT_MS: u64 = 2000;

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable = 1,
    Denied = 2,
    IncorrectPassword = 3,
    InvalidInput = 4,
    SessionChanged = 5,
    Storage = 6,
    Unsupported = 7,
    CapabilityRejected = 8,
    ConcurrentChange = 9,
    BrokerUnavailable = 10,
}
impl Error {
    pub fn from_raw(raw: u64) -> Self {
        match raw {
            2 => Self::Denied,
            3 => Self::IncorrectPassword,
            4 => Self::InvalidInput,
            5 => Self::SessionChanged,
            6 => Self::Storage,
            7 => Self::Unsupported,
            8 => Self::CapabilityRejected,
            9 => Self::ConcurrentChange,
            10 => Self::BrokerUnavailable,
            _ => Self::Unavailable,
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "Account service unavailable",
            Self::Denied => "Authorization denied",
            Self::IncorrectPassword => "Current password is incorrect",
            Self::InvalidInput => "Invalid account request",
            Self::SessionChanged => "Session changed. Refresh and try again.",
            Self::Storage => "Could not save account changes",
            Self::Unsupported => "This account operation is not supported",
            Self::CapabilityRejected => "Authorization expired or is no longer valid",
            Self::ConcurrentChange => "Account changed concurrently. Refresh and try again.",
            Self::BrokerUnavailable => "Capability Broker unavailable. No changes were made.",
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    pub username: [u8; 64],
    pub display_name: [u8; 48],
}
impl User {
    pub const EMPTY: Self = Self {
        uid: 0,
        gid: 0,
        username: [0; 64],
        display_name: [0; 48],
    };
    pub fn login(&self) -> &str {
        text(&self.username)
    }
    pub fn name(&self) -> &str {
        let name = text(&self.display_name);
        if name.is_empty() {
            self.login()
        } else {
            name
        }
    }
    // UID 0 is the only implemented administrative identity. Wheel membership
    // alone does not grant an account operation in the existing broker.
    pub fn is_admin(&self) -> bool {
        self.uid == 0
    }
    pub fn avatar_id(&self) -> u32 {
        self.uid.wrapping_mul(2654435761)
    }
    pub fn initial(&self) -> &str {
        let n = self.name();
        n.get(..n.chars().next().map_or(0, char::len_utf8))
            .filter(|s| !s.is_empty())
            .unwrap_or("?")
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub gid: u32,
    pub members: u32,
    pub name: [u8; 32],
}
impl Group {
    pub const EMPTY: Self = Self {
        gid: 0,
        members: 0,
        name: [0; 32],
    };
    pub fn name(&self) -> &str {
        text(&self.name)
    }
}

/// Plain integer/byte wire representation: no pointers, bools, or enums.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub revision: u64,
    pub session_id: u64,
    pub generation: u64,
    pub version: u32,
    pub current_uid: u32,
    pub user_count: u32,
    pub group_count: u32,
    pub policy: u32,
    pub reserved: u32,
    pub users: [User; MAX_USERS],
    pub groups: [Group; MAX_GROUPS],
}
const _: () = assert!(core::mem::size_of::<Snapshot>() <= SHM_BYTES);
impl Snapshot {
    pub const EMPTY: Self = Self {
        revision: 0,
        session_id: 0,
        generation: 0,
        version: VERSION,
        current_uid: 0,
        user_count: 0,
        group_count: 0,
        policy: 0,
        reserved: 0,
        users: [User::EMPTY; MAX_USERS],
        groups: [Group::EMPTY; MAX_GROUPS],
    };
    pub fn users(&self) -> &[User] {
        &self.users[..(self.user_count as usize).min(MAX_USERS)]
    }
    pub fn groups(&self) -> &[Group] {
        &self.groups[..(self.group_count as usize).min(MAX_GROUPS)]
    }
    pub fn current(&self) -> Option<&User> {
        self.users().iter().find(|u| u.uid == self.current_uid)
    }
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && self.session_id != 0
            && self.user_count as usize <= MAX_USERS
            && self.group_count as usize <= MAX_GROUPS
            && self.current().is_some()
    }
}
pub fn text(bytes: &[u8]) -> &str {
    core::str::from_utf8(&bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())])
        .unwrap_or("")
}

fn service(name: &str) -> Option<CapabilityToken> {
    #[cfg(target_os = "none")]
    {
        nameserver_lookup_timeout(name, TIMEOUT_MS)
    }
    #[cfg(not(target_os = "none"))]
    {
        let _ = name;
        None
    }
}

pub fn active_session() -> Result<(u64, u64, u32), Error> {
    let ep = service(SESSION_ENDPOINT).ok_or(Error::Unavailable)?;
    let r = ipc_call_timeout(
        ep,
        IpcMsg::with_label(SessionMsg::SESSION_CURRENT_IDENTITY),
        TIMEOUT_MS,
    )
    .map_err(|_| Error::Unavailable)?;
    if r.label != SessionMsg::REPLY
        || r.word_count < 3
        || !matches!(
            SessionState::from_u64((r.words[2] >> 32) & 255),
            Some(SessionState::Running | SessionState::Degraded)
        )
    {
        return Err(Error::SessionChanged);
    }
    Ok((r.words[0], r.words[1], r.words[2] as u32))
}
fn endpoint() -> Result<CapabilityToken, Error> {
    service("uac").ok_or(Error::Unavailable)
}
fn check_reply(r: IpcMsg) -> Result<(), Error> {
    if r.label == REPLY {
        Ok(())
    } else if r.label == ERROR {
        Err(Error::from_raw(r.words[0]))
    } else {
        Err(Error::Unavailable)
    }
}
pub fn snapshot() -> Result<Snapshot, Error> {
    let ep = endpoint()?;
    let (ptr, token) = shm_create(SHM_BYTES, 0).map_err(|_| Error::Unavailable)?;
    let result = ipc_call_timeout(
        ep,
        IpcMsg::with_label(SNAPSHOT).with_cap(0, token),
        TIMEOUT_MS,
    )
    .map_err(|_| Error::Unavailable)
    .and_then(check_reply)
    .and_then(|()| {
        let result = unsafe { core::ptr::read_unaligned(ptr.cast::<Snapshot>()) };
        if result.valid() {
            Ok(result)
        } else {
            Err(Error::Unavailable)
        }
    });
    let _ = shm_free(token);
    result
}
/// Compiler-resistant clearing for GUI fields and shared request memory.
pub fn clear_secret(bytes: &mut [u8]) {
    for b in bytes {
        unsafe {
            core::ptr::write_volatile(b, 0);
        }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}
pub fn password_matches(new: &[u8], confirm: &[u8]) -> bool {
    !new.is_empty() && new.len() <= SECRET_LIMIT && new == confirm
}

pub fn change_own_password(session: &Snapshot, current: &[u8], new: &[u8]) -> Result<(), Error> {
    if current.is_empty()
        || current.len() > SECRET_LIMIT
        || new.is_empty()
        || new.len() > SECRET_LIMIT
        || current.contains(&0)
        || new.contains(&0)
    {
        return Err(Error::InvalidInput);
    }
    let ep = endpoint()?;
    let (ptr, token) = shm_create(SHM_BYTES, 0).map_err(|_| Error::Unavailable)?;
    unsafe {
        core::ptr::copy_nonoverlapping(current.as_ptr(), ptr, current.len());
        core::ptr::copy_nonoverlapping(new.as_ptr(), ptr.add(SECRET_LIMIT), new.len());
    }
    let msg = IpcMsg::with_label(CHANGE_OWN_PASSWORD)
        .word(0, session.session_id)
        .word(1, session.generation)
        .word(2, (current.len() as u64) | ((new.len() as u64) << 32))
        .with_cap(0, token);
    let result = ipc_call_timeout(ep, msg, TIMEOUT_MS)
        .map_err(|_| Error::Unavailable)
        .and_then(check_reply);
    unsafe {
        clear_secret(core::slice::from_raw_parts_mut(ptr, SECRET_LIMIT * 2));
    }
    let _ = shm_free(token);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_service_session_lookup_is_explicit_and_narrow() {
        use crate::{
            name_to_u64, service_capability_allows_hashed_name as allows, ServiceCapability as C,
        };
        assert!(!allows(
            C::Authentication.bit(),
            name_to_u64(SESSION_ENDPOINT)
        ));
        assert!(allows(
            C::SessionIdentity.bit(),
            name_to_u64(SESSION_ENDPOINT)
        ));
        for endpoint in ["uac", "spawn", "display_server", "vfs"] {
            assert!(!allows(C::SessionIdentity.bit(), name_to_u64(endpoint)));
        }
        assert_eq!(C::from_str("session-identity"), Some(C::SessionIdentity));
    }

    #[test]
    fn identity_is_resolved_by_session_uid_and_refresh() {
        let mut s = Snapshot::EMPTY;
        s.session_id = 20;
        s.user_count = 2;
        s.current_uid = 42;
        s.users[0].uid = 0;
        s.users[1].uid = 42;
        s.users[1].username[..3].copy_from_slice(b"ada");
        assert_eq!(s.current().unwrap().name(), "ada");
        s.users[1].display_name[..3].copy_from_slice(b"Ada");
        assert_eq!(s.current().unwrap().name(), "Ada");
        assert!(s.valid());
        s.current_uid = 43;
        assert!(!s.valid());
    }
    #[test]
    fn password_confirmation_rejects_empty_and_mismatch() {
        assert!(!password_matches(b"", b""));
        assert!(!password_matches(b"a", b"b"));
        assert!(password_matches(b"abc", b"abc"));
    }
    #[test]
    fn unavailable_account_transport_fails_closed() {
        assert_eq!(snapshot(), Err(Error::Unavailable));
        assert_eq!(active_session(), Err(Error::Unavailable));
        assert_eq!(
            change_own_password(&Snapshot::EMPTY, b"old", b"new"),
            Err(Error::Unavailable)
        );
        assert!(Operation::from_raw(999).is_none());
    }

    #[test]
    fn deterministic_avatar_and_secret_clear() {
        let mut u = User::EMPTY;
        u.uid = 42;
        assert_eq!(u.avatar_id(), u.avatar_id());
        let mut b = *b"secret";
        clear_secret(&mut b);
        assert_eq!(b, [0; 6]);
    }
}

pub const AUTHORIZE: u64 = 0xC202;
pub const MUTATE_GROUP: u64 = 0xC203;
#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    ChangeOwnPassword = 1,
    GroupCreate = 2,
    GroupAddMember = 3,
    GroupRemoveMember = 4,
    GroupDelete = 5,
    UpdateOwnProfile = 6,
}
impl Operation {
    pub fn from_raw(raw: u64) -> Option<Self> {
        match raw {
            1 => Some(Self::ChangeOwnPassword),
            2 => Some(Self::GroupCreate),
            3 => Some(Self::GroupAddMember),
            4 => Some(Self::GroupRemoveMember),
            5 => Some(Self::GroupDelete),
            6 => Some(Self::UpdateOwnProfile),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrantScope {
    pub revision: u64,
    pub operation: Operation,
    pub target: u32,
    pub member: u32,
    pub session: u64,
    pub session_generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrantBinding {
    pub pid: u64,
    pub process_generation: u64,
    pub scope: GrantScope,
}
/// Shared broker policy, exercised on the host and used by the kernel table.
#[derive(Clone, Copy, Debug)]
pub struct Grant {
    pub binding: GrantBinding,
    pub expires: u64,
    pub used: bool,
}
impl Grant {
    pub fn consume(&mut self, expected: GrantBinding, now: u64) -> bool {
        let allowed = !self.used && now < self.expires && self.binding == expected;
        self.used = true;
        allowed
    }
}
/// Only trusted UAC servicing this exact synchronous caller can mint/consume.
pub fn broker_grant(
    pid: u64,
    scope: GrantScope,
    consume: Option<CapabilityToken>,
) -> Option<CapabilityToken> {
    let call = if consume.is_some() {
        crate::SunlightSyscall::ConsumeAccountGrant
    } else {
        crate::SunlightSyscall::MintAccountGrant
    };
    let (ret, _) = unsafe {
        crate::raw_syscall(
            call,
            pid,
            scope.operation as u64,
            scope.target as u64 | ((scope.member as u64) << 32),
            scope.session,
            scope.session_generation,
            consume.map_or(0, |t| t.0),
            scope.revision,
        )
    };
    if ret == u64::MAX {
        None
    } else {
        Some(CapabilityToken(ret))
    }
}
#[cfg(test)]
mod grant_tests {
    use super::*;
    fn binding() -> GrantBinding {
        GrantBinding {
            pid: 20,
            process_generation: 3,
            scope: GrantScope {
                revision: 10,
                operation: Operation::GroupAddMember,
                target: 1001,
                member: 42,
                session: 8,
                session_generation: 2,
            },
        }
    }
    #[test]
    fn capability_is_one_use_expiring_and_exactly_bound() {
        let b = binding();
        let g = Grant {
            binding: b,
            expires: 100,
            used: false,
        };
        let mut valid = g;
        assert!(valid.consume(b, 99));
        assert!(!valid.consume(b, 99));
        let mut changed = g;
        assert!(!changed.consume(
            GrantBinding {
                scope: GrantScope {
                    revision: 11,
                    ..b.scope
                },
                ..b
            },
            10
        ));
        let mut expired = g;
        assert!(!expired.consume(b, 100));
        for bad in [
            GrantBinding { pid: 21, ..b },
            GrantBinding {
                process_generation: 4,
                ..b
            },
            GrantBinding {
                scope: GrantScope {
                    revision: 10,
                    operation: Operation::GroupDelete,
                    ..b.scope
                },
                ..b
            },
            GrantBinding {
                scope: GrantScope {
                    member: 43,
                    ..b.scope
                },
                ..b
            },
            GrantBinding {
                scope: GrantScope {
                    target: 1002,
                    ..b.scope
                },
                ..b
            },
            GrantBinding {
                scope: GrantScope {
                    session: 9,
                    ..b.scope
                },
                ..b
            },
            GrantBinding {
                scope: GrantScope {
                    session_generation: 3,
                    ..b.scope
                },
                ..b
            },
        ] {
            let mut grant = g;
            assert!(!grant.consume(bad, 10));
            assert!(!grant.consume(b, 11));
        }
    }
}

/// Typed operation delegated to Run-As. No arbitrary privileged command bytes.
#[derive(Clone, Copy)]
pub struct GroupRequest {
    pub scope: GrantScope,
    pub name: [u8; 32],
}
impl GroupRequest {
    fn message(self, label: u64) -> IpcMsg {
        IpcMsg::with_label(label)
            .word(0, self.scope.session)
            .word(1, self.scope.session_generation)
            .word(2, self.scope.operation as u64)
            .word(
                3,
                self.scope.target as u64 | ((self.scope.member as u64) << 32),
            )
    }
}
/// Run-As owns the credential entry; a successful reply contains authorization,
/// never a reusable password. Each grant covers exactly one group/operation/user.
pub fn authorize_group(request: GroupRequest, password: &[u8]) -> Result<CapabilityToken, Error> {
    if password.is_empty() || password.len() > SECRET_LIMIT {
        return Err(Error::InvalidInput);
    }
    let ep = endpoint()?;
    let (ptr, token) = shm_create(SHM_BYTES, 0).map_err(|_| Error::Unavailable)?;
    unsafe {
        core::ptr::copy_nonoverlapping(password.as_ptr(), ptr, password.len());
        core::ptr::write_unaligned(ptr.add(256).cast::<u64>(), request.scope.revision);
    }
    let result = ipc_call_timeout(
        ep,
        request.message(AUTHORIZE).with_cap(0, token),
        TIMEOUT_MS,
    )
    .map_err(|_| Error::Unavailable)
    .and_then(|r| {
        check_reply(r)?;
        if r.caps[0] == CapabilityToken::INVALID {
            Err(Error::Denied)
        } else {
            Ok(r.caps[0])
        }
    });
    unsafe {
        clear_secret(core::slice::from_raw_parts_mut(ptr, SECRET_LIMIT));
    }
    let _ = shm_free(token);
    result
}
pub fn mutate_group(request: GroupRequest, grant: CapabilityToken) -> Result<(), Error> {
    let ep = endpoint()?;
    let (ptr, token) = shm_create(SHM_BYTES, 0).map_err(|_| Error::Unavailable)?;
    unsafe {
        core::ptr::copy_nonoverlapping(request.name.as_ptr(), ptr, 32);
        core::ptr::write_unaligned(ptr.add(256).cast::<u64>(), request.scope.revision);
    }
    let result = ipc_call_timeout(
        ep,
        request
            .message(MUTATE_GROUP)
            .with_cap(0, grant)
            .with_cap(1, token),
        TIMEOUT_MS,
    )
    .map_err(|_| Error::Unavailable)
    .and_then(check_reply);
    let _ = shm_free(token);
    result
}

/// Public-record fingerprint for optimistic concurrency, never an auth token.
pub fn revision(passwd: &[u8], groups: &[u8]) -> u64 {
    passwd
        .iter()
        .chain(core::iter::once(&0xff))
        .chain(groups.iter())
        .fold(0xcbf29ce484222325, |h, b| {
            (h ^ (*b as u64)).wrapping_mul(0x100000001b3)
        })
}
pub const UPDATE_OWN_PROFILE: u64 = 0xC204;
pub fn update_own_profile(snapshot: &Snapshot, name: &str) -> Result<(), Error> {
    if name.is_empty()
        || name.len() > 47
        || name.chars().any(|c| c.is_control() || c == ':' || c == ',')
    {
        return Err(Error::InvalidInput);
    }
    let ep = endpoint()?;
    let (ptr, token) = shm_create(SHM_BYTES, 0).map_err(|_| Error::Unavailable)?;
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), ptr, name.len());
    }
    let result = ipc_call_timeout(
        ep,
        IpcMsg::with_label(UPDATE_OWN_PROFILE)
            .word(0, snapshot.session_id)
            .word(1, snapshot.generation)
            .word(2, snapshot.revision)
            .with_cap(0, token),
        TIMEOUT_MS,
    )
    .map_err(|_| Error::Unavailable)
    .and_then(check_reply);
    let _ = shm_free(token);
    result
}
