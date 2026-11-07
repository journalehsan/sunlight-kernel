//! Public read-only storage snapshots and version 1 presentation. No mount calls.
use core::fmt::{self, Write};
use sunlight_ipc::storage as api;

pub fn query(op: u64, index: u64, out: &mut [u8]) -> Result<bool, i64> {
    if !api::read_only(op)
        || out.len()
            < if op == api::INVENTORY {
                api::INVENTORY_RECORD_SIZE
            } else {
                api::MOUNT_RECORD_SIZE
            }
    {
        return Err(-22);
    }
    let result = unsafe {
        sunlight_libc::sys::syscall3(
            sunlight_ipc::SunlightSyscall::StorageCtl as u64,
            op,
            index,
            out.as_mut_ptr() as u64,
        )
    } as i64;
    if result < 0 {
        Err(result)
    } else {
        Ok(result != 0)
    }
}
pub fn word(bytes: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap())
}
pub fn guid(out: &mut impl Write, bytes: &[u8]) -> fmt::Result {
    for (i, j) in [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15]
        .iter()
        .enumerate()
    {
        if [4, 6, 8, 10].contains(&i) {
            out.write_char('-')?;
        }
        write!(out, "{:02x}", bytes[*j])?;
    }
    Ok(())
}
fn json_string(out: &mut impl Write, text: &str) -> fmt::Result {
    out.write_char('"')?;
    for c in text.chars() {
        match c {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            c if c < ' ' => write!(out, "\\u{:04x}", c as u32)?,
            c => out.write_char(c)?,
        }
    }
    out.write_char('"')
}
struct Capacity(Option<u64>);
impl fmt::Display for Capacity {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(n) => fmt::Display::fmt(&n, out),
            None => out.pad("?"),
        }
    }
}

pub fn render_device(
    out: &mut impl Write,
    record: &[u8; api::INVENTORY_RECORD_SIZE],
    mount: Option<(&str, bool)>,
    json: bool,
) -> fmt::Result {
    let id = word(record, 0);
    let parent = word(record, 1);
    let ops = word(record, 4);
    let scheme = match word(record, 6) {
        1 => Some("superfloppy"),
        2 => Some("mbr"),
        3 => Some("gpt"),
        _ => None,
    };
    let transport = if ops & 16 != 0 { "usb" } else { "virtio" };
    let uuid = &record[64..80];
    let kind = &record[80..96];
    let has_uuid = uuid.iter().any(|b| *b != 0);
    let trusted = mount.is_some_and(|(path, protected)| path == "/state" && protected);
    if json {
        write!(out, "{{\"runtime_id\":{id},\"parent_id\":")?;
        if parent == 0 {
            out.write_str("null")?;
        } else {
            write!(out, "{parent}")?;
        }
        out.write_str(",\"persistent_id\":")?;
        if has_uuid {
            out.write_str("\"PARTUUID=")?;
            guid(out, uuid)?;
            out.write_char('"')?;
        } else {
            out.write_str("null")?;
        }
        write!(
            out,
            ",\"transport\":\"{transport}\",\"removable\":null,\"capacity_bytes\":"
        )?;
        if let Some(size) = word(record, 3).checked_mul(word(record, 7)) {
            write!(out, "{size}")?;
        } else {
            out.write_str("null")?;
        }
        write!(
            out,
            ",\"logical_sector_size\":{},\"partition_scheme\":",
            word(record, 7)
        )?;
        if let Some(scheme) = scheme {
            json_string(out, scheme)?;
        } else {
            out.write_str("null")?;
        }
        out.write_str(",\"partition_ordinal\":")?;
        if parent == 0 {
            out.write_str("null")?;
        } else {
            write!(out, "{}", word(record, 5))?;
        }
        write!(
            out,
            ",\"start_lba\":{},\"blocks\":{},\"partition_type\":",
            word(record, 2),
            word(record, 3)
        )?;
        if kind.iter().any(|b| *b != 0) {
            out.write_char('"')?;
            guid(out, kind)?;
            out.write_char('"')?;
        } else {
            out.write_str("null")?;
        }
        write!(
            out,
            ",\"partition_name\":null,\"filesystem\":{},\"fat32_candidate\":{},\"mountpoint\":",
            if mount.is_some() { "\"fat32\"" } else { "null" },
            ops & 8 != 0
        )?;
        if let Some((path, _)) = mount {
            json_string(out, path)?;
        } else {
            out.write_str("null")?;
        }
        out.write_str(",\"mount_protected\":")?;
        if let Some((_, protected)) = mount {
            write!(out, "{protected}")?;
        } else {
            out.write_str("null")?;
        }
        // WRITE is a supported operation, not verified hardware write protection.
        write!(out,",\"write_supported\":{},\"read_only\":{},\"mount_read_only\":{},\"trusted_state\":{trusted}}}",ops & 2 != 0,if ops & 2 == 0 { "true" } else { "null" },if mount.is_some() { "false" } else { "null" })
    } else {
        write!(
            out,
            "{id:<4} {parent:<6} {transport:<7} {:<12} {:<6} {:<12} {:<8} {:<2} ?  {:<6} ",
            Capacity(word(record, 3).checked_mul(word(record, 7))),
            word(record, 7),
            scheme.unwrap_or("?"),
            if mount.is_some() { "fat32" } else { "?" },
            if ops & 2 == 0 { "yes" } else { "?" },
            if trusted { "yes" } else { "no" }
        )?;
        if has_uuid {
            out.write_str("PARTUUID=")?;
            guid(out, uuid)?;
        } else {
            out.write_char('?')?;
        }
        writeln!(out, " {}", mount.map_or("-", |m| m.0))
    }
}
pub fn inventory(out: &mut impl Write, json: bool) -> Result<(), i64> {
    if json {
        out.write_str("{\"schema_version\":1,\"devices\":[")
            .map_err(|_| -5)?;
    } else {
        out.write_str("ID   PARENT TRANSPORT BYTES        SECTOR SCHEME       FS       RO RM TRUST  PERSISTENT_ID MOUNTPOINT\n").map_err(|_|-5)?;
    }
    let mut record = [0; api::INVENTORY_RECORD_SIZE];
    let mut mounts = [0; api::MOUNT_RECORD_SIZE];
    for index in 0..api::MAX_DEVICES {
        if !query(api::INVENTORY, index as u64, &mut record)? {
            break;
        }
        let mut found = None;
        for j in 0..8 {
            if !query(api::INVENTORY_MOUNTS, j, &mut mounts)? {
                break;
            }
            if word(&mounts, 1) == word(&record, 0) {
                let len = word(&mounts, 3) as usize;
                if len >= api::TARGET_MAX {
                    return Err(-5);
                }
                found = Some((
                    core::str::from_utf8(&mounts[32..32 + len]).map_err(|_| -5)?,
                    word(&mounts, 2) != 0,
                ));
                break;
            }
        }
        if json && index != 0 {
            out.write_char(',').map_err(|_| -5)?;
        }
        render_device(out, &record, found, json).map_err(|_| -5)?;
    }
    if json {
        out.write_str("]}\n").map_err(|_| -5)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_query_wrapper_cannot_issue_mutations() {
        for op in [
            api::ENUMERATE,
            api::LIST_MOUNTS,
            api::MOUNT_FAT32,
            api::UNMOUNT,
            u64::MAX,
        ] {
            assert_eq!(query(op, 0, &mut [0; api::MOUNT_RECORD_SIZE]), Err(-22));
        }
    }
    #[test]
    fn unavailable_metadata_and_table() {
        let mut r = [0; api::INVENTORY_RECORD_SIZE];
        for (i, n) in [9u64, 0, 0, 100, 3, 0, 0, 512].iter().enumerate() {
            r[i * 8..i * 8 + 8].copy_from_slice(&n.to_le_bytes());
        }
        let mut out = std::string::String::new();
        render_device(&mut out, &r, None, true).unwrap();
        for field in [
            "persistent_id",
            "removable",
            "partition_scheme",
            "partition_ordinal",
            "partition_type",
            "partition_name",
            "filesystem",
            "mountpoint",
            "read_only",
            "mount_read_only",
            "mount_protected",
        ] {
            assert!(out.contains(&std::format!("\"{field}\":null")), "{out}");
        }
        assert_eq!(
            out,
            include_str!("../tests/data/inventory-v1-unavailable.json").trim_end()
        );
        assert!(out.contains("\"capacity_bytes\":51200"));
        assert!(out.contains("\"trusted_state\":false"));
        out.clear();
        render_device(&mut out, &r, None, false).unwrap();
        assert!(out.contains("virtio"));
        assert!(out.ends_with("? -\n"));
    }
    #[test]
    fn usb_read_only_unknown_capacity_and_type_metadata() {
        let mut r = [0; api::INVENTORY_RECORD_SIZE];
        for (i, n) in [1u64, 0, 0, u64::MAX, 17, 0, 3, 512].iter().enumerate() {
            r[i * 8..i * 8 + 8].copy_from_slice(&n.to_le_bytes());
        }
        r[80] = 1;
        let mut out = std::string::String::new();
        render_device(&mut out, &r, None, true).unwrap();
        for field in [
            "\"transport\":\"usb\"",
            "\"read_only\":true",
            "\"capacity_bytes\":null",
            "\"partition_type\":\"00000001-0000-0000-0000-000000000000\"",
        ] {
            assert!(out.contains(field), "{out}");
        }
        out.clear();
        render_device(&mut out, &r, None, false).unwrap();
        assert!(out.contains("usb"));
        assert!(!out.contains("18446744073709551615"));
    }
    #[test]
    fn guid_mount_trust_and_json_escaping() {
        let mut r = [0; api::INVENTORY_RECORD_SIZE];
        r[64..80].copy_from_slice(
            &crate::fstab::parse_partuuid("PARTUUID=12345678-1234-5678-9abc-123456789abc").unwrap(),
        );
        let mut out = std::string::String::new();
        render_device(&mut out, &r, Some(("/state", true)), true).unwrap();
        assert!(out.contains("PARTUUID=12345678-1234-5678-9abc-123456789abc"));
        assert!(out.contains("\"trusted_state\":true"));
        assert!(out.contains("\"mount_protected\":true"));
        out.clear();
        render_device(&mut out, &r, Some(("/mnt/\"x\\\n", false)), true).unwrap();
        assert!(out.contains("/mnt/\\\"x\\\\\\u000a"));
        assert!(out.contains("\"trusted_state\":false"));
    }
}
