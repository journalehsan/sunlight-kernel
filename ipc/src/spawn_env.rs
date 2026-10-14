//! Optional native spawn environment transport. This carries data, never authority.
//!
//! With ENV_EXTENSION in r9, rsi points to SpawnEnvironment instead of argv.
//! All other registers retain the stdio-extension meaning. Legacy callers keep
//! inheritance. The blob replaces the environment, including when it is empty.

pub const STDIO_EXTENSION: u64 = 0x5355_4E53_5444_494F;
pub const ENV_EXTENSION: u64 = 0x5355_4E45_4E56_3031;
pub const MAX_ENTRIES: usize = 128;
pub const MAX_BYTES: usize = 16 * 1024;
/// Includes the terminating NUL.
pub const MAX_ENTRY_BYTES: usize = 4096;
pub const MAX_NAME_BYTES: usize = 255;
pub const HEADER_BYTES: usize = 24;

#[repr(C)]
pub struct SpawnEnvironment {
    pub argv: u64,
    pub bytes: u64,
    pub len: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentError {
    Invalid,
    TooLarge,
    Duplicate,
}

pub fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn entry_size(name: &str, value: &str) -> Result<usize, EnvironmentError> {
    if !valid_name(name) || value.as_bytes().contains(&0) {
        return Err(EnvironmentError::Invalid);
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(EnvironmentError::TooLarge);
    }
    name.len()
        .checked_add(value.len())
        .and_then(|n| n.checked_add(2))
        .filter(|n| *n <= MAX_ENTRY_BYTES)
        .ok_or(EnvironmentError::TooLarge)
}

/// Fully validate before constructing a PCB or scheduling a child. Duplicate
/// names are rejected rather than relying on conflicting getenv conventions.
pub fn validate(blob: &[u8]) -> Result<(), EnvironmentError> {
    if blob.len() > MAX_BYTES {
        return Err(EnvironmentError::TooLarge);
    }
    let mut names = [""; MAX_ENTRIES];
    let mut count = 0;
    let mut rest = blob;
    while !rest.is_empty() {
        if count == MAX_ENTRIES {
            return Err(EnvironmentError::TooLarge);
        }
        let end = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or(EnvironmentError::Invalid)?;
        let entry = core::str::from_utf8(&rest[..end]).map_err(|_| EnvironmentError::Invalid)?;
        let (key, value) = entry.split_once('=').ok_or(EnvironmentError::Invalid)?;
        entry_size(key, value)?;
        if names[..count].contains(&key) {
            return Err(EnvironmentError::Duplicate);
        }
        names[count] = key;
        count += 1;
        rest = &rest[end + 1..];
    }
    Ok(())
}

/// Only use after validate() succeeds.
pub fn entries(blob: &[u8]) -> impl Iterator<Item = (&str, &str)> {
    blob.split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| core::str::from_utf8(s).ok()?.split_once('='))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::{format, vec, vec::Vec};

    #[test]
    fn accepts_empty_values_equals_newlines_and_unicode_values() {
        assert_eq!(validate(b""), Ok(()));
        let blob = "EMPTY=\0A=x=y\n\0UNICODE=سلام\0".as_bytes();
        assert_eq!(validate(blob), Ok(()));
        assert_eq!(entries(blob).count(), 3);
        assert_eq!(entries(blob).nth(1), Some(("A", "x=y\n")));
    }
    #[test]
    fn rejects_malformed_and_duplicate_entries() {
        for blob in [
            b"A=missing-nul".as_slice(),
            b"\0",
            b"novalue\0",
            b"=x\0",
            b"1A=x\0",
            b"A-B=x\0",
            b"A=\xff\0",
            b"A=x\0A=y\0",
        ] {
            assert!(validate(blob).is_err(), "{blob:?}");
        }
        assert_eq!(entry_size("A", "x\0y"), Err(EnvironmentError::Invalid));
        assert_eq!(
            entry_size(&"A".repeat(MAX_NAME_BYTES), ""),
            Ok(MAX_NAME_BYTES + 2)
        );
        assert_eq!(
            entry_size(&"A".repeat(MAX_NAME_BYTES + 1), ""),
            Err(EnvironmentError::TooLarge)
        );
    }
    #[test]
    fn enforces_entry_count_and_byte_boundaries() {
        let mut blob = Vec::new();
        for i in 0..MAX_ENTRIES {
            blob.extend_from_slice(format!("V{i}=\0").as_bytes());
        }
        assert_eq!(validate(&blob), Ok(()));
        blob.extend_from_slice(b"EXTRA=\0");
        assert_eq!(validate(&blob), Err(EnvironmentError::TooLarge));
        let mut blob = Vec::new();
        for name in ["A", "B", "C", "D"] {
            blob.extend_from_slice(
                format!("{name}={}\0", "x".repeat(MAX_ENTRY_BYTES - 3)).as_bytes(),
            );
        }
        assert_eq!(blob.len(), MAX_BYTES);
        assert_eq!(validate(&blob), Ok(()));
        blob.push(0);
        assert_eq!(validate(&blob), Err(EnvironmentError::TooLarge));
        assert_eq!(
            validate(&vec![b'x'; MAX_BYTES + 1]),
            Err(EnvironmentError::TooLarge)
        );
        assert_eq!(
            entry_size("A", &"x".repeat(MAX_ENTRY_BYTES - 2)),
            Err(EnvironmentError::TooLarge)
        );
    }
}
