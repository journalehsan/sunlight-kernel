//! The single boot mount configuration, shared by the legacy VFS and storagectl.
pub const MAX_FSTAB_ENTRIES: usize = 8;
pub const FSTAB_TEXT: &str = include_str!("../etc/fstab");
pub const MAX_FSTAB_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy)]
pub struct FstabEntry<'a> {
    pub device: &'a str,
    pub mountpoint: &'a str,
    pub fs_type: &'a str,
    pub options: &'a str,
}
pub type FstabTable<'a> = [Option<FstabEntry<'a>>; MAX_FSTAB_ENTRIES];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FstabError {
    pub line: usize,
    pub reason: &'static str,
}

/// Parse the canonical GPT GUID spelling into on-disk byte order.
pub fn parse_partuuid(source: &str) -> Option<[u8; 16]> {
    let s = source.strip_prefix("PARTUUID=")?.as_bytes();
    if s.len() != 36 || [8, 13, 18, 23].iter().any(|&i| s[i] != b'-') {
        return None;
    }
    let mut bytes = [0u8; 16];
    let mut digits = 0;
    for (i, &c) in s.iter().enumerate() {
        if [8, 13, 18, 23].contains(&i) {
            continue;
        }
        let n = (c as char).to_digit(16)? as u8;
        bytes[digits / 2] = (bytes[digits / 2] << 4) | n;
        digits += 1;
    }
    bytes[..4].reverse();
    bytes[4..6].reverse();
    bytes[6..8].reverse();
    (bytes != [0; 16]).then_some(bytes)
}

pub fn optional(entry: &FstabEntry<'_>) -> bool {
    entry.options.split(',').any(|s| s == "nofail")
}

pub fn parse_fstab(text: &str) -> Result<FstabTable<'_>, FstabError> {
    if text.len() > MAX_FSTAB_BYTES {
        return Err(FstabError {
            line: 1,
            reason: "configuration exceeds 4096 bytes",
        });
    }
    let mut table: FstabTable<'_> = [None; MAX_FSTAB_ENTRIES];
    let mut count = 0;
    for (index, line) in text.lines().enumerate() {
        let fail = |reason| FstabError {
            line: index + 1,
            reason,
        };
        let mut fields = line.split('#').next().unwrap_or("").split_whitespace();
        let Some(device) = fields.next() else {
            continue;
        };
        let mountpoint = fields.next().ok_or(fail("expected exactly four fields"))?;
        let fs_type = fields.next().ok_or(fail("expected exactly four fields"))?;
        let options = fields.next().ok_or(fail("expected exactly four fields"))?;
        if fields.next().is_some() {
            return Err(fail("expected exactly four fields"));
        }
        if count == MAX_FSTAB_ENTRIES {
            return Err(fail("too many entries"));
        }
        if !mountpoint.starts_with('/')
            || mountpoint.len() >= 128
            || (mountpoint != "/"
                && mountpoint[1..].split('/').any(|s| {
                    s.is_empty()
                        || s == "."
                        || s == ".."
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                }))
        {
            return Err(fail("invalid absolute target"));
        }
        match fs_type {
            "ramfs" | "tmpfs"
                if device == "/dev/ram0" && mountpoint == "/" && options == "defaults" => {}
            "bootfs" if device == "/dev/sda1" && mountpoint == "/boot" && options == "defaults" => {
            }
            "fat32" => {
                if parse_partuuid(device).is_none() {
                    return Err(fail("fat32 source requires a nonzero GPT PARTUUID"));
                }
                if mountpoint != "/mnt" && !mountpoint.starts_with("/mnt/") {
                    return Err(fail(
                        "ordinary FAT mounts are restricted to /mnt; no /state trust",
                    ));
                }
                let mut mode = false;
                let mut nofail = false;
                for option in options.split(',') {
                    match option {
                        "defaults" | "rw" if !mode => mode = true,
                        "nofail" if !nofail => nofail = true,
                        _ => {
                            return Err(fail(
                                "unsupported or duplicate option (only defaults/rw,nofail)",
                            ))
                        }
                    }
                }
                if !mode {
                    return Err(fail("explicit defaults or rw required"));
                }
            }
            _ => return Err(fail("unsupported filesystem or legacy entry")),
        }
        for old in table.iter().flatten() {
            if old.mountpoint == mountpoint {
                return Err(fail("duplicate target"));
            }
            if old.device == device
                || (fs_type == "fat32" && parse_partuuid(old.device) == parse_partuuid(device))
            {
                return Err(fail("duplicate source"));
            }
        }
        table[count] = Some(FstabEntry {
            device,
            mountpoint,
            fs_type,
            options,
        });
        count += 1;
    }
    Ok(table)
}

/// A configured GPT source belongs to storagectl. On invalid configuration,
/// fail closed for GPT boot automount rather than silently applying boot defaults
/// to an entry whose options could not be validated. Legacy raw media is unchanged.
pub fn defer_uuid_to_runtime(text: &str, uuid: &[u8; 16]) -> bool {
    if *uuid == [0; 16] {
        return false;
    }
    match parse_fstab(text) {
        Ok(entries) => entries
            .iter()
            .flatten()
            .any(|e| parse_partuuid(e.device).as_ref() == Some(uuid)),
        Err(_) => true,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    Missing,
    Ambiguous,
}
/// Resolve across the complete registry. Cloned GUIDs must fail closed.
pub fn resolve_uuid(
    uuid: [u8; 16],
    devices: impl IntoIterator<Item = (u64, [u8; 16])>,
) -> Result<u64, ResolveError> {
    let mut found = None;
    for (id, candidate) in devices {
        if uuid == candidate && uuid != [0; 16] {
            if found.is_some() {
                return Err(ResolveError::Ambiguous);
            }
            found = Some(id);
        }
    }
    found.ok_or(ResolveError::Missing)
}

/// nofail tolerates absence only; ambiguous identities always fail.
pub fn resolve_entry(
    entry: &FstabEntry<'_>,
    devices: impl IntoIterator<Item = (u64, [u8; 16])>,
) -> Result<Option<u64>, ResolveError> {
    let uuid = parse_partuuid(entry.device).ok_or(ResolveError::Missing)?;
    match resolve_uuid(uuid, devices) {
        Ok(id) => Ok(Some(id)),
        Err(ResolveError::Missing) if optional(entry) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "PARTUUID=12345678-1234-5678-9abc-123456789abc";
    #[test]
    fn legacy_and_comments() {
        let table = parse_fstab(FSTAB_TEXT).unwrap();
        assert_eq!(table[0].unwrap().fs_type, "bootfs");
        assert_eq!(table[1].unwrap().mountpoint, "/");
        assert_eq!(table[1].unwrap().fs_type, "tmpfs");
        assert!(parse_fstab("/dev/ram0 / ramfs defaults").is_ok());
        assert!(parse_fstab(" # blank\n").unwrap()[0].is_none());
    }
    #[test]
    fn strict_fields_options_and_state_boundary() {
        assert!(parse_fstab(&"#".repeat(MAX_FSTAB_BYTES + 1)).is_err());
        for line in [
            "a",
            "/dev/ram0 / ramfs defaults 0 0",
            "1 /mnt fat32 rw",
            "LABEL=SUNSTATE /state fat32 rw",
            "PARTUUID=00000000-0000-0000-0000-000000000000 /mnt fat32 rw",
        ] {
            assert!(parse_fstab(line).is_err(), "{line}");
        }
        for option in [
            "ro",
            "rw,ro",
            "rw,exec",
            "rw,rw",
            "defaults,rw",
            "nofail",
            "rw,",
            "rw,nofail,nofail",
        ] {
            assert!(parse_fstab(&std::format!("{ID} /mnt fat32 {option}")).is_err());
        }
        for target in [
            "/state",
            "/state/users",
            "/boot",
            "/",
            "/mnt/../state",
            "/mnt//x",
        ] {
            assert!(parse_fstab(&std::format!("{ID} {target} fat32 rw,nofail")).is_err());
        }
    }
    #[test]
    fn duplicate_sources_and_targets_and_capacity() {
        for line in [
            std::format!("{ID} /mnt/a fat32 rw\n{ID} /mnt/b fat32 rw"),
            std::format!(
                "{ID} /mnt fat32 rw\nPARTUUID=22345678-1234-5678-9abc-123456789abc /mnt fat32 rw"
            ),
            std::format!(
                "{ID} /mnt/a fat32 rw\n{} /mnt/b fat32 rw",
                ID.to_ascii_uppercase()
            ),
        ] {
            assert!(parse_fstab(&line).is_err());
        }
        let text = (1..=9)
            .map(|i| {
                std::format!("PARTUUID={i:08x}-1234-5678-9abc-123456789abc /mnt/{i} fat32 rw\n")
            })
            .collect::<std::string::String>();
        assert_eq!(parse_fstab(&text).unwrap_err().reason, "too many entries");
    }
    #[test]
    fn stable_resolution_and_missing_optional_policy() {
        let uuid = parse_partuuid(ID).unwrap();
        assert_eq!(&uuid[..4], &[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(resolve_uuid(uuid, [(99, uuid)]), Ok(99));
        assert_eq!(resolve_uuid(uuid, [(2, uuid)]), Ok(2));
        assert_eq!(resolve_uuid(uuid, []), Err(ResolveError::Missing));
        assert_eq!(
            resolve_uuid(uuid, [(1, uuid), (2, uuid)]),
            Err(ResolveError::Ambiguous)
        );
        for (options, expected) in [("rw", false), ("rw,nofail", true)] {
            let text = std::format!("{ID} /mnt fat32 {options}");
            let entry = parse_fstab(&text).unwrap()[0].unwrap();
            assert_eq!(optional(&entry), expected);
            assert_eq!(
                resolve_entry(&entry, []),
                if expected {
                    Ok(None)
                } else {
                    Err(ResolveError::Missing)
                }
            );
            assert_eq!(resolve_entry(&entry, [(88, uuid)]), Ok(Some(88)));
            assert_eq!(
                resolve_entry(&entry, [(1, uuid), (2, uuid)]),
                Err(ResolveError::Ambiguous)
            );
            assert!(defer_uuid_to_runtime(&text, &uuid));
            assert!(defer_uuid_to_runtime("malformed", &uuid));
            assert!(!defer_uuid_to_runtime(FSTAB_TEXT, &[0; 16]));
        }
    }
}
