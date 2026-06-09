//! No-heap parsers for /etc/passwd, /etc/group, and /etc/shadow.
//! All functions operate on fixed-size stack arrays only.

#[derive(Clone, Copy)]
pub struct PasswdEntry {
    pub username: [u8; 64],
    pub uid: u32,
    pub gid: u32,
    pub home: [u8; 128],
    pub shell: [u8; 128],
}

impl PasswdEntry {
    pub const fn zeroed() -> Self {
        Self {
            username: [0; 64],
            uid: 0,
            gid: 0,
            home: [0; 128],
            shell: [0; 128],
        }
    }
}

#[derive(Clone, Copy)]
pub struct GroupEntry {
    pub groupname: [u8; 64],
    pub gid: u32,
}

impl GroupEntry {
    pub const fn zeroed() -> Self {
        Self { groupname: [0; 64], gid: 0 }
    }
}

#[derive(Clone, Copy)]
pub struct ShadowEntry {
    pub username: [u8; 64],
    pub password: [u8; 128],
}

impl ShadowEntry {
    pub const fn zeroed() -> Self {
        Self { username: [0; 64], password: [0; 128] }
    }
}

/// Parse `/etc/passwd` from a byte slice; returns up to 16 entries.
pub fn parse_passwd(data: &[u8]) -> ([PasswdEntry; 16], usize) {
    let mut entries = [PasswdEntry::zeroed(); 16];
    let mut count = 0;
    let mut pos = 0;

    while pos < data.len() && count < 16 {
        let line_end = next_newline(data, pos);
        let line = &data[pos..line_end];
        pos = if line_end < data.len() { line_end + 1 } else { data.len() };

        if line.is_empty() || line[0] == b'#' || line[0] == 0 {
            continue;
        }

        // Fields: username:x:uid:gid:comment:home:shell
        let mut fields = [0usize; 8];
        let mut fcount = 0;
        let mut i = 0;
        while i <= line.len() && fcount < 8 {
            if i == line.len() || line[i] == b':' {
                fields[fcount] = i;
                fcount += 1;
            }
            i += 1;
        }
        if fcount < 7 {
            continue;
        }

        let f: [&[u8]; 7] = [
            field_slice(line, 0,           fields[0]),
            field_slice(line, fields[0]+1, fields[1]),
            field_slice(line, fields[1]+1, fields[2]),
            field_slice(line, fields[2]+1, fields[3]),
            field_slice(line, fields[3]+1, fields[4]),
            field_slice(line, fields[4]+1, fields[5]),
            field_slice(line, fields[5]+1, fields[6]),
        ];

        let uid = match parse_u32(f[2]) { Some(v) => v, None => continue };
        let gid = match parse_u32(f[3]) { Some(v) => v, None => continue };

        let mut entry = PasswdEntry::zeroed();
        copy_bytes(&mut entry.username, f[0]);
        entry.uid = uid;
        entry.gid = gid;
        copy_bytes(&mut entry.home, f[5]);
        copy_bytes(&mut entry.shell, f[6]);
        entries[count] = entry;
        count += 1;
    }

    (entries, count)
}

/// Parse `/etc/group` from a byte slice; returns up to 32 entries.
pub fn parse_group(data: &[u8]) -> ([GroupEntry; 32], usize) {
    let mut entries = [GroupEntry::zeroed(); 32];
    let mut count = 0;
    let mut pos = 0;

    while pos < data.len() && count < 32 {
        let line_end = next_newline(data, pos);
        let line = &data[pos..line_end];
        pos = if line_end < data.len() { line_end + 1 } else { data.len() };

        if line.is_empty() || line[0] == b'#' || line[0] == 0 {
            continue;
        }

        // Fields: groupname:x:gid:members
        let mut fields = [0usize; 4];
        let mut fcount = 0;
        let mut i = 0;
        while i <= line.len() && fcount < 4 {
            if i == line.len() || line[i] == b':' {
                fields[fcount] = i;
                fcount += 1;
            }
            i += 1;
        }
        if fcount < 3 {
            continue;
        }

        let f0 = field_slice(line, 0,           fields[0]);
        let f2 = field_slice(line, fields[1]+1, fields[2]);

        let gid = match parse_u32(f2) { Some(v) => v, None => continue };

        let mut entry = GroupEntry::zeroed();
        copy_bytes(&mut entry.groupname, f0);
        entry.gid = gid;
        entries[count] = entry;
        count += 1;
    }

    (entries, count)
}

/// Parse `/etc/shadow` from a byte slice; returns up to 16 entries.
/// Only the username and password fields are stored.
pub fn parse_shadow(data: &[u8]) -> ([ShadowEntry; 16], usize) {
    let mut entries = [ShadowEntry::zeroed(); 16];
    let mut count = 0;
    let mut pos = 0;

    while pos < data.len() && count < 16 {
        let line_end = next_newline(data, pos);
        let line = &data[pos..line_end];
        pos = if line_end < data.len() { line_end + 1 } else { data.len() };

        if line.is_empty() || line[0] == b'#' || line[0] == 0 {
            continue;
        }

        // Fields: username:password:...
        let colon1 = match line.iter().position(|&b| b == b':') {
            Some(p) => p,
            None => continue,
        };
        let colon2 = match line[colon1+1..].iter().position(|&b| b == b':') {
            Some(p) => colon1 + 1 + p,
            None => line.len(),
        };

        let username = &line[..colon1];
        let password = &line[colon1+1..colon2];

        let mut entry = ShadowEntry::zeroed();
        copy_bytes(&mut entry.username, username);
        copy_bytes(&mut entry.password, password);
        entries[count] = entry;
        count += 1;
    }

    (entries, count)
}

/// Find a passwd entry by username (null-terminated match).
pub fn lookup_by_name<'a>(
    entries: &'a [PasswdEntry],
    name: &[u8],
) -> Option<&'a PasswdEntry> {
    for entry in entries {
        let elen = entry.username.iter().position(|&b| b == 0).unwrap_or(64);
        if elen == name.len() && &entry.username[..elen] == name {
            return Some(entry);
        }
    }
    None
}

/// Find a passwd entry by uid.
pub fn lookup_by_uid(entries: &[PasswdEntry], uid: u32) -> Option<&PasswdEntry> {
    entries.iter().find(|e| e.uid == uid)
}

// --- private helpers ---

fn next_newline(data: &[u8], from: usize) -> usize {
    match data[from..].iter().position(|&b| b == b'\n') {
        Some(p) => from + p,
        None => data.len(),
    }
}

fn field_slice(line: &[u8], start: usize, end: usize) -> &[u8] {
    if start > line.len() {
        return &[];
    }
    &line[start..end.min(line.len())]
}

fn parse_u32(data: &[u8]) -> Option<u32> {
    if data.is_empty() {
        return None;
    }
    let mut result: u32 = 0;
    for &b in data {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}

fn copy_bytes(dst: &mut [u8], src: &[u8]) {
    let len = src.len().min(dst.len().saturating_sub(1));
    dst[..len].copy_from_slice(&src[..len]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD_DATA: &[u8] = b"\
root:x:0:0:root:/root:/bin/sh\n\
user:x:1000:1000:Regular User:/home/user:/bin/sh\n";

    const GROUP_DATA: &[u8] = b"\
root:x:0:root\n\
wheel:x:10:root\n\
users:x:100:user\n";

    const SHADOW_DATA: &[u8] = b"\
root:root:0:0:99999:7:::\n\
user:user:0:0:99999:7:::\n";

    #[test]
    fn parse_passwd_two_users() {
        let (entries, count) = parse_passwd(PASSWD_DATA);
        assert_eq!(count, 2);

        let root = lookup_by_name(&entries[..count], b"root").unwrap();
        assert_eq!(root.uid, 0);
        assert_eq!(root.gid, 0);
        assert_eq!(&root.home[..5], b"/root");
        assert_eq!(&root.shell[..7], b"/bin/sh");

        let user = lookup_by_name(&entries[..count], b"user").unwrap();
        assert_eq!(user.uid, 1000);
        assert_eq!(user.gid, 1000);
    }

    #[test]
    fn lookup_by_uid_finds_root() {
        let (entries, count) = parse_passwd(PASSWD_DATA);
        let root = lookup_by_uid(&entries[..count], 0).unwrap();
        assert_eq!(&root.username[..4], b"root");
    }

    #[test]
    fn parse_group_three_entries() {
        let (entries, count) = parse_group(GROUP_DATA);
        assert_eq!(count, 3);
        assert_eq!(entries[0].gid, 0);
        assert_eq!(entries[1].gid, 10);
        assert_eq!(entries[2].gid, 100);
    }

    #[test]
    fn parse_shadow_two_entries() {
        let (entries, count) = parse_shadow(SHADOW_DATA);
        assert_eq!(count, 2);
        assert_eq!(&entries[0].username[..4], b"root");
        assert_eq!(&entries[0].password[..4], b"root");
        assert_eq!(&entries[1].username[..4], b"user");
        assert_eq!(&entries[1].password[..4], b"user");
    }

    #[test]
    fn lookup_missing_returns_none() {
        let (entries, count) = parse_passwd(PASSWD_DATA);
        assert!(lookup_by_name(&entries[..count], b"nobody").is_none());
        assert!(lookup_by_uid(&entries[..count], 999).is_none());
    }

    #[test]
    fn skips_comment_and_empty_lines() {
        let data = b"# comment\n\nroot:x:0:0:root:/root:/bin/sh\n";
        let (entries, count) = parse_passwd(data);
        assert_eq!(count, 1);
        assert_eq!(entries[0].uid, 0);
    }
}
