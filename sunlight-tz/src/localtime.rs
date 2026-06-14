//! Read/write /etc/localtime as JSON via VFS IPC

use sunlight_ipc::{
    CapabilityToken, IpcMsg, VfsMsg, debug_log, ipc_call, nameserver_lookup,
};

// tz_by_id is available to callers who want to validate ids against CSV before write.

/// The active timezone config read from /etc/localtime
#[derive(Clone, Copy, Debug)]
pub struct LocalTimeCfg {
    pub id:                 [u8; 64],   // IANA id, null-terminated
    pub id_len:             usize,
    pub display_name:       [u8; 128],
    pub display_name_len:   usize,
    pub utc_offset_hours:   i8,
    pub utc_offset_minutes: u8,
    pub dst_offset_minutes: u8,
    pub dst_start_month:    u8,
    pub dst_end_month:      u8,
}

impl LocalTimeCfg {
    pub const fn utc_default() -> Self {
        Self {
            id: [0u8; 64],
            id_len: 3,
            display_name: [0u8; 128],
            display_name_len: 24,
            utc_offset_hours: 0,
            utc_offset_minutes: 0,
            dst_offset_minutes: 0,
            dst_start_month: 0,
            dst_end_month: 0,
        }
    }

    pub fn id_str(&self) -> &str {
        core::str::from_utf8(&self.id[..self.id_len]).unwrap_or("UTC")
    }

    /// Convert to TzEntry-compatible offset for use with offset.rs
    pub fn to_tz_entry_fields(&self) -> (i8, u8, u8, u8, u8) {
        (self.utc_offset_hours, self.utc_offset_minutes,
         self.dst_offset_minutes, self.dst_start_month, self.dst_end_month)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TzError {
    VfsUnavailable,
    FileNotFound,
    ParseError,
    WriteError,
    UnknownZone,
}

/// Read /etc/localtime via VFS. On any failure return UTC default (never panic).
pub fn read_localtime() -> LocalTimeCfg {
    let vfs_cap = match nameserver_lookup("vfs") {
        Some(c) => c,
        None => {
            debug_log("[TZ] VFS unavailable for localtime read");
            return LocalTimeCfg::utc_default();
        }
    };

    let data = read_file_vfs(vfs_cap, "/etc/localtime");
    if data.is_empty() {
        debug_log("[TZ] /etc/localtime missing or empty, using UTC");
        return LocalTimeCfg::utc_default();
    }

    match parse_localtime_json(&data) {
        Some(cfg) => cfg,
        None => {
            debug_log("[TZ] /etc/localtime parse failed, using UTC");
            LocalTimeCfg::utc_default()
        }
    }
}

/// Write cfg as JSON to /etc/localtime. Returns Err on VFS or write failure.
pub fn write_localtime(cfg: &LocalTimeCfg) -> Result<(), TzError> {
    let vfs_cap = nameserver_lookup("vfs").ok_or(TzError::VfsUnavailable)?;

    let json = serialize_localtime_json(cfg);
    write_file_vfs(vfs_cap, "/etc/localtime", &json).map_err(|_| TzError::WriteError)
}

/// Very small VFS file reader (stack only, chunks via words like sunshell).
fn read_file_vfs(vfs_cap: CapabilityToken, path: &str) -> [u8; 512] {
    let mut out = [0u8; 512];
    let mut out_len = 0usize;

    let open_msg = path_msg(VfsMsg::OPEN, path);
    let reply = ipc_call(vfs_cap, open_msg);
    if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
        return out;
    }
    let handle = reply.words[1] as u32;
    let mut offset = 0usize;

    loop {
        let read_msg = IpcMsg::with_label(VfsMsg::READ)
            .word(0, handle as u64)
            .word(1, offset as u64)
            .word(2, 16);
        let r = ipc_call(vfs_cap, read_msg);
        if r.label != VfsMsg::REPLY {
            break;
        }
        let n = r.words[1] as usize;
        if n == 0 { break; }

        // data is packed in words[2..] little-endian bytes
        let src_words = &r.words[2..];
        for i in 0..n {
            if out_len >= out.len() { break; }
            let word_idx = i / 8;
            let byte_idx = i % 8;
            let b = ((src_words[word_idx] >> (byte_idx * 8)) & 0xFF) as u8;
            out[out_len] = b;
            out_len += 1;
        }
        offset += n;
        if n < 16 { break; }
    }

    let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));
    out
}

/// VFS file writer (small files only).
fn write_file_vfs(vfs_cap: CapabilityToken, path: &str, data: &[u8]) -> Result<(), ()> {
    let open_msg = path_msg(VfsMsg::OPEN, path);
    let reply = ipc_call(vfs_cap, open_msg);
    if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
        // Try to create? For now fail if cannot open for write (ramfs allows write on existing).
        return Err(());
    }
    let handle = reply.words[1] as u32;
    let mut offset = 0usize;

    while offset < data.len() {
        let chunk_end = (offset + 16).min(data.len());
        let chunk = &data[offset..chunk_end];
        let mut msg = IpcMsg::with_label(VfsMsg::WRITE)
            .word(0, handle as u64)
            .word(1, offset as u64);
        let mut word_idx = 2usize;
        let mut byte_idx = 0usize;
        let mut word = 0u64;
        for &b in chunk {
            word |= (b as u64) << (byte_idx * 8);
            byte_idx += 1;
            if byte_idx == 8 {
                msg = msg.word(word_idx, word);
                word = 0;
                byte_idx = 0;
                word_idx += 1;
            }
        }
        if byte_idx > 0 {
            msg = msg.word(word_idx, word);
        }
        let r = ipc_call(vfs_cap, msg);
        if r.label != VfsMsg::REPLY || r.words[0] != 0 {
            let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));
            return Err(());
        }
        let n = r.words[1] as usize;
        offset += n;
        if n == 0 { break; }
    }

    let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));
    Ok(())
}

/// Build a VfsMsg path-carrying message (first 32 bytes of path in words[0..3]).
fn path_msg(label: u64, path: &str) -> IpcMsg {
    let bytes = path.as_bytes();
    let mut msg = IpcMsg::with_label(label);
    for word_idx in 0..4 {
        let start = word_idx * 8;
        let end = (start + 8).min(bytes.len());
        if start < bytes.len() {
            let mut w = 0u64;
            for (bi, &b) in bytes[start..end].iter().enumerate() {
                w |= (b as u64) << (bi * 8);
            }
            msg = msg.word(word_idx, w);
        }
    }
    msg
}

/// Parse the exact 7-field JSON for localtime. Very tolerant field scanner, no alloc.
fn parse_localtime_json(data: &[u8]) -> Option<LocalTimeCfg> {
    let s = core::str::from_utf8(data).ok()?;
    let mut cfg = LocalTimeCfg::utc_default();

    // Find each "key": value
    if let Some(id) = extract_json_string(s, "id") {
        let bytes = id.as_bytes();
        let len = bytes.len().min(63);
        cfg.id[..len].copy_from_slice(&bytes[..len]);
        cfg.id[len] = 0;
        cfg.id_len = len;
    } else { return None; }

    if let Some(dn) = extract_json_string(s, "display_name") {
        let bytes = dn.as_bytes();
        let len = bytes.len().min(127);
        cfg.display_name[..len].copy_from_slice(&bytes[..len]);
        cfg.display_name[len] = 0;
        cfg.display_name_len = len;
    }

    if let Some(v) = extract_json_int(s, "utc_offset_hours") {
        cfg.utc_offset_hours = v as i8;
    }
    if let Some(v) = extract_json_int(s, "utc_offset_minutes") {
        cfg.utc_offset_minutes = v as u8;
    }
    if let Some(v) = extract_json_int(s, "dst_offset_minutes") {
        cfg.dst_offset_minutes = v as u8;
    }
    if let Some(v) = extract_json_int(s, "dst_start_month") {
        cfg.dst_start_month = v as u8;
    }
    if let Some(v) = extract_json_int(s, "dst_end_month") {
        cfg.dst_end_month = v as u8;
    }

    Some(cfg)
}

fn extract_json_string<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    // Look for "key": "...."
    let pat = "\"";
    let _key_pat = "\"";
    // naive scan
    let mut search = s;
    while let Some(pos) = search.find(key) {
        let after_key = &search[pos + key.len()..];
        if let Some(col) = after_key.find(':') {
            let rest = after_key[col + 1..].trim_start();
            if rest.starts_with('"') {
                let content = &rest[1..];
                if let Some(endq) = content.find('"') {
                    return Some(&content[..endq]);
                }
            }
        }
        // advance
        search = &search[pos + 1..];
    }
    None
}

fn extract_json_int(s: &str, key: &str) -> Option<i32> {
    let mut search = s;
    while let Some(pos) = search.find(key) {
        let after = &search[pos + key.len()..];
        if let Some(col) = after.find(':') {
            let num_str = after[col + 1..].trim_start();
            // allow optional sign and digits
            let mut end = 0usize;
            let bytes = num_str.as_bytes();
            if !bytes.is_empty() && (bytes[0] == b'-' || bytes[0] == b'+') { end = 1; }
            while end < bytes.len() && bytes[end].is_ascii_digit() { end += 1; }
            if end > 0 {
                let candidate = &num_str[..end];
                if let Ok(v) = crate::config::parse_i32(candidate) {
                    return Some(v);
                }
            }
        }
        search = &search[pos + 1..];
    }
    None
}

/// Serialize cfg to compact JSON in stack buffer (mirrors timed/state.rs style).
fn serialize_localtime_json(cfg: &LocalTimeCfg) -> [u8; 512] {
    let mut buf = [0u8; 512];
    let mut pos = 0usize;

    // {"id":"...","display_name":"...","utc_offset_hours":N,...}
    let start = b"{\"id\":\"";
    copy(&mut buf, &mut pos, start);

    copy(&mut buf, &mut pos, &cfg.id[..cfg.id_len]);
    copy(&mut buf, &mut pos, b"\",\"display_name\":\"");
    copy(&mut buf, &mut pos, &cfg.display_name[..cfg.display_name_len]);
    copy(&mut buf, &mut pos, b"\",\"utc_offset_hours\":");
    pos += append_i32(&mut buf[pos..], cfg.utc_offset_hours as i32);

    copy(&mut buf, &mut pos, b",\"utc_offset_minutes\":");
    pos += append_u8(&mut buf[pos..], cfg.utc_offset_minutes);

    copy(&mut buf, &mut pos, b",\"dst_offset_minutes\":");
    pos += append_u8(&mut buf[pos..], cfg.dst_offset_minutes);

    copy(&mut buf, &mut pos, b",\"dst_start_month\":");
    pos += append_u8(&mut buf[pos..], cfg.dst_start_month);

    copy(&mut buf, &mut pos, b",\"dst_end_month\":");
    pos += append_u8(&mut buf[pos..], cfg.dst_end_month);

    copy(&mut buf, &mut pos, b"}");
    buf
}

fn copy(buf: &mut [u8; 512], pos: &mut usize, src: &[u8]) {
    let n = src.len().min(512 - *pos);
    buf[*pos..*pos + n].copy_from_slice(&src[..n]);
    *pos += n;
}

fn append_i32(dst: &mut [u8], v: i32) -> usize {
    // reuse the logic similar to timed state format_i32 but write directly and return len
    if v == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 16];
    let mut n = 0usize;
    let mut val = if v < 0 { (-v) as u32 } else { v as u32 };
    while val > 0 {
        tmp[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    let mut w = 0usize;
    if v < 0 {
        dst[w] = b'-'; w += 1;
    }
    for i in (0..n).rev() {
        dst[w] = tmp[i];
        w += 1;
    }
    w
}

fn append_u8(dst: &mut [u8], v: u8) -> usize {
    if v == 0 {
        dst[0] = b'0'; return 1;
    }
    let mut tmp = [0u8; 4];
    let mut n = 0usize;
    let mut val = v as u32;
    while val > 0 {
        tmp[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    let mut w = 0usize;
    for i in (0..n).rev() {
        dst[w] = tmp[i]; w += 1;
    }
    w
}
