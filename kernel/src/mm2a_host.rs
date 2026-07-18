#![no_std]

#[path = "process/mm2a_plan.rs"]
pub mod mm2a_plan;

#[path = "process/mm2b_state.rs"]
pub mod mm2b_state;

#[cfg(test)]
mod tests {
    use super::mm2a_plan::{checked_page_layout, DeferredCursor, PlanError};

    #[test]
    fn checked_page_layout_rejects_zero_and_overflow() {
        assert_eq!(checked_page_layout(0), Err(PlanError::ZeroLength));
        assert_eq!(checked_page_layout(u64::MAX), Err(PlanError::Overflow));
        assert_eq!(checked_page_layout(1), Ok((1, 4096)));
        assert_eq!(checked_page_layout(4097), Ok((2, 8192)));
    }

    #[test]
    fn cursor_changes_only_at_commit() {
        let mut cursor = 0;
        let transaction = DeferredCursor::new(cursor, 0x10_0000_0000, 8192).unwrap();
        assert_eq!(transaction.base(), 0x10_0000_0000);

        // A failed allocation/map path drops the transaction without publishing.
        assert_eq!(cursor, 0);

        transaction.commit(&mut cursor);
        assert_eq!(cursor, 0x10_0000_2000);
    }

    #[test]
    fn cursor_overflow_fails_without_mutation() {
        let cursor = u64::MAX - 4095;
        assert_eq!(
            DeferredCursor::new(cursor, 0, 8192),
            Err(PlanError::Overflow)
        );
        assert_eq!(cursor, u64::MAX - 4095);
    }
}
