//! Lightweight launch tracing helpers shared by launchers, GUI apps, and the
//! display server.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::{debug_log, monotonic_millis};

const ARG_PREFIX: &[u8] = b"--sunlight-launch=";

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchSource {
    Unknown = 0,
    Dock = 1,
    Runner = 2,
    Shortcut = 3,
    Boot = 4,
    Shell = 5,
}

impl LaunchSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Dock => "dock",
            Self::Runner => "runner",
            Self::Shortcut => "shortcut",
            Self::Boot => "boot",
            Self::Shell => "shell",
        }
    }

    pub fn parse(bytes: &[u8]) -> Self {
        match bytes {
            b"dock" => Self::Dock,
            b"runner" => Self::Runner,
            b"shortcut" => Self::Shortcut,
            b"boot" => Self::Boot,
            b"shell" => Self::Shell,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchTrace {
    pub launch_id: u64,
    pub source: LaunchSource,
    pub requested_at_ms: u64,
}

impl LaunchTrace {
    pub const fn new(launch_id: u64, source: LaunchSource, requested_at_ms: u64) -> Self {
        Self {
            launch_id,
            source,
            requested_at_ms,
        }
    }

    pub fn is_active(self) -> bool {
        self.launch_id != 0
    }
}

static CURRENT_LAUNCH_ID: AtomicU64 = AtomicU64::new(0);
static CURRENT_SOURCE: AtomicU8 = AtomicU8::new(LaunchSource::Unknown as u8);
static CURRENT_REQUESTED_AT_MS: AtomicU64 = AtomicU64::new(0);

pub fn current() -> Option<LaunchTrace> {
    let launch_id = CURRENT_LAUNCH_ID.load(Ordering::Relaxed);
    if launch_id == 0 {
        return None;
    }
    Some(LaunchTrace {
        launch_id,
        source: LaunchSource::parse_source_tag(CURRENT_SOURCE.load(Ordering::Relaxed)),
        requested_at_ms: CURRENT_REQUESTED_AT_MS.load(Ordering::Relaxed),
    })
}

pub fn set_current(trace: LaunchTrace) {
    CURRENT_LAUNCH_ID.store(trace.launch_id, Ordering::Relaxed);
    CURRENT_SOURCE.store(trace.source as u8, Ordering::Relaxed);
    CURRENT_REQUESTED_AT_MS.store(trace.requested_at_ms, Ordering::Relaxed);
}

pub fn clear_current() {
    set_current(LaunchTrace::new(0, LaunchSource::Unknown, 0));
}

impl LaunchSource {
    fn parse_source_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Dock,
            2 => Self::Runner,
            3 => Self::Shortcut,
            4 => Self::Boot,
            5 => Self::Shell,
            _ => Self::Unknown,
        }
    }
}

pub fn parse_launch_arg(bytes: &[u8]) -> Option<LaunchTrace> {
    if !bytes.starts_with(ARG_PREFIX) {
        return None;
    }
    let rest = &bytes[ARG_PREFIX.len()..];
    let (id_bytes, rest) = split_once(rest, b':')?;
    let (source_bytes, requested_at_bytes) = split_once(rest, b':')?;
    let launch_id = parse_u64(id_bytes)?;
    let source = LaunchSource::parse(source_bytes);
    let requested_at_ms = parse_u64(requested_at_bytes)?;
    Some(LaunchTrace::new(launch_id, source, requested_at_ms))
}

pub fn format_launch_arg(trace: LaunchTrace, out: &mut [u8]) -> Option<usize> {
    let mut len = 0usize;
    copy_bytes(ARG_PREFIX, out, &mut len)?;
    append_u64(trace.launch_id, out, &mut len)?;
    append_byte(b':', out, &mut len)?;
    copy_bytes(trace.source.as_str().as_bytes(), out, &mut len)?;
    append_byte(b':', out, &mut len)?;
    append_u64(trace.requested_at_ms, out, &mut len)?;
    Some(len)
}

pub fn log_phase(
    trace: LaunchTrace,
    subject: &str,
    phase: &str,
    pid: Option<u64>,
    timestamp_ms: u64,
) {
    let mut bytes = [0u8; 256];
    let len = format_phase(&mut bytes, trace, subject, phase, pid, timestamp_ms);
    // One bounded syscall keeps a record together and avoids dozens of
    // scheduler transitions and serial-lock acquisitions per launch phase.
    debug_log(core::str::from_utf8(&bytes[..len]).unwrap_or("launch: invalid trace"));
}

fn format_phase(
    bytes: &mut [u8; 256],
    trace: LaunchTrace,
    subject: &str,
    phase: &str,
    pid: Option<u64>,
    timestamp_ms: u64,
) -> usize {
    use core::fmt::Write;
    struct Line<'a> {
        bytes: &'a mut [u8],
        len: usize,
    }
    impl core::fmt::Write for Line<'_> {
        fn write_str(&mut self, value: &str) -> core::fmt::Result {
            copy_bytes(value.as_bytes(), self.bytes, &mut self.len).ok_or(core::fmt::Error)
        }
    }
    fn bounded(value: &str) -> &str {
        let mut end = value.len().min(48);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        &value[..end]
    }
    let mut line = Line { bytes, len: 0 };
    let _ = write!(
        line,
        "launch[{}] source={} {} phase={} timestamp_ms={}",
        trace.launch_id,
        trace.source.as_str(),
        bounded(subject),
        bounded(phase),
        timestamp_ms
    );
    if trace.requested_at_ms != 0 {
        let _ = write!(
            line,
            " delta={}ms",
            timestamp_ms.saturating_sub(trace.requested_at_ms)
        );
    }
    if let Some(pid) = pid {
        let _ = write!(line, " pid={}", pid);
    }
    let _ = line.write_str("\n");
    line.len
}

pub fn log_phase_now(trace: LaunchTrace, subject: &str, phase: &str, pid: Option<u64>) {
    log_phase(trace, subject, phase, pid, monotonic_millis());
}

fn split_once<'a>(bytes: &'a [u8], needle: u8) -> Option<(&'a [u8], &'a [u8])> {
    let idx = bytes.iter().position(|&b| b == needle)?;
    Some((&bytes[..idx], &bytes[idx + 1..]))
}

fn parse_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(value)
}

fn copy_bytes(src: &[u8], dst: &mut [u8], len: &mut usize) -> Option<()> {
    let end = (*len).checked_add(src.len())?;
    if end > dst.len() {
        return None;
    }
    dst[*len..end].copy_from_slice(src);
    *len = end;
    Some(())
}

fn append_byte(byte: u8, dst: &mut [u8], len: &mut usize) -> Option<()> {
    let end = (*len).checked_add(1)?;
    if end > dst.len() {
        return None;
    }
    dst[*len] = byte;
    *len = end;
    Some(())
}

fn append_u64(mut value: u64, dst: &mut [u8], len: &mut usize) -> Option<()> {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    copy_bytes(&buf[i..], dst, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_log_is_one_complete_bounded_record() {
        let mut bytes = [0; 256];
        let len = format_phase(
            &mut bytes,
            LaunchTrace::new(7, LaunchSource::Dock, 100),
            "Terminal",
            "window_registered",
            Some(42),
            150,
        );
        assert_eq!(core::str::from_utf8(&bytes[..len]).unwrap(),
            "launch[7] source=dock Terminal phase=window_registered timestamp_ms=150 delta=50ms pid=42\n");
        let long = "🌞".repeat(80);
        let len = format_phase(
            &mut bytes,
            LaunchTrace::new(u64::MAX, LaunchSource::Shortcut, 1),
            &long,
            &long,
            Some(u64::MAX),
            u64::MAX,
        );
        assert!(len <= 256);
        assert!(core::str::from_utf8(&bytes[..len]).unwrap().ends_with('\n'));
    }
}
