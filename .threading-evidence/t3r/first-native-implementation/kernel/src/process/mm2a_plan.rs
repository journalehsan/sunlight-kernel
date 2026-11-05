#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    ZeroLength,
    Overflow,
}

pub fn checked_page_layout(length: u64) -> Result<(u64, u64), PlanError> {
    if length == 0 {
        return Err(PlanError::ZeroLength);
    }
    let rounded = length.checked_add(4095).ok_or(PlanError::Overflow)?;
    let page_count = rounded / 4096;
    let span = page_count.checked_mul(4096).ok_or(PlanError::Overflow)?;
    Ok((page_count, span))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferredCursor {
    base: u64,
    end: u64,
}

impl DeferredCursor {
    pub fn new(current: u64, floor: u64, span: u64) -> Result<Self, PlanError> {
        let base = current.max(floor);
        let end = base.checked_add(span).ok_or(PlanError::Overflow)?;
        Ok(Self { base, end })
    }

    pub fn base(self) -> u64 {
        self.base
    }

    pub fn commit(self, cursor: &mut u64) {
        *cursor = self.end;
    }
}
