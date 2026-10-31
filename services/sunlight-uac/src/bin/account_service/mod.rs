//! UAC-owned account IPC handlers; included only by the trusted daemon.
use sunlight_ipc::{accounts as api, IpcMsg};
use sunlight_uac::accounts::{authorize_own_change, parse_snapshot, replacement_shadow};
use zeroize::Zeroize;
#[cfg(any(feature = "session-account-exec-test", feature = "session-account-cancel-test"))]
pub(super) static EXEC_ENDPOINT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// No publication: all normal consumption policy/transport checks still run.
/// This label is dispatched only in an explicitly test-enabled UAC binary.
#[cfg(feature = "session-race-test")]
pub(super) fn consume_probe(msg: &IpcMsg) -> IpcMsg {
    let mut request = *msg;
    request.label = api::MUTATE_GROUP;
    match dispatch_group(&request, true) {
        Ok(None) => IpcMsg::with_label(api::REPLY),
        _ => IpcMsg::with_label(api::ERROR),
    }
}

#[cfg(feature = "session-race-test")]
static RACE_CONTROL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[cfg(feature = "session-race-test")]
fn race_checkpoint(scope: api::GrantScope, stage: u64) -> Result<(), api::Error> {
    let case = scope.target.checked_sub(61000).unwrap_or(0);
    let selected = matches!(
        (case, stage),
        (1 | 3 | 4 | 9 | 12..=15, 0) | (2 | 5 | 6, 1) | (7 | 10 | 11, 2) | (8, 3)
    );
    if !selected {
        return Ok(());
    }
    sunlight_ipc::debug_log(&alloc::format!(
        "[SESSION-E3-UAC] checkpoint=policy-rendezvous case={} stage={} pid={} session={} epoch={}",
        case,
        stage,
        sunlight_libc::getpid(),
        scope.session,
        scope.session_generation
    ));
    let control =
        sunlight_ipc::CapabilityToken(RACE_CONTROL.load(core::sync::atomic::Ordering::Relaxed));
    if control == sunlight_ipc::CapabilityToken::INVALID {
        return Err(api::Error::Unavailable);
    }
    let request = IpcMsg::with_label(0xE301)
        .word(0, u64::from(case))
        .word(1, stage)
        .word(2, scope.session)
        .word(3, scope.session_generation);
    #[cfg(feature = "session-reap-test")]
    let userspace = if (case, stage) == (6, 1) {
        let (ptr, cap) = sunlight_ipc::shm_alloc().map_err(|_| api::Error::Unavailable)?;
        // Page-aligned shared storage is initialized before delegating its cap.
        unsafe { &*ptr.cast::<core::sync::atomic::AtomicU64>() }
            .store(0, core::sync::atomic::Ordering::Relaxed);
        Some((ptr, cap))
    } else {
        None
    };
    #[cfg(feature = "session-reap-test")]
    let request = userspace.map_or(request, |(_, cap)| request.with_cap(0, cap));
    let reply = sunlight_ipc::ipc_call_timeout(control, request, 5000);
    #[cfg(feature = "session-reap-test")]
    if !reply.as_ref().is_ok_and(|r| r.label == 0xE301) {
        if let Some((_, cap)) = userspace {
            let _ = sunlight_ipc::shm_free(cap);
        }
    }
    let reply = reply.map_err(|_| api::Error::Unavailable)?;
    if reply.label != 0xE301 {
        return Err(api::Error::Unavailable);
    }
    #[cfg(feature = "session-reply-test")]
    if (case, stage) == (6, 1) {
        assert_eq!(reply.cap_count, 1);
        super::authenticated_reply::account(reply.caps[0]);
        let done = sunlight_ipc::ipc_call_timeout(control, IpcMsg::with_label(0xE312), 5000)
            .expect("authenticated reply peer completion");
        assert_eq!(done.label, 0xE301);
    }
    #[cfg(feature = "session-reap-test")]
    if let Some((ptr, cap)) = userspace {
        // This publication occurs in ring 3, after the rendezvous syscall has
        // returned. The controller observes it before submitting normal restart.
        // Make no further syscall until the bounded loop expires or aborts:
        // the real timer/fatal-signal path, rather than syscall return, must
        // terminate this participant. No kernel lock or interrupt state changes.
        let state = unsafe { &*ptr.cast::<core::sync::atomic::AtomicU64>() };
        state.store(1, core::sync::atomic::Ordering::Release);
        for _ in 0..100_000_000u64 {
            if state.load(core::sync::atomic::Ordering::Acquire) != 1 {
                break;
            }
            core::hint::spin_loop();
        }
        let _ = sunlight_ipc::shm_free(cap);
        return Err(api::Error::Unavailable);
    }
    Ok(())
}

pub(super) fn read(path: &[u8]) -> Result<alloc::vec::Vec<u8>, api::Error> {
    let fd = sunlight_libc::open(path).map_err(|_| api::Error::Storage)?;
    let mut bytes = alloc::vec::Vec::new();
    let result = (|| {
        let mut block = zeroize::Zeroizing::new([0u8; 512]);
        loop {
            let count = sunlight_libc::read(fd, &mut block[..]).map_err(|_| api::Error::Storage)?;
            if count == 0 {
                return Ok(());
            }
            if bytes.len() + count > 8192 {
                return Err(api::Error::Storage);
            }
            bytes.extend_from_slice(&block[..count]);
        }
    })();
    let _ = sunlight_libc::close(fd);
    if let Err(error) = result {
        bytes.zeroize();
        return Err(error);
    }
    Ok(bytes)
}
// E2 restores the broker protocol separately. Account database publication
// remains unavailable until real session/login and mutation lifecycle evidence
// covers these handlers. Boot-only development-shadow maintenance is separate.
fn require_verified_mutation_publication() -> Result<(), api::Error> {
    Err(api::Error::SessionAuthorityUnavailable)
}

pub(super) fn publish_shadow(data: &[u8]) -> Result<(), api::Error> {
    publish(b"/etc/.shadow-uac-new", b"/etc/shadow", data, 0o600)
}
fn publish(path: &[u8], destination: &[u8], data: &[u8], mode: u16) -> Result<(), api::Error> {
    // Exclusive root-only staging; rename publishes the complete replacement.
    // No fsync primitive exists: visibility is atomic, crash durability is not.

    let fd = sunlight_libc::open_with_flags_mode(
        path,
        sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT | sunlight_libc::O_EXCL,
        mode,
    )
    .map_err(|_| api::Error::Storage)?;
    let written = sunlight_libc::write_all(fd, data);
    let closed = sunlight_libc::close(fd);
    if written.is_err() || closed.is_err() {
        let _ = sunlight_libc::unlink(path);
        return Err(api::Error::Storage);
    }
    if sunlight_libc::rename(path, destination).is_err() {
        let _ = sunlight_libc::unlink(path);
        return Err(api::Error::Storage);
    }
    Ok(())
}
fn dispatch(msg: &IpcMsg) -> Result<(), api::Error> {
    let caller = sunlight_ipc::session_query_process(msg.badge).ok_or(api::Error::Denied)?;
    if msg.cap_count != 1 {
        return Err(api::Error::InvalidInput);
    }
    let active = api::active_session()?;
    let ptr = sunlight_ipc::shm_map(msg.caps[0]).map_err(|_| api::Error::InvalidInput)?;
    // Kernel SHM regions are page-rounded, including requests smaller than a
    // page. All accesses below fit within the first 4096 bytes.
    let result = (|| {
        if msg.label == api::SNAPSHOT {
            let mut snapshot = parse_snapshot(&read(b"/etc/passwd")?, &read(b"/etc/group")?)?;
            snapshot.session_id = active.0;
            snapshot.generation = active.1;
            snapshot.current_uid = active.2;
            snapshot.policy =
                u32::from(sunlight_uac::accounts::authorize_admin(caller.uid, active.2).is_ok());
            if !snapshot.valid() {
                return Err(api::Error::Storage);
            }
            unsafe {
                core::ptr::write_unaligned(ptr.cast::<api::Snapshot>(), snapshot);
            }
            return Ok(());
        }
        if msg.word_count != 3 {
            return Err(api::Error::InvalidInput);
        }
        authorize_own_change(
            caller.uid,
            active.2,
            (msg.words[0], msg.words[1]),
            (active.0, active.1),
        )?;
        let old_len = msg.words[2] as u32 as usize;
        let new_len = (msg.words[2] >> 32) as usize;
        if old_len == 0
            || new_len == 0
            || old_len > api::SECRET_LIMIT
            || new_len > api::SECRET_LIMIT
        {
            return Err(api::Error::InvalidInput);
        }
        let mut secrets = zeroize::Zeroizing::new([0u8; api::SECRET_LIMIT * 2]);
        unsafe {
            core::ptr::copy_nonoverlapping(ptr, secrets.as_mut_ptr(), secrets.len());
            api::clear_secret(core::slice::from_raw_parts_mut(ptr, secrets.len()));
        }
        let passwd = read(b"/etc/passwd")?;
        let shadow = zeroize::Zeroizing::new(read(b"/etc/shadow")?);
        let mut replacement = replacement_shadow(
            &passwd,
            &shadow,
            caller.uid,
            &secrets[..old_len],
            &secrets[api::SECRET_LIMIT..api::SECRET_LIMIT + new_len],
        )?;
        secrets.zeroize();
        let result = (|| {
            let now = api::active_session()?;
            authorize_own_change(caller.uid, now.2, (active.0, active.1), (now.0, now.1))?;
            // Timeouts/cancellation invalidate the kernel pending-call binding.
            let live = sunlight_ipc::session_query_process(msg.badge).ok_or(api::Error::Denied)?;
            if live != caller {
                return Err(api::Error::SessionChanged);
            }
            if read(b"/etc/shadow")?.as_slice() != shadow.as_slice() {
                return Err(api::Error::SessionChanged);
            }
            let scope = api::GrantScope {
                revision: api::revision(&passwd, &shadow),
                operation: api::Operation::ChangeOwnPassword,
                target: caller.uid,
                member: caller.uid,
                session: active.0,
                session_generation: active.1,
            };
            require_verified_mutation_publication()?;
            let grant = api::try_broker_grant(msg.badge, scope, None)?;
            api::try_broker_grant(msg.badge, scope, Some(grant))?;
            publish_shadow(replacement.as_bytes())
        })();
        replacement.zeroize();
        result
    })();
    if msg.label == api::CHANGE_OWN_PASSWORD {
        unsafe {
            api::clear_secret(core::slice::from_raw_parts_mut(ptr, api::SECRET_LIMIT * 2));
        }
    }
    let _ = sunlight_ipc::shm_free(msg.caps[0]);
    result
}
pub fn handle(msg: &IpcMsg) -> IpcMsg {
    if matches!(msg.label, api::AUTHORIZE | api::MUTATE_GROUP) {
        return match dispatch_group(msg, false) {
            Ok(Some(grant)) => IpcMsg::with_label(api::REPLY).with_cap(0, grant),
            Ok(None) => IpcMsg::with_label(api::REPLY),
            Err(e) => IpcMsg::with_label(api::ERROR).word(0, e as u64),
        };
    }
    let result = if msg.label == api::UPDATE_OWN_PROFILE {
        dispatch_profile(msg)
    } else {
        dispatch(msg)
    };
    match result {
        Ok(()) => IpcMsg::with_label(api::REPLY),
        Err(e) => IpcMsg::with_label(api::ERROR).word(0, e as u64),
    }
}

fn dispatch_group(
    msg: &IpcMsg,
    consume_only: bool,
) -> Result<Option<sunlight_ipc::CapabilityToken>, api::Error> {
    use sunlight_uac::accounts::{authorize_admin, group_policy, replacement_groups};
    if msg.word_count != 4 {
        return Err(api::Error::InvalidInput);
    }
    let caller = sunlight_ipc::session_query_process(msg.badge).ok_or(api::Error::Denied)?;
    let active = api::active_session()?;
    authorize_own_change(
        caller.uid,
        active.2,
        (msg.words[0], msg.words[1]),
        (active.0, active.1),
    )?;
    authorize_admin(caller.uid, active.2)?;
    let op = api::Operation::from_raw(msg.words[2]).ok_or(api::Error::InvalidInput)?;
    let mut scope = api::GrantScope {
        revision: 0,
        operation: op,
        target: msg.words[3] as u32,
        member: (msg.words[3] >> 32) as u32,
        session: active.0,
        session_generation: active.1,
    };
    let passwd = read(b"/etc/passwd")?;
    let groups = read(b"/etc/group")?;
    let snapshot = parse_snapshot(&passwd, &groups)?;
    group_policy(&snapshot, op, scope.target, scope.member)?;
    let index = if msg.label == api::AUTHORIZE { 0 } else { 1 };
    let expected_caps = index as u32 + 1;
    #[cfg(feature = "session-race-test")]
    let expected_caps = if msg.label == api::AUTHORIZE && (61001..=61015).contains(&scope.target) {
        2
    } else {
        expected_caps
    };
    if msg.cap_count != expected_caps {
        return Err(api::Error::InvalidInput);
    }
    let ptr = sunlight_ipc::shm_map(msg.caps[index]).map_err(|_| api::Error::InvalidInput)?;
    let result = (|| {
        scope.revision = unsafe { core::ptr::read_unaligned(ptr.add(256).cast::<u64>()) };
        if scope.revision != snapshot.revision {
            return Err(api::Error::ConcurrentChange);
        }
        if msg.label == api::AUTHORIZE {
            let mut password = zeroize::Zeroizing::new([0u8; api::SECRET_LIMIT]);
            unsafe {
                core::ptr::copy_nonoverlapping(ptr, password.as_mut_ptr(), password.len());
                api::clear_secret(core::slice::from_raw_parts_mut(ptr, password.len()));
            }
            let len = password
                .iter()
                .position(|b| *b == 0)
                .unwrap_or(password.len());
            let user = snapshot
                .users()
                .iter()
                .find(|u| u.uid == caller.uid)
                .ok_or(api::Error::Denied)?;
            let shadow = zeroize::Zeroizing::new(read(b"/etc/shadow")?);
            sunlight_uac::auth::verify_shadow_credentials(
                &passwd,
                &shadow,
                user.login().as_bytes(),
                &password[..len],
            )
            .map_err(|_| api::Error::IncorrectPassword)?;
            password.zeroize();
            if api::active_session()? != active {
                return Err(api::Error::SessionChanged);
            }
            #[cfg(feature = "session-race-test")]
            if expected_caps == 2 {
                RACE_CONTROL.store(msg.caps[1].0, core::sync::atomic::Ordering::Relaxed);
            }
            #[cfg(feature = "session-race-test")]
            race_checkpoint(scope, 0)?;
            let result = api::try_broker_grant(msg.badge, scope, None);
            #[cfg(feature = "session-race-test")]
            if let Ok(committed_grant) = result {
                let _ = committed_grant;
                #[cfg(feature = "session-account-cancel-test")]
                if scope.target == 61006 {
                    super::account_cancel::account(
                        sunlight_ipc::EndpointId(EXEC_ENDPOINT.load(core::sync::atomic::Ordering::Relaxed)),
                        committed_grant, scope, msg, ptr,
                    );
                    return Err(api::Error::Unavailable);
                }
                #[cfg(feature = "session-account-exec-test")]
                if scope.target == 61006 {
                    super::account_exec::account(
                        sunlight_ipc::EndpointId(
                            EXEC_ENDPOINT.load(core::sync::atomic::Ordering::Relaxed),
                        ),
                        sunlight_ipc::CapabilityToken(
                            RACE_CONTROL.load(core::sync::atomic::Ordering::Relaxed),
                        ),
                        committed_grant,
                        scope,
                        msg.badge,
                    );
                }
                race_checkpoint(scope, 1)?;
            }
            return result.map(Some);
        }
        // The kernel checks operation, target, membership, process generation,
        // session, expiry and one-use state. Never test merely for nonzero.
        #[cfg(feature = "session-race-test")]
        if consume_only {
            race_checkpoint(scope, 2)?;
            api::try_broker_grant(msg.badge, scope, Some(msg.caps[0]))?;
            let repeated = api::try_broker_grant(msg.badge, scope, Some(msg.caps[0]));
            // Concurrent logout can remove the session between the successful
            // consume and this repeat. Both production denials are valid; the
            // parser still requires the first consume's authoritative commit.
            if repeated != Err(api::Error::CapabilityRejected)
                && !(scope.target == 61010
                    && repeated == Err(api::Error::SessionAuthorityUnavailable))
            {
                return Err(api::Error::BrokerUnavailable);
            }
            race_checkpoint(scope, 3)?;
            return Ok(None);
        }
        #[cfg(not(feature = "session-race-test"))]
        let _ = consume_only;
        require_verified_mutation_publication()?;
        api::try_broker_grant(msg.badge, scope, Some(msg.caps[0]))?;
        let mut name = [0u8; 32];
        unsafe {
            core::ptr::copy_nonoverlapping(ptr, name.as_mut_ptr(), 32);
        }
        let replacement = replacement_groups(
            &passwd,
            &groups,
            op,
            scope.target,
            scope.member,
            api::text(&name),
        )?;
        if api::active_session()? != active {
            return Err(api::Error::SessionChanged);
        }
        if read(b"/etc/group")? != groups {
            return Err(api::Error::ConcurrentChange);
        }
        publish(
            b"/etc/.group-uac-new",
            b"/etc/group",
            replacement.as_bytes(),
            0o644,
        )?;
        Ok(None)
    })();
    let _ = sunlight_ipc::shm_free(msg.caps[index]);
    result
}

fn dispatch_profile(msg: &IpcMsg) -> Result<(), api::Error> {
    if msg.word_count != 3 || msg.cap_count != 1 {
        return Err(api::Error::InvalidInput);
    }
    let caller = sunlight_ipc::session_query_process(msg.badge).ok_or(api::Error::Denied)?;
    let active = api::active_session()?;
    authorize_own_change(
        caller.uid,
        active.2,
        (msg.words[0], msg.words[1]),
        (active.0, active.1),
    )?;
    let ptr = sunlight_ipc::shm_map(msg.caps[0]).map_err(|_| api::Error::InvalidInput)?;
    let mut name = [0u8; 48];
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, name.as_mut_ptr(), 48);
    }
    let _ = sunlight_ipc::shm_free(msg.caps[0]);
    let passwd = read(b"/etc/passwd")?;
    let groups = read(b"/etc/group")?;
    let snapshot = parse_snapshot(&passwd, &groups)?;
    if snapshot.revision != msg.words[2] {
        return Err(api::Error::ConcurrentChange);
    }
    let replacement =
        sunlight_uac::accounts::replacement_profile(&passwd, caller.uid, api::text(&name))?;
    if api::active_session()? != active {
        return Err(api::Error::SessionChanged);
    }
    let scope = api::GrantScope {
        revision: snapshot.revision,
        operation: api::Operation::UpdateOwnProfile,
        target: caller.uid,
        member: caller.uid,
        session: active.0,
        session_generation: active.1,
    };
    require_verified_mutation_publication()?;
    let grant = api::try_broker_grant(msg.badge, scope, None)?;
    api::try_broker_grant(msg.badge, scope, Some(grant))?;
    publish(
        b"/etc/.passwd-uac-new",
        b"/etc/passwd",
        replacement.as_bytes(),
        0o644,
    )
}
