//! Raw Linux sched_getaffinity mask sizing for up to 64 Helios CPUs.

pub const MASK_BYTES: usize = 8;

/// Linux returns the number of mask bytes actually copied, not libc's zero.
pub fn mask_for_online_cores(
    online: usize,
    cpusetsize: usize,
) -> Option<([u8; MASK_BYTES], usize)> {
    if cpusetsize < MASK_BYTES || cpusetsize % core::mem::size_of::<u64>() != 0 {
        return None;
    }
    let mask = match online {
        0 => return None,
        64.. => u64::MAX,
        count => (1u64 << count) - 1,
    };
    Some((mask.to_ne_bytes(), MASK_BYTES))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_online_cpu_mask_and_raw_return_size() {
        let (bytes, copied) = mask_for_online_cores(4, 128).unwrap();
        assert_eq!(u64::from_ne_bytes(bytes), 0x0f);
        assert_eq!(copied, 8);
        assert_eq!(
            u64::from_ne_bytes(mask_for_online_cores(64, 8).unwrap().0),
            u64::MAX
        );
    }

    #[test]
    fn rejects_small_or_misaligned_buffer() {
        assert!(mask_for_online_cores(1, 4).is_none());
        assert!(mask_for_online_cores(1, 9).is_none());
        assert!(mask_for_online_cores(0, 128).is_none());
    }
}
