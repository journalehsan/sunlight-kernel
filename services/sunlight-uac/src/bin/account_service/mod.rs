//! UAC-owned account IPC handlers; included only by the trusted daemon.
use sunlight_ipc::{accounts as api, IpcMsg};
use sunlight_uac::accounts::{authorize_own_change, parse_snapshot, replacement_shadow};
use zeroize::Zeroize;

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
            let grant =
                api::broker_grant(msg.badge, scope, None).ok_or(api::Error::BrokerUnavailable)?;
            api::broker_grant(msg.badge, scope, Some(grant)).ok_or(api::Error::Denied)?;
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
        return match dispatch_group(msg) {
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

fn dispatch_group(msg: &IpcMsg) -> Result<Option<sunlight_ipc::CapabilityToken>, api::Error> {
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
    if msg.cap_count != index as u32 + 1 {
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
            return api::broker_grant(msg.badge, scope, None)
                .map(Some)
                .ok_or(api::Error::BrokerUnavailable);
        }
        // The kernel checks operation, target, membership, process generation,
        // session, expiry and one-use state. Never test merely for nonzero.
        api::broker_grant(msg.badge, scope, Some(msg.caps[0]))
            .ok_or(api::Error::CapabilityRejected)?;
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
    let grant = api::broker_grant(msg.badge, scope, None).ok_or(api::Error::BrokerUnavailable)?;
    api::broker_grant(msg.badge, scope, Some(grant)).ok_or(api::Error::CapabilityRejected)?;
    publish(
        b"/etc/.passwd-uac-new",
        b"/etc/passwd",
        replacement.as_bytes(),
        0o644,
    )
}
