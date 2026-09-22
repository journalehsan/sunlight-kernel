//! Account presentation parsing and password replacement policy. Never GUI code.
use crate::auth::{hash_password, verify_shadow_credentials};
use alloc::{format, string::String};
use sunlight_ipc::accounts::{Error, Group, Snapshot, User, MAX_GROUPS, MAX_USERS};

fn copy(out: &mut [u8], s: &str) -> Result<(), Error> {
    if s.len() >= out.len() || s.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidInput);
    }
    out[..s.len()].copy_from_slice(s.as_bytes());
    Ok(())
}
/// Reject oversized/malformed databases instead of presenting partial authority.
pub fn parse_snapshot(passwd: &[u8], groups: &[u8]) -> Result<Snapshot, Error> {
    let mut result = Snapshot::EMPTY;
    for line in core::str::from_utf8(passwd)
        .map_err(|_| Error::Storage)?
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let fields: alloc::vec::Vec<_> = line.split(':').collect();
        if fields.len() != 7 || result.user_count as usize == MAX_USERS {
            return Err(Error::Storage);
        }
        let mut user = User::EMPTY;
        user.uid = fields[2].parse().map_err(|_| Error::Storage)?;
        user.gid = fields[3].parse().map_err(|_| Error::Storage)?;
        copy(&mut user.username, fields[0])?;
        copy(
            &mut user.display_name,
            fields[4].split(',').next().unwrap_or(""),
        )?;
        if user.login().is_empty()
            || result
                .users()
                .iter()
                .any(|u| u.uid == user.uid || u.login() == user.login())
        {
            return Err(Error::Storage);
        }
        result.users[result.user_count as usize] = user;
        result.user_count += 1;
    }
    for line in core::str::from_utf8(groups)
        .map_err(|_| Error::Storage)?
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let fields: alloc::vec::Vec<_> = line.split(':').collect();
        if fields.len() != 4 || result.group_count as usize == MAX_GROUPS {
            return Err(Error::Storage);
        }
        let mut group = Group::EMPTY;
        group.gid = fields[2].parse().map_err(|_| Error::Storage)?;
        copy(&mut group.name, fields[0])?;
        if group.name().is_empty()
            || result
                .groups()
                .iter()
                .any(|g| g.gid == group.gid || g.name() == group.name())
        {
            return Err(Error::Storage);
        }
        for (index, user) in result.users().iter().enumerate() {
            if user.gid == group.gid || fields[3].split(',').any(|name| name == user.login()) {
                group.members |= 1 << index;
            }
        }
        // Do not silently drop unknown members from the displayed account view.
        if fields[3]
            .split(',')
            .any(|n| !n.is_empty() && !result.users().iter().any(|u| u.login() == n))
        {
            return Err(Error::Storage);
        }
        result.groups[result.group_count as usize] = group;
        result.group_count += 1;
    }
    result.revision = sunlight_ipc::accounts::revision(passwd, groups);
    Ok(result)
}

pub fn authorize_own_change(
    caller_uid: u32,
    session_uid: u32,
    expected: (u64, u64),
    active: (u64, u64),
) -> Result<(), Error> {
    if expected != active || active.0 == 0 {
        return Err(Error::SessionChanged);
    }
    if caller_uid != session_uid {
        return Err(Error::Denied);
    }
    Ok(())
}

/// Verifies the old password here, derives the target from kernel caller UID,
/// and preserves every other shadow field/account. No verification flag input.
pub fn replacement_shadow(
    passwd: &[u8],
    shadow: &[u8],
    uid: u32,
    old: &[u8],
    new: &[u8],
) -> Result<String, Error> {
    let (users, count) = sunlight_fs::parse_passwd(passwd);
    let user = users[..count]
        .iter()
        .find(|u| u.uid == uid)
        .ok_or(Error::Denied)?;
    let username = sunlight_ipc::accounts::text(&user.username);
    let verified = verify_shadow_credentials(passwd, shadow, username.as_bytes(), old)
        .map_err(|_| Error::IncorrectPassword)?;
    if verified.uid != uid {
        return Err(Error::Denied);
    }
    let hash = hash_password(new).map_err(|_| Error::InvalidInput)?;
    let mut out = String::new();
    let mut replaced = false;
    for line in core::str::from_utf8(shadow)
        .map_err(|_| Error::Storage)?
        .lines()
    {
        let mut fields = line.splitn(3, ':');
        let name = fields.next().unwrap_or("");
        if name == username {
            if replaced || fields.next().is_none() {
                return Err(Error::Storage);
            }
            let rest = fields.next();
            out.push_str(&format!("{name}:{hash}"));
            if let Some(rest) = rest {
                out.push(':');
                out.push_str(rest);
            }
            replaced = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !replaced {
        return Err(Error::Storage);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PASSWD: &[u8] = b"root:x:0:0:Root:/root:/bin/sh\nada:x:42:100:Ada:/home/ada:/bin/sh\n";
    #[test]
    fn real_group_members_include_primary_and_explicit_members() {
        let s = parse_snapshot(PASSWD, b"root:x:0:root\nusers:x:100:\nwheel:x:10:ada\n").unwrap();
        assert_eq!(s.users()[1].name(), "Ada");
        assert_eq!(s.groups()[1].members, 2);
        assert_eq!(s.groups()[2].members, 2);
        assert!(!s.users()[1].is_admin());
    }
    #[test]
    fn other_user_and_stale_session_denied() {
        assert_eq!(
            authorize_own_change(43, 42, (1, 1), (1, 1)),
            Err(Error::Denied)
        );
        assert_eq!(
            authorize_own_change(42, 42, (1, 1), (1, 2)),
            Err(Error::SessionChanged)
        );
        assert!(authorize_own_change(42, 42, (1, 1), (1, 1)).is_ok());
    }
    #[test]
    fn backend_verifies_old_password_and_preserves_other_accounts() {
        let hash = hash_password(b"old").unwrap();
        let shadow = format!("root:!:0:0:99999:7:::\nada:{hash}:0:0:99999:7:::\n");
        assert_eq!(
            replacement_shadow(PASSWD, shadow.as_bytes(), 42, b"wrong", b"new"),
            Err(Error::IncorrectPassword)
        );
        let changed = replacement_shadow(PASSWD, shadow.as_bytes(), 42, b"old", b"new").unwrap();
        assert!(changed.starts_with("root:!:0:0:99999:7:::\n"));
        assert!(verify_shadow_credentials(PASSWD, changed.as_bytes(), b"ada", b"new").is_ok());
        assert!(verify_shadow_credentials(PASSWD, changed.as_bytes(), b"ada", b"old").is_err());
    }
    #[test]
    fn malformed_and_unknown_group_members_fail_closed() {
        assert!(parse_snapshot(PASSWD, b"wheel:x:10:missing\n").is_err());
        assert!(parse_snapshot(b"broken", b"").is_err());
    }
}

/// Only UID 0 has administrative authority in the current policy. In
/// particular, a GUI claiming is_admin or a member of wheel grants nothing.
pub fn authorize_admin(caller_uid: u32, session_uid: u32) -> Result<(), Error> {
    if caller_uid == 0 && session_uid == 0 {
        Ok(())
    } else {
        Err(Error::Denied)
    }
}
pub fn group_policy(
    snapshot: &Snapshot,
    op: sunlight_ipc::accounts::Operation,
    gid: u32,
    member: u32,
) -> Result<(), Error> {
    use sunlight_ipc::accounts::Operation::*;
    if gid < 1000 || op == ChangeOwnPassword {
        return Err(Error::Denied);
    }
    let group = snapshot.groups().iter().find(|g| g.gid == gid);
    match op {
        GroupCreate
            if group.is_none()
                && !snapshot.users().iter().any(|u| u.gid == gid)
                && snapshot.group_count < MAX_GROUPS as u32 =>
        {
            Ok(())
        }
        GroupAddMember | GroupRemoveMember
            if group.is_some() && snapshot.users().iter().any(|u| u.uid == member) =>
        {
            if op == GroupRemoveMember
                && snapshot
                    .users()
                    .iter()
                    .any(|u| u.uid == member && u.gid == gid)
            {
                Err(Error::Denied)
            } else {
                Ok(())
            }
        }
        GroupDelete
            if group.is_some()
                && !snapshot.users().iter().any(|u| u.gid == gid)
                && group.unwrap().members == 0 =>
        {
            Ok(())
        }
        _ => Err(Error::Denied),
    }
}
pub fn replacement_groups(
    passwd: &[u8],
    groups: &[u8],
    op: sunlight_ipc::accounts::Operation,
    gid: u32,
    member: u32,
    name: &str,
) -> Result<String, Error> {
    use sunlight_ipc::accounts::Operation::*;
    let s = parse_snapshot(passwd, groups)?;
    group_policy(&s, op, gid, member)?;
    let mut result = String::new();
    if op == GroupCreate
        && (name.is_empty()
            || name.len() > 31
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
            || s.groups().iter().any(|g| g.name() == name))
    {
        return Err(Error::InvalidInput);
    }
    for line in core::str::from_utf8(groups)
        .map_err(|_| Error::Storage)?
        .lines()
    {
        let fields: alloc::vec::Vec<_> = line.split(':').collect();
        if fields.len() != 4 || fields[2].parse::<u32>().ok() != Some(gid) {
            result.push_str(line);
            result.push('\n');
            continue;
        }
        if op == GroupDelete {
            continue;
        }
        if matches!(op, GroupAddMember | GroupRemoveMember) {
            let username = s
                .users()
                .iter()
                .find(|u| u.uid == member)
                .ok_or(Error::Denied)?
                .login();
            let mut names: alloc::vec::Vec<_> = fields[3]
                .split(',')
                .filter(|n| !n.is_empty() && *n != username)
                .collect();
            if op == GroupAddMember {
                names.push(username);
            }
            result.push_str(&format!(
                "{}:{}:{}:{}\n",
                fields[0],
                fields[1],
                fields[2],
                names.join(",")
            ));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    if op == GroupCreate {
        result.push_str(&format!("{name}:x:{gid}:\n"));
    }
    Ok(result)
}
#[cfg(test)]
mod group_tests {
    use super::*;
    use sunlight_ipc::accounts::Operation::*;
    const P: &[u8] = b"root:x:0:0:Root:/root:/bin/sh\nada:x:42:100:Ada:/home/ada:/bin/sh\n";
    const G: &[u8] = b"root:x:0:root\nusers:x:100:\nproject:x:1001:\n";
    #[test]
    fn membership_and_group_lifecycle() {
        let add = replacement_groups(P, G, GroupAddMember, 1001, 42, "").unwrap();
        assert!(add.contains("project:x:1001:ada"));
        assert!(replacement_groups(P, add.as_bytes(), GroupDelete, 1001, 0, "").is_err());
        let remove =
            replacement_groups(P, add.as_bytes(), GroupRemoveMember, 1001, 42, "").unwrap();
        assert_eq!(remove.as_bytes(), G);
        let deleted = replacement_groups(P, remove.as_bytes(), GroupDelete, 1001, 0, "").unwrap();
        assert!(!deleted.contains("project"));
        assert!(
            replacement_groups(P, deleted.as_bytes(), GroupCreate, 1002, 0, "team")
                .unwrap()
                .contains("team:x:1002:")
        );
    }
    #[test]
    fn reserved_groups_and_non_admin_denied() {
        for op in [GroupDelete, GroupAddMember, GroupRemoveMember, GroupCreate] {
            assert!(replacement_groups(P, G, op, 0, 42, "root").is_err());
        }
        assert_eq!(authorize_admin(42, 42), Err(Error::Denied));
        assert_eq!(authorize_admin(0, 42), Err(Error::Denied));
        assert!(authorize_admin(0, 0).is_ok());
    }
}

pub fn replacement_profile(passwd: &[u8], uid: u32, name: &str) -> Result<String, Error> {
    if name.is_empty()
        || name.len() > 47
        || name.chars().any(|c| c.is_control() || c == ':' || c == ',')
    {
        return Err(Error::InvalidInput);
    }
    let mut result = String::new();
    let mut found = false;
    for line in core::str::from_utf8(passwd)
        .map_err(|_| Error::Storage)?
        .lines()
    {
        let fields: alloc::vec::Vec<_> = line.split(':').collect();
        if fields.len() == 7 && fields[2].parse::<u32>().ok() == Some(uid) {
            if found {
                return Err(Error::Storage);
            }
            found = true;
            result.push_str(&format!(
                "{}:{}:{}:{}:{}",
                fields[0], fields[1], fields[2], fields[3], name
            ));
            if let Some((_, rest)) = fields[4].split_once(',') {
                result.push(',');
                result.push_str(rest);
            }
            result.push_str(&format!(":{}:{}", fields[5], fields[6]));
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    if !found {
        return Err(Error::Denied);
    }
    Ok(result)
}
#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn profile_preserves_identity_and_gecos_suffix_and_refreshes() {
        let p = b"ada:x:42:100:Ada,office:/home/ada:/bin/sh\nroot:x:0:0:Root:/root:/bin/sh\n";
        let updated = replacement_profile(p, 42, "Ada Lovelace").unwrap();
        assert!(updated.contains("Ada Lovelace,office"));
        assert!(updated.contains("root:x:0:0:Root"));
        let s = parse_snapshot(updated.as_bytes(), b"users:x:100:\n").unwrap();
        assert_eq!(s.users()[0].name(), "Ada Lovelace");
        assert_ne!(
            s.revision,
            parse_snapshot(p, b"users:x:100:\n").unwrap().revision
        );
        assert!(replacement_profile(p, 43, "Other").is_err());
        assert!(replacement_profile(p, 42, "Ada:root").is_err());
    }
}
