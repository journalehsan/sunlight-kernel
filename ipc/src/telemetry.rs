//! Append-only shared telemetry ABI extension, introduced in page version 5.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ProcessMetricsV5 {
    /// Executable basename; never inherited from a parent for attribution.
    pub app_id: [u8; 32],
    pub private_bytes: u64,
    pub shared_bytes: u64,
    /// Present user virtual mappings, including aliases, in bytes.
    pub mapped_bytes: u64,
    pub uptime_secs: u32,
    pub threads: u32,
}
impl ProcessMetricsV5 {
    pub const ZERO: Self = Self {
        app_id: [0; 32],
        private_bytes: 0,
        shared_bytes: 0,
        mapped_bytes: 0,
        uptime_secs: 0,
        threads: 0,
    };
}
/// Resolve only the supplied executable/command, never a parent name.
pub fn executable_name(path: &[u8]) -> [u8; 32] {
    let end = path.iter().position(|b| *b == 0).unwrap_or(path.len());
    let path = &path[..end];
    let start = path.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1);
    let mut result = [0; 32];
    let n = (end - start).min(32);
    result[..n].copy_from_slice(&path[start..start + n]);
    result
}
/// Count a physical frame once per address space, after alias deduplication.
pub fn frame_bytes(owners: usize) -> (u64, u64) {
    if owners == 1 {
        (4096, 0)
    } else {
        (0, 4096)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_use_own_command() {
        assert_eq!(
            &executable_name(b"/bin/pdf-viewer\0ignored")[..10],
            b"pdf-viewer"
        );
        assert_ne!(
            executable_name(b"melody-mina"),
            executable_name(b"pdf-viewer")
        );
        assert_eq!(executable_name(b""), [0; 32]);
        assert_eq!(executable_name(&[b'x'; 40]), [b'x'; 32]);
    }
    #[test]
    fn physical_frames_are_not_proportionally_charged() {
        assert_eq!(frame_bytes(1), (4096, 0));
        assert_eq!(frame_bytes(2), (0, 4096));
        assert_eq!(frame_bytes(50), (0, 4096));
    }
}

/// Stable consumer-facing process record assembled from a coherent v5 page.
/// cpu_percent is basis points: 10000 means 100.00% of machine capacity.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ProcessRecordV1 {
    pub version: u32,
    pub pid: u32,
    pub ppid: u32,
    pub state: u32,
    pub app_id: [u8; 32],
    pub name: [u8; 32],
    pub cpu_percent: u32,
    pub threads: u32,
    pub private_bytes: u64,
    pub shared_bytes: u64,
    pub mapped_bytes: u64,
    pub uptime: u64,
}

/// Physical counts for sorted frame addresses. Aliases count once. Private
/// requires both exclusive process ownership and the UserPrivate PMM class.
pub fn unique_memory_bytes(
    frames: &[u64],
    mut ownership: impl FnMut(u64) -> (usize, bool),
) -> (u64, u64) {
    let mut previous = None;
    let mut private = 0u64;
    let mut shared = 0u64;
    for frame in frames {
        if previous == Some(*frame) {
            continue;
        }
        previous = Some(*frame);
        let (owners, user_private) = ownership(*frame);
        if owners == 1 && user_private {
            private += 4096;
        } else {
            shared += 4096;
        }
    }
    (private, shared)
}
#[cfg(test)]
mod memory_tests {
    use super::*;
    #[test]
    fn aliases_shared_backing_and_unique_frames() {
        let frames = [1, 1, 2, 3, 3, 4];
        assert_eq!(
            unique_memory_bytes(&frames, |f| match f {
                1 => (1, true),
                2 => (2, true),
                3 => (1, false),
                _ => (1, true),
            }),
            (8192, 8192)
        );
        assert_eq!(unique_memory_bytes(&[], |_| (1, true)), (0, 0));
        // A sibling exits: its shared COW frame becomes private next sample.
        assert_eq!(unique_memory_bytes(&[2], |_| (1, true)), (4096, 0));
    }
}

const _: () = assert!(core::mem::size_of::<ProcessMetricsV5>() == 64);
const _: () = assert!(core::mem::size_of::<ProcessRecordV1>() == 120);
