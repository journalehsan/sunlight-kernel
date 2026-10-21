//! State carried by an existing recipient grant, never a second authority table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecipientIdentity {
    pub pid: usize,
    pub incarnation: u64,
    pub vm_generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GrantDelivery {
    pub recipient: RecipientIdentity,
    pub issuer_pid: usize,
    pub issuer_incarnation: u64,
    pub request: u64,
    pub delivered: bool,
}
impl GrantDelivery {
    pub fn matches(&self, pid: usize, incarnation: u64, request: u64) -> bool {
        self.recipient.pid == pid
            && self.recipient.incarnation == incarnation
            && self.request == request
    }
    pub fn usable(&self, recipient: RecipientIdentity) -> bool {
        self.delivered && self.recipient == recipient
    }
}
