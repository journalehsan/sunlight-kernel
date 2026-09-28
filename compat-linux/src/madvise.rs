//! Checked x86-64 madvise request layout; VM effects are owned by the kernel.

pub const MADV_DONTNEED: u32 = 4;
pub const MADV_FREE: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdviceError {
    Invalid,
    Overflow,
}

pub fn checked_range(addr: u64, len: u64, advice: u32) -> Result<(u64, u64), AdviceError> {
    if advice != MADV_DONTNEED && advice != MADV_FREE || addr & 4095 != 0 {
        return Err(AdviceError::Invalid);
    }
    let rounded_len = len.checked_add(4095).ok_or(AdviceError::Overflow)? & !4095;
    let end = addr.checked_add(rounded_len).ok_or(AdviceError::Overflow)?;
    Ok((addr, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_advice_and_rounding() {
        assert_eq!(
            checked_range(0x1000, 1, MADV_DONTNEED),
            Ok((0x1000, 0x2000))
        );
        assert_eq!(checked_range(0x2000, 0, MADV_FREE), Ok((0x2000, 0x2000)));
    }

    #[test]
    fn rejects_misalignment_unknown_advice_and_overflow() {
        assert_eq!(
            checked_range(0x1001, 4096, MADV_FREE),
            Err(AdviceError::Invalid)
        );
        assert_eq!(checked_range(0x1000, 4096, 123), Err(AdviceError::Invalid));
        assert_eq!(
            checked_range(0x1000, u64::MAX, MADV_FREE),
            Err(AdviceError::Overflow)
        );
    }
}
