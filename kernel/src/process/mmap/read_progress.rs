//! Applies bounded positional replies to a zero-initialized private snapshot.

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReadProgress {
    More,
    Complete,
}

pub(super) fn apply_reply(
    staging: &mut [u8],
    filled: &mut usize,
    requested: usize,
    reported: u64,
    payload: &[u8],
) -> Result<ReadProgress, ()> {
    let available = staging.get_mut(*filled..).ok_or(())?;
    if reported != payload.len() as u64 || payload.len() > requested || requested > available.len()
    {
        return Err(());
    }
    if payload.is_empty() {
        return Ok(ReadProgress::Complete);
    }
    available[..payload.len()].copy_from_slice(payload);
    *filled += payload.len();
    if *filled == staging.len() {
        Ok(ReadProgress::Complete)
    } else {
        Ok(ReadProgress::More)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_reads_continue_and_eof_preserves_zero_padding() {
        let mut staged = [0; 8192];
        let mut filled = 0;
        assert_eq!(
            apply_reply(&mut staged, &mut filled, 4096, 4096, &[17; 4096]),
            Ok(ReadProgress::More)
        );
        assert_eq!(
            apply_reply(&mut staged, &mut filled, 4096, 9, b"tail-data"),
            Ok(ReadProgress::More)
        );
        assert_eq!(
            apply_reply(&mut staged, &mut filled, 4087, 0, b""),
            Ok(ReadProgress::Complete)
        );
        assert_eq!(filled, 4105);
        assert!(staged[..4096].iter().all(|byte| *byte == 17));
        assert_eq!(&staged[4096..4105], b"tail-data");
        assert!(staged[4105..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn full_page_completes_without_an_extra_read() {
        let mut staged = [0; 4096];
        let mut filled = 0;
        assert_eq!(
            apply_reply(&mut staged, &mut filled, 4096, 4096, &[23; 4096]),
            Ok(ReadProgress::Complete)
        );
        assert_eq!(staged, [23; 4096]);
    }

    #[test]
    fn inconsistent_and_oversized_replies_leave_staging_unchanged() {
        let mut staged = [0; 8];
        let mut filled = 4;
        assert_eq!(
            apply_reply(&mut staged, &mut filled, 4, 3, b"1234"),
            Err(())
        );
        assert_eq!(
            apply_reply(&mut staged, &mut filled, 3, 4, b"1234"),
            Err(())
        );
        assert_eq!(
            apply_reply(&mut staged, &mut filled, 5, 4, b"1234"),
            Err(())
        );
        assert_eq!(filled, 4);
        assert_eq!(staged, [0; 8]);
    }
}
