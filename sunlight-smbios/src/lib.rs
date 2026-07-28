//! Bounded SMBIOS entry-point validation and structure parser.
//!
//! Specification reference (implementation target):
//! - DMTF DSP0134 SMBIOS Specification **Version 3.9.0** (2025-07-07)
//!   (structure layouts remain compatible with 2.x/3.x entry points).
//!
//! Parsing rules (deliberate safety properties):
//! - explicit little-endian field reads;
//! - no unaligned typed pointer dereferences;
//! - checked arithmetic and table bounds;
//! - unknown structure types skipped via declared length;
//! - serial number / UUID are never placed in [`PublicSystemIdentity`].

#![no_std]

#[cfg(test)]
extern crate std;

/// Maximum accepted SMBIOS structure table size (bytes).
/// Rejects unreasonably large firmware tables before mapping/copying.
pub const MAX_TABLE_BYTES: usize = 64 * 1024;

/// Maximum length of a single identity string field (bytes, UTF-8).
pub const MAX_STRING_BYTES: usize = 64;

/// Maximum strings stored for one structure during parse.
const MAX_STRINGS_PER_STRUCT: usize = 16;

/// SMBIOS 2.x (32-bit) entry-point anchor `_SM_`.
pub const ANCHOR_32: [u8; 4] = *b"_SM_";
/// Intermediate anchor `_DMI_`.
pub const ANCHOR_INTERMEDIATE: [u8; 5] = *b"_DMI_";
/// SMBIOS 3.x (64-bit) entry-point anchor `_SM3_`.
pub const ANCHOR_64: [u8; 5] = *b"_SM3_";

/// Entry-point kind after validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryPointKind {
    /// 32-bit / legacy SMBIOS entry point (`_SM_` / `_DMI_`).
    Legacy32,
    /// 64-bit SMBIOS 3.x entry point (`_SM3_`).
    Smbios3,
}

/// Validated SMBIOS entry point (physical table location only; no raw identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedEntryPoint {
    pub kind: EntryPointKind,
    pub major: u8,
    pub minor: u8,
    pub docrev: u8,
    /// Physical address of the structure table.
    pub table_address: u64,
    /// Maximum structure table length reported by firmware.
    pub table_length: u32,
    /// Number of structures (legacy 32-bit only; 0 for 3.x).
    pub structure_count: u16,
}

impl ValidatedEntryPoint {
    pub const fn smbios_version_word(self) -> u16 {
        ((self.major as u16) << 8) | self.minor as u16
    }
}

/// Parse error for entry points / tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmbiosError {
    Truncated,
    BadAnchor,
    BadLength,
    BadChecksum,
    BadIntermediateChecksum,
    UnsupportedVersion,
    Overflow,
    OversizedTable,
    BadTableBounds,
    MalformedStructure,
    MissingEnd,
}

/// Public (non-unique) system identity for UI/services.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicSystemIdentity {
    pub manufacturer: StringBuf,
    pub product_name: StringBuf,
    pub product_version: StringBuf,
    pub board_manufacturer: StringBuf,
    pub board_product: StringBuf,
    pub bios_vendor: StringBuf,
    pub bios_version: StringBuf,
    pub bios_release_date: StringBuf,
    pub smbios_major: u8,
    pub smbios_minor: u8,
    pub identity_confidence: IdentityConfidence,
}

impl PublicSystemIdentity {
    pub const fn empty() -> Self {
        Self {
            manufacturer: StringBuf::empty(),
            product_name: StringBuf::empty(),
            product_version: StringBuf::empty(),
            board_manufacturer: StringBuf::empty(),
            board_product: StringBuf::empty(),
            bios_vendor: StringBuf::empty(),
            bios_version: StringBuf::empty(),
            bios_release_date: StringBuf::empty(),
            smbios_major: 0,
            smbios_minor: 0,
            identity_confidence: IdentityConfidence::None,
        }
    }

    /// True when manufacturer + product are both known non-placeholder strings.
    pub fn has_product_identity(self) -> bool {
        !self.manufacturer.is_unknown() && !self.product_name.is_unknown()
    }
}

/// Privileged unique identifiers — never put these in public IPC or boot logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivilegedUniqueIds {
    pub serial_number: StringBuf,
    pub uuid: [u8; 16],
    pub uuid_valid: bool,
}

impl PrivilegedUniqueIds {
    pub const fn empty() -> Self {
        Self {
            serial_number: StringBuf::empty(),
            uuid: [0; 16],
            uuid_valid: false,
        }
    }
}

/// Processor descriptive metadata from Type 4 (not trusted for topology).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorInfo {
    pub socket: StringBuf,
    pub manufacturer: StringBuf,
    pub version: StringBuf,
    pub core_count: Option<u16>,
    pub thread_count: Option<u16>,
}

impl ProcessorInfo {
    pub const fn empty() -> Self {
        Self {
            socket: StringBuf::empty(),
            manufacturer: StringBuf::empty(),
            version: StringBuf::empty(),
            core_count: None,
            thread_count: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IdentityConfidence {
    None = 0,
    Partial = 1,
    Full = 2,
}

/// Fixed-capacity UTF-8 string for identity fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringBuf {
    len: u8,
    data: [u8; MAX_STRING_BYTES],
}

impl StringBuf {
    pub const fn empty() -> Self {
        Self {
            len: 0,
            data: [0; MAX_STRING_BYTES],
        }
    }

    pub fn from_bytes(raw: &[u8]) -> Self {
        let trimmed = trim_firmware_bytes(raw);
        let mut out = Self::empty();
        let n = trimmed.len().min(MAX_STRING_BYTES);
        out.data[..n].copy_from_slice(&trimmed[..n]);
        out.len = n as u8;
        out
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_unknown(&self) -> bool {
        if self.is_empty() {
            return true;
        }
        is_placeholder_string(self.as_str())
    }

    /// Case-folded exact match for allowlists (ASCII fold only).
    pub fn eq_ignore_ascii_case(&self, other: &str) -> bool {
        let a = self.as_bytes();
        let b = other.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
    }
}

fn trim_firmware_bytes(raw: &[u8]) -> &[u8] {
    let mut end = raw.len();
    while end > 0 {
        let b = raw[end - 1];
        if b == 0 || b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
            end -= 1;
        } else {
            break;
        }
    }
    let mut start = 0;
    while start < end {
        let b = raw[start];
        if b == b' ' || b == b'\t' {
            start += 1;
        } else {
            break;
        }
    }
    &raw[start..end]
}

fn is_placeholder_string(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    // Known firmware placeholders (exact, case-insensitive).
    const PLACEHOLDERS: &[&str] = &[
        "to be filled by o.e.m.",
        "to be filled by oem",
        "default string",
        "system product name",
        "system manufacturer",
        "system version",
        "base board product name",
        "base board manufacturer",
        "none",
        "n/a",
        "na",
        "unknown",
        "not specified",
        "not available",
        "oem",
        "o.e.m.",
        "123456789",
        "0123456789",
    ];
    let mut buf = [0u8; MAX_STRING_BYTES];
    let bytes = s.as_bytes();
    let n = bytes.len().min(MAX_STRING_BYTES);
    for i in 0..n {
        buf[i] = bytes[i].to_ascii_lowercase();
    }
    let lower = core::str::from_utf8(&buf[..n]).unwrap_or("");
    PLACEHOLDERS.iter().any(|p| *p == lower)
}

/// Exact-match allowlist helper for future model-specific backends.
/// Uses case-insensitive ASCII equality on manufacturer and product only.
/// Never authorizes hardware writes by itself.
pub fn matches_product_allowlist(
    identity: &PublicSystemIdentity,
    manufacturer: &str,
    product: &str,
) -> bool {
    if !identity.has_product_identity() {
        return false;
    }
    identity.manufacturer.eq_ignore_ascii_case(manufacturer)
        && identity.product_name.eq_ignore_ascii_case(product)
}

fn checksum_ok(bytes: &[u8]) -> bool {
    let mut sum: u8 = 0;
    for b in bytes {
        sum = sum.wrapping_add(*b);
    }
    sum == 0
}

fn read_u16_le(data: &[u8], off: usize) -> Result<u16, SmbiosError> {
    let end = off.checked_add(2).ok_or(SmbiosError::Overflow)?;
    let slice = data.get(off..end).ok_or(SmbiosError::Truncated)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(data: &[u8], off: usize) -> Result<u32, SmbiosError> {
    let end = off.checked_add(4).ok_or(SmbiosError::Overflow)?;
    let slice = data.get(off..end).ok_or(SmbiosError::Truncated)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64_le(data: &[u8], off: usize) -> Result<u64, SmbiosError> {
    let end = off.checked_add(8).ok_or(SmbiosError::Overflow)?;
    let slice = data.get(off..end).ok_or(SmbiosError::Truncated)?;
    let mut b = [0u8; 8];
    b.copy_from_slice(slice);
    Ok(u64::from_le_bytes(b))
}

/// Validate a 32-bit SMBIOS entry point at `data` (must start at anchor).
pub fn validate_entry_point_32(data: &[u8]) -> Result<ValidatedEntryPoint, SmbiosError> {
    // Minimum length field at offset 5; full EPS is at least 0x1F bytes.
    if data.len() < 0x1F {
        return Err(SmbiosError::Truncated);
    }
    if data[0..4] != ANCHOR_32 {
        return Err(SmbiosError::BadAnchor);
    }
    let ep_length = data[5] as usize;
    if ep_length < 0x1F || ep_length > data.len() {
        return Err(SmbiosError::BadLength);
    }
    if !checksum_ok(&data[..ep_length]) {
        return Err(SmbiosError::BadChecksum);
    }
    // Intermediate area starts at offset 0x10: _DMI_ + checksum + length + address + count + bcd.
    if data[0x10..0x15] != ANCHOR_INTERMEDIATE {
        return Err(SmbiosError::BadAnchor);
    }
    // Intermediate checksum covers offsets 0x10..0x1F (15 bytes).
    if ep_length < 0x1F || !checksum_ok(&data[0x10..0x1F]) {
        return Err(SmbiosError::BadIntermediateChecksum);
    }
    let major = data[6];
    let minor = data[7];
    if major < 2 {
        return Err(SmbiosError::UnsupportedVersion);
    }
    let table_length = read_u16_le(data, 0x16)? as u32;
    let table_address = read_u32_le(data, 0x18)? as u64;
    let structure_count = read_u16_le(data, 0x1C)?;
    if table_length == 0 {
        return Err(SmbiosError::BadTableBounds);
    }
    if table_length as usize > MAX_TABLE_BYTES {
        return Err(SmbiosError::OversizedTable);
    }
    // 32-bit address cannot overflow u64; still reject wrap of address+length.
    let end = table_address
        .checked_add(table_length as u64)
        .ok_or(SmbiosError::Overflow)?;
    if end < table_address {
        return Err(SmbiosError::Overflow);
    }
    Ok(ValidatedEntryPoint {
        kind: EntryPointKind::Legacy32,
        major,
        minor,
        docrev: 0,
        table_address,
        table_length,
        structure_count,
    })
}

/// Validate a 64-bit SMBIOS 3.x entry point.
pub fn validate_entry_point_64(data: &[u8]) -> Result<ValidatedEntryPoint, SmbiosError> {
    if data.len() < 0x18 {
        return Err(SmbiosError::Truncated);
    }
    if data[0..5] != ANCHOR_64 {
        return Err(SmbiosError::BadAnchor);
    }
    let ep_length = data[6] as usize;
    if ep_length < 0x18 || ep_length > data.len() {
        return Err(SmbiosError::BadLength);
    }
    if !checksum_ok(&data[..ep_length]) {
        return Err(SmbiosError::BadChecksum);
    }
    let major = data[7];
    let minor = data[8];
    let docrev = data[9];
    if major < 3 {
        return Err(SmbiosError::UnsupportedVersion);
    }
    let table_max_size = read_u32_le(data, 0x0C)?;
    let table_address = read_u64_le(data, 0x10)?;
    if table_max_size == 0 {
        return Err(SmbiosError::BadTableBounds);
    }
    if table_max_size as usize > MAX_TABLE_BYTES {
        return Err(SmbiosError::OversizedTable);
    }
    let _end = table_address
        .checked_add(table_max_size as u64)
        .ok_or(SmbiosError::Overflow)?;
    Ok(ValidatedEntryPoint {
        kind: EntryPointKind::Smbios3,
        major,
        minor,
        docrev,
        table_address,
        table_length: table_max_size,
        structure_count: 0,
    })
}

/// Prefer a valid SMBIOS 3.x entry point when both are valid.
pub fn select_entry_point(
    ep32: Option<ValidatedEntryPoint>,
    ep64: Option<ValidatedEntryPoint>,
) -> Option<ValidatedEntryPoint> {
    match (ep32, ep64) {
        (_, Some(e64)) => Some(e64),
        (Some(e32), None) => Some(e32),
        (None, None) => None,
    }
}

#[derive(Clone, Copy)]
struct StructHeader {
    typ: u8,
    length: u8,
    #[allow(dead_code)]
    handle: u16,
}

/// Parse public identity from a validated structure table.
pub fn parse_public_identity(
    table: &[u8],
    ep: &ValidatedEntryPoint,
) -> Result<(PublicSystemIdentity, PrivilegedUniqueIds, ProcessorInfo), SmbiosError> {
    if table.len() > MAX_TABLE_BYTES {
        return Err(SmbiosError::OversizedTable);
    }
    let limit = (ep.table_length as usize).min(table.len());
    let table = &table[..limit];

    let mut identity = PublicSystemIdentity::empty();
    identity.smbios_major = ep.major;
    identity.smbios_minor = ep.minor;
    let mut privileged = PrivilegedUniqueIds::empty();
    let mut processor = ProcessorInfo::empty();
    let mut saw_type0 = false;
    let mut saw_type1 = false;
    let mut saw_type2 = false;
    let mut saw_type4 = false;
    let mut saw_end = false;

    let mut off = 0usize;
    while off < table.len() {
        let hdr = read_header(table, off)?;
        if hdr.typ == 127 {
            saw_end = true;
            break;
        }
        let formatted_end = off
            .checked_add(hdr.length as usize)
            .ok_or(SmbiosError::Overflow)?;
        if formatted_end > table.len() || hdr.length < 4 {
            return Err(SmbiosError::MalformedStructure);
        }
        let strings_start = formatted_end;
        let (strings_end, strings) = parse_string_area(table, strings_start)?;
        match hdr.typ {
            0 if !saw_type0 => {
                // Type 0 BIOS Information: vendor@4, version@5, release_date@8 (if length)
                let vendor = string_at(&strings, field_u8(table, off, 4, hdr.length)?);
                let version = string_at(&strings, field_u8(table, off, 5, hdr.length)?);
                let release = if hdr.length > 8 {
                    string_at(&strings, field_u8(table, off, 8, hdr.length)?)
                } else {
                    StringBuf::empty()
                };
                identity.bios_vendor = normalize_field(vendor);
                identity.bios_version = normalize_field(version);
                identity.bios_release_date = normalize_field(release);
                saw_type0 = true;
            }
            1 if !saw_type1 => {
                // Type 1 System Information
                let manufacturer = string_at(&strings, field_u8(table, off, 4, hdr.length)?);
                let product = string_at(&strings, field_u8(table, off, 5, hdr.length)?);
                let version = string_at(&strings, field_u8(table, off, 6, hdr.length)?);
                let serial = string_at(&strings, field_u8(table, off, 7, hdr.length)?);
                identity.manufacturer = normalize_field(manufacturer);
                identity.product_name = normalize_field(product);
                identity.product_version = normalize_field(version);
                // Serial is privileged only.
                privileged.serial_number = normalize_field(serial);
                if hdr.length >= 0x19 {
                    let mut uuid = [0u8; 16];
                    uuid.copy_from_slice(&table[off + 8..off + 24]);
                    if is_uuid_valid(&uuid) {
                        privileged.uuid = uuid;
                        privileged.uuid_valid = true;
                    }
                }
                saw_type1 = true;
            }
            2 if !saw_type2 => {
                let manufacturer = string_at(&strings, field_u8(table, off, 4, hdr.length)?);
                let product = string_at(&strings, field_u8(table, off, 5, hdr.length)?);
                let version = string_at(&strings, field_u8(table, off, 6, hdr.length)?);
                identity.board_manufacturer = normalize_field(manufacturer);
                identity.board_product = normalize_field(product);
                // board version not in PublicSystemIdentity product_version
                let _ = version;
                saw_type2 = true;
            }
            4 if !saw_type4 => {
                let socket = string_at(&strings, field_u8(table, off, 4, hdr.length)?);
                let manufacturer = string_at(&strings, field_u8(table, off, 7, hdr.length)?);
                let version = string_at(&strings, field_u8(table, off, 0x10, hdr.length)?);
                processor.socket = normalize_field(socket);
                processor.manufacturer = normalize_field(manufacturer);
                processor.version = normalize_field(version);
                // Core/thread counts when structure length supports them (SMBIOS 2.5+ / 3.0+).
                if hdr.length >= 0x24 {
                    let cores = table[off + 0x23] as u16;
                    if cores != 0 {
                        processor.core_count = Some(cores);
                    }
                }
                if hdr.length >= 0x26 {
                    let threads = table[off + 0x25] as u16;
                    if threads != 0 {
                        processor.thread_count = Some(threads);
                    }
                }
                // SMBIOS 3.0 extended core/thread at 0x2A/0x2E when length permits.
                if hdr.length >= 0x2E {
                    let cores_ext = read_u16_le(table, off + 0x2A).unwrap_or(0);
                    let threads_ext = read_u16_le(table, off + 0x2C).unwrap_or(0);
                    if cores_ext != 0 {
                        processor.core_count = Some(cores_ext);
                    }
                    if threads_ext != 0 {
                        processor.thread_count = Some(threads_ext);
                    }
                }
                saw_type4 = true;
            }
            _ => {
                // Unknown or duplicate: skip deterministically.
            }
        }
        off = strings_end;
    }

    if !saw_end && off < table.len() {
        // Legacy tables may omit end-of-table when structure_count is exhausted;
        // still accept if we consumed the advertised region without OOB.
    } else if !saw_end && ep.kind == EntryPointKind::Smbios3 && off >= table.len() {
        // 3.x may end at max size without type 127 if truncated — reject if empty.
        if !saw_type0 && !saw_type1 {
            return Err(SmbiosError::MissingEnd);
        }
    }

    identity.identity_confidence = if identity.has_product_identity() && saw_type0 {
        IdentityConfidence::Full
    } else if saw_type0 || saw_type1 || saw_type2 {
        IdentityConfidence::Partial
    } else {
        IdentityConfidence::None
    };

    Ok((identity, privileged, processor))
}

fn normalize_field(s: StringBuf) -> StringBuf {
    if s.is_unknown() {
        StringBuf::empty()
    } else {
        s
    }
}

fn is_uuid_valid(uuid: &[u8; 16]) -> bool {
    // All zeros or all 0xFF are not valid unique identifiers.
    !uuid.iter().all(|&b| b == 0) && !uuid.iter().all(|&b| b == 0xFF)
}

fn read_header(table: &[u8], off: usize) -> Result<StructHeader, SmbiosError> {
    if off.checked_add(4).ok_or(SmbiosError::Overflow)? > table.len() {
        return Err(SmbiosError::MalformedStructure);
    }
    Ok(StructHeader {
        typ: table[off],
        length: table[off + 1],
        handle: u16::from_le_bytes([table[off + 2], table[off + 3]]),
    })
}

fn field_u8(table: &[u8], base: usize, rel: usize, formatted_len: u8) -> Result<u8, SmbiosError> {
    if rel >= formatted_len as usize {
        return Ok(0); // field not present → no string
    }
    let idx = base.checked_add(rel).ok_or(SmbiosError::Overflow)?;
    table
        .get(idx)
        .copied()
        .ok_or(SmbiosError::MalformedStructure)
}

fn parse_string_area(
    table: &[u8],
    start: usize,
) -> Result<(usize, [StringBuf; MAX_STRINGS_PER_STRUCT]), SmbiosError> {
    let mut strings = [StringBuf::empty(); MAX_STRINGS_PER_STRUCT];
    // Empty string area is double-NUL immediately.
    if start >= table.len() {
        return Err(SmbiosError::MalformedStructure);
    }
    if start + 1 < table.len() && table[start] == 0 && table[start + 1] == 0 {
        return Ok((start + 2, strings));
    }
    let mut i = start;
    let mut count = 0usize;
    while i < table.len() {
        // Find next NUL.
        let mut j = i;
        while j < table.len() && table[j] != 0 {
            j += 1;
        }
        if j >= table.len() {
            return Err(SmbiosError::MalformedStructure);
        }
        if count < MAX_STRINGS_PER_STRUCT {
            strings[count] = StringBuf::from_bytes(&table[i..j]);
            count += 1;
        }
        j += 1; // skip NUL
        if j < table.len() && table[j] == 0 {
            // double-NUL terminator
            return Ok((j + 1, strings));
        }
        i = j;
    }
    Err(SmbiosError::MalformedStructure)
}

/// String indexes are one-based; 0 means no string.
fn string_at(strings: &[StringBuf; MAX_STRINGS_PER_STRUCT], index: u8) -> StringBuf {
    if index == 0 {
        return StringBuf::empty();
    }
    let i = (index as usize).saturating_sub(1);
    if i >= MAX_STRINGS_PER_STRUCT {
        return StringBuf::empty();
    }
    let s = strings[i];
    // Empty slot means invalid index (structure had fewer strings).
    if s.is_empty() && index as usize > 0 {
        // Distinguish missing trailing strings: treat as unknown/empty.
        return StringBuf::empty();
    }
    s
}

/// Build a minimal synthetic Type structure for tests.
#[cfg(test)]
pub mod test_util {
    use super::*;

    pub fn append_struct(buf: &mut std::vec::Vec<u8>, typ: u8, formatted: &[u8], strings: &[&str]) {
        let length = (4 + formatted.len()) as u8;
        buf.push(typ);
        buf.push(length);
        buf.push(0);
        buf.push(0); // handle
        buf.extend_from_slice(formatted);
        if strings.is_empty() {
            buf.push(0);
            buf.push(0);
        } else {
            for s in strings {
                buf.extend_from_slice(s.as_bytes());
                buf.push(0);
            }
            buf.push(0); // second NUL
        }
    }

    pub fn end_of_table(buf: &mut std::vec::Vec<u8>) {
        append_struct(buf, 127, &[], &[]);
    }

    pub fn make_ep32(table_addr: u32, table_len: u16, major: u8, minor: u8) -> std::vec::Vec<u8> {
        let mut ep = std::vec![0u8; 0x1F];
        ep[0..4].copy_from_slice(b"_SM_");
        ep[5] = 0x1F;
        ep[6] = major;
        ep[7] = minor;
        ep[0x10..0x15].copy_from_slice(b"_DMI_");
        ep[0x16] = (table_len & 0xff) as u8;
        ep[0x17] = (table_len >> 8) as u8;
        ep[0x18] = (table_addr & 0xff) as u8;
        ep[0x19] = ((table_addr >> 8) & 0xff) as u8;
        ep[0x1A] = ((table_addr >> 16) & 0xff) as u8;
        ep[0x1B] = ((table_addr >> 24) & 0xff) as u8;
        ep[0x1C] = 4; // structure count low
        ep[0x1D] = 0;
        // Intermediate checksum at 0x15 covers 0x10..0x1F
        ep[0x15] = 0;
        let mut sum: u8 = 0;
        for b in &ep[0x10..0x1F] {
            sum = sum.wrapping_add(*b);
        }
        ep[0x15] = (0u8).wrapping_sub(sum);
        // Entry-point checksum at offset 4
        ep[4] = 0;
        let mut sum: u8 = 0;
        for b in &ep[..0x1F] {
            sum = sum.wrapping_add(*b);
        }
        ep[4] = (0u8).wrapping_sub(sum);
        ep
    }

    pub fn make_ep64(table_addr: u64, table_max: u32, major: u8, minor: u8) -> std::vec::Vec<u8> {
        let mut ep = std::vec![0u8; 0x18];
        ep[0..5].copy_from_slice(b"_SM3_");
        ep[6] = 0x18;
        ep[7] = major;
        ep[8] = minor;
        ep[9] = 0; // docrev
        ep[0x0C..0x10].copy_from_slice(&table_max.to_le_bytes());
        ep[0x10..0x18].copy_from_slice(&table_addr.to_le_bytes());
        ep[5] = 0;
        let mut sum: u8 = 0;
        for b in &ep[..0x18] {
            sum = sum.wrapping_add(*b);
        }
        ep[5] = (0u8).wrapping_sub(sum);
        ep
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;
    use std::vec::Vec;

    fn sample_table() -> Vec<u8> {
        let mut t = Vec::new();
        // Type 0: vendor, version, segment(u16), release date index
        // formatted after header: vendor_idx, ver_idx, start_seg(u16), release_idx, ...
        append_struct(
            &mut t,
            0,
            &[
                1, 2, 0x00, 0xE8, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            &["Lenovo", "GJET75WW (2.25 )", "06/25/2019"],
        );
        // Type 1
        let mut formatted = std::vec![1u8, 2, 3, 4]; // mfr, product, version, serial indices
                                                     // UUID 16 bytes
        formatted.extend_from_slice(&[
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x01,
        ]);
        // wake-up etc padding to length
        formatted.extend_from_slice(&[0, 0, 0, 0, 0]);
        append_struct(
            &mut t,
            1,
            &formatted,
            &["LENOVO", "20AW00xxxx", "ThinkPad T440p", "PF0SECRET"],
        );
        // Type 2
        append_struct(
            &mut t,
            2,
            &[1, 2, 3, 0, 0, 0, 0, 0],
            &["LENOVO", "20AWCTO1WW", "Not Defined"],
        );
        // Type 4
        let mut p = std::vec![0u8; 0x2A];
        p[0] = 1; // socket
        p[3] = 2; // manufacturer @ offset 7 from structure start → rel 3 from formatted?
                  // formatted starts at structure offset 4; socket is at offset 4 → formatted[0]
                  // manufacturer at offset 7 → formatted[3]
                  // version at offset 0x10 → formatted[0x0C]
        p[0x0C] = 3;
        p[0x1F] = 4; // core count at structure offset 0x23 → formatted offset 0x1F
        p[0x21] = 8; // thread count at 0x25
        append_struct(
            &mut t,
            4,
            &p,
            &["CPU0", "Intel(R) Corporation", "Intel(R) Core(TM) i7"],
        );
        // Unknown type
        append_struct(&mut t, 99, &[1, 2, 3, 4], &["ignore"]);
        end_of_table(&mut t);
        t
    }

    #[test]
    fn valid_entry_point_32() {
        let ep = make_ep32(0x1000, 256, 2, 8);
        let v = validate_entry_point_32(&ep).unwrap();
        assert_eq!(v.kind, EntryPointKind::Legacy32);
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 8);
        assert_eq!(v.table_address, 0x1000);
        assert_eq!(v.table_length, 256);
    }

    #[test]
    fn valid_entry_point_64() {
        let ep = make_ep64(0x1_0000_0000, 512, 3, 2);
        let v = validate_entry_point_64(&ep).unwrap();
        assert_eq!(v.kind, EntryPointKind::Smbios3);
        assert_eq!(v.table_address, 0x1_0000_0000);
        assert_eq!(v.table_length, 512);
    }

    #[test]
    fn prefer_64_when_both() {
        let e32 = validate_entry_point_32(&make_ep32(0x1000, 100, 2, 7)).unwrap();
        let e64 = validate_entry_point_64(&make_ep64(0x2000, 100, 3, 0)).unwrap();
        let sel = select_entry_point(Some(e32), Some(e64)).unwrap();
        assert_eq!(sel.kind, EntryPointKind::Smbios3);
    }

    #[test]
    fn invalid_checksum() {
        let mut ep = make_ep32(0x1000, 100, 2, 7);
        ep[4] ^= 0xFF;
        assert_eq!(validate_entry_point_32(&ep), Err(SmbiosError::BadChecksum));
    }

    #[test]
    fn invalid_intermediate_checksum() {
        let mut ep = make_ep32(0x1000, 100, 2, 7);
        ep[0x15] ^= 0xFF;
        // Fix overall checksum so intermediate is the failing check.
        ep[4] = 0;
        let mut sum: u8 = 0;
        for b in &ep[..0x1F] {
            sum = sum.wrapping_add(*b);
        }
        ep[4] = (0u8).wrapping_sub(sum);
        assert_eq!(
            validate_entry_point_32(&ep),
            Err(SmbiosError::BadIntermediateChecksum)
        );
    }

    #[test]
    fn truncated_entry_point() {
        let ep = make_ep32(0x1000, 100, 2, 7);
        assert_eq!(
            validate_entry_point_32(&ep[..10]),
            Err(SmbiosError::Truncated)
        );
    }

    #[test]
    fn address_length_overflow_64() {
        let mut ep = make_ep64(u64::MAX - 10, 32, 3, 0);
        // Remake checksum after... already in make; force table_addr high
        assert_eq!(validate_entry_point_64(&ep), Err(SmbiosError::Overflow));
        let _ = &mut ep;
    }

    #[test]
    fn oversized_table() {
        let ep = make_ep64(0x1000, (MAX_TABLE_BYTES as u32) + 1, 3, 0);
        assert_eq!(
            validate_entry_point_64(&ep),
            Err(SmbiosError::OversizedTable)
        );
    }

    #[test]
    fn parse_types_0_1_2_4() {
        let table = sample_table();
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Legacy32,
            major: 2,
            minor: 8,
            docrev: 0,
            table_address: 0,
            table_length: table.len() as u32,
            structure_count: 6,
        };
        let (pub_id, priv_id, proc) = parse_public_identity(&table, &ep).unwrap();
        assert_eq!(pub_id.bios_vendor.as_str(), "Lenovo");
        assert_eq!(pub_id.manufacturer.as_str(), "LENOVO");
        assert_eq!(pub_id.product_name.as_str(), "20AW00xxxx");
        assert_eq!(pub_id.board_product.as_str(), "20AWCTO1WW");
        assert_eq!(proc.socket.as_str(), "CPU0");
        // Privileged present but public must not include them as fields
        assert_eq!(priv_id.serial_number.as_str(), "PF0SECRET");
        assert!(priv_id.uuid_valid);
        // Public identity has no serial/uuid members — compile-time separation.
        let _ = pub_id.product_version;
    }

    #[test]
    fn unknown_structure_skipped() {
        let table = sample_table();
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Smbios3,
            major: 3,
            minor: 2,
            docrev: 0,
            table_address: 0,
            table_length: table.len() as u32,
            structure_count: 0,
        };
        assert!(parse_public_identity(&table, &ep).is_ok());
    }

    #[test]
    fn structure_too_short_header() {
        let table = [1u8, 3, 0, 0]; // length 3 < 4
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Legacy32,
            major: 2,
            minor: 0,
            docrev: 0,
            table_address: 0,
            table_length: 4,
            structure_count: 1,
        };
        assert_eq!(
            parse_public_identity(&table, &ep),
            Err(SmbiosError::MalformedStructure)
        );
    }

    #[test]
    fn structure_extends_beyond_table() {
        let table = [0u8, 20, 0, 0, 1, 2]; // claims length 20 but only 6 bytes
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Legacy32,
            major: 2,
            minor: 0,
            docrev: 0,
            table_address: 0,
            table_length: 6,
            structure_count: 1,
        };
        assert!(parse_public_identity(&table, &ep).is_err());
    }

    #[test]
    fn missing_double_nul() {
        let mut t = Vec::new();
        t.extend_from_slice(&[0, 5, 0, 0, 1]); // type0 length5 vendor_idx=1
        t.extend_from_slice(b"Vendor"); // no NULs
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Legacy32,
            major: 2,
            minor: 0,
            docrev: 0,
            table_address: 0,
            table_length: t.len() as u32,
            structure_count: 1,
        };
        assert!(parse_public_identity(&t, &ep).is_err());
    }

    #[test]
    fn invalid_string_index_safe() {
        let mut t = Vec::new();
        append_struct(&mut t, 0, &[9, 0, 0, 0, 0], &["OnlyOne"]); // index 9 invalid
        end_of_table(&mut t);
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Legacy32,
            major: 2,
            minor: 0,
            docrev: 0,
            table_address: 0,
            table_length: t.len() as u32,
            structure_count: 2,
        };
        let (id, _, _) = parse_public_identity(&t, &ep).unwrap();
        assert!(id.bios_vendor.is_empty());
    }

    #[test]
    fn empty_and_placeholder_strings() {
        let mut t = Vec::new();
        // Index 0 = no string; placeholders normalize to empty/unknown.
        // SMBIOS string area cannot embed a true middle empty string (double-NUL
        // ends the area), so empty is expressed via string index 0.
        append_struct(
            &mut t,
            1,
            &[
                1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            &["To Be Filled By O.E.M.", "System Version"],
        );
        end_of_table(&mut t);
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Legacy32,
            major: 2,
            minor: 0,
            docrev: 0,
            table_address: 0,
            table_length: t.len() as u32,
            structure_count: 2,
        };
        let (id, _, _) = parse_public_identity(&t, &ep).unwrap();
        assert!(id.manufacturer.is_empty()); // placeholder → unknown
        assert!(id.product_name.is_empty()); // index 0 → no string
        assert!(id.product_version.is_empty()); // "System Version" placeholder
    }

    #[test]
    fn end_of_table_marker() {
        let mut t = Vec::new();
        end_of_table(&mut t);
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Legacy32,
            major: 2,
            minor: 0,
            docrev: 0,
            table_address: 0,
            table_length: t.len() as u32,
            structure_count: 1,
        };
        let (id, _, _) = parse_public_identity(&t, &ep).unwrap();
        assert_eq!(id.identity_confidence, IdentityConfidence::None);
    }

    #[test]
    fn newer_optional_fields_skipped() {
        // Type 0 with extra trailing formatted bytes beyond what we read.
        let mut t = Vec::new();
        let mut formatted = std::vec![1u8, 2, 0, 0, 3];
        formatted.extend_from_slice(&[0xAA; 40]); // future fields
        append_struct(&mut t, 0, &formatted, &["VendorX", "1.0", "01/01/2020"]);
        end_of_table(&mut t);
        let ep = ValidatedEntryPoint {
            kind: EntryPointKind::Smbios3,
            major: 3,
            minor: 7,
            docrev: 0,
            table_address: 0,
            table_length: t.len() as u32,
            structure_count: 0,
        };
        let (id, _, _) = parse_public_identity(&t, &ep).unwrap();
        assert_eq!(id.bios_vendor.as_str(), "VendorX");
    }

    #[test]
    fn serial_uuid_absent_from_public_identity() {
        // Ensure PublicSystemIdentity has no serial/uuid by field access only.
        let id = PublicSystemIdentity::empty();
        let _ = id.manufacturer;
        let _ = id.product_name;
        // Privileged is separate type.
        let p = PrivilegedUniqueIds::empty();
        assert!(!p.uuid_valid);
        assert!(p.serial_number.is_empty());
    }

    #[test]
    fn allowlist_exact_match() {
        let mut id = PublicSystemIdentity::empty();
        id.manufacturer = StringBuf::from_bytes(b"LENOVO");
        id.product_name = StringBuf::from_bytes(b"20AWCTO1WW");
        assert!(matches_product_allowlist(&id, "Lenovo", "20AWCTO1WW"));
        assert!(!matches_product_allowlist(&id, "Lenovo", "T440p")); // no loose substring
    }
}
