//! Recipient-bound issuance. The caller must own SCHEDULER (or an unpublished
//! Scheduler in tests). Borrowing it for this entire operation serializes the
//! final authorization, broker commit and recipient retirement. Never return a
//! validated PID/binding for a caller to mint after releasing that borrow.
use super::delivery::{GrantDelivery, RecipientIdentity};
use super::{CapabilityToken, CAP_BROKER};

impl RecipientIdentity {
    pub(crate) fn of(process: &Process) -> Self {
        Self {
            pid: process.pid,
            incarnation: process.foreground.incarnation,
            vm_generation: process.address_space.identity().generation,
        }
    }
}
use crate::process::Process;
use crate::sched::Scheduler;
use sunlight_ipc::accounts::{GrantBinding, GrantScope};

impl Scheduler {
    /// Legacy VFS minting is a bearer issuance to the current caller, not a
    /// recipient-targeted grant. Keep the old slot restriction without treating
    /// that slot as proof of authority. The launch-chain role is withdrawn by
    /// successful exec. This borrow must span validation and broker insertion.
    pub(crate) fn issue_vfs_capability(
        &self,
        cap: super::VfsCapability,
    ) -> Result<CapabilityToken, super::CapError> {
        let issuer = self.current_process();
        if issuer.pid != super::CAPABILITY_BROKER_PID as usize
            || !issuer.trusted_auth_broker
            || issuer.is_exiting()
            || issuer.foreground.incarnation == 0
            || issuer.address_space.identity().generation == 0
        {
            return Err(super::CapError::InvalidCaller);
        }
        #[cfg(test)]
        super::lifecycle_tests::before_commit();
        // SCHEDULER -> CAP_BROKER. No IPC or userspace copy under these locks.
        CAP_BROKER.lock().grant_vfs_capability(issuer.pid, cap)
    }

    fn grant_requester(&self, pid: usize) -> Option<&Process> {
        let issuer = self.current_process();
        if !issuer.trusted_auth_broker
            || issuer.is_exiting()
            || issuer.foreground.incarnation == 0
            || issuer.address_space.identity().generation == 0
        {
            return None;
        }
        let recipient = self.processes.iter().find(|p| p.pid == pid)?;
        if recipient.is_exiting()
            || recipient.foreground.incarnation == 0
            || recipient.address_space.identity().generation == 0
            || !crate::ipc::reply_target_matches_pending_request(
                pid,
                recipient.foreground.incarnation,
                recipient.pending_call,
                issuer.ipc_reply_target,
            )
        {
            return None;
        }
        Some(recipient)
    }

    fn grant_delivery(&self, recipient: &Process) -> Option<GrantDelivery> {
        let issuer = self.current_process();
        Some(GrantDelivery {
            recipient: RecipientIdentity::of(recipient),
            issuer_pid: issuer.pid,
            issuer_incarnation: issuer.foreground.incarnation,
            request: issuer.ipc_reply_target?.call.generation,
            delivered: false,
        })
    }

    pub(crate) fn issue_auth_session_grant(
        &self,
        pid: usize,
        uid: u32,
        gid: u32,
    ) -> Option<CapabilityToken> {
        let recipient = self.grant_requester(pid)?;
        let delivery = self.grant_delivery(recipient)?;
        #[cfg(test)]
        super::lifecycle_tests::before_commit();
        // SCHEDULER -> CAP_BROKER -> conditioned entropy. No source collection,
        // IPC, userspace access or VM work occurs here. Insertion linearizes.
        let mut broker = CAP_BROKER.lock();
        let token = broker.mint_auth_session_grant(
            recipient.pid,
            uid,
            gid,
            self.global_tick.saturating_add(500),
        );
        if token.is_some() {
            broker.auth_session_grants.last_mut()?.delivery = Some(delivery);
        }
        #[cfg(feature = "cap_issuance_test")]
        if token.is_some() {
            super::gate::AUTH_COMMITS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        token
    }

    pub(crate) fn account_grant(
        &self,
        pid: usize,
        scope: GrantScope,
        consume: Option<CapabilityToken>,
    ) -> Option<CapabilityToken> {
        let recipient = self.grant_requester(pid)?;
        let delivery = self.grant_delivery(recipient)?;
        let binding = GrantBinding {
            pid: recipient.pid as u64,
            process_generation: recipient.address_space.identity().generation,
            scope,
        };
        #[cfg(test)]
        super::lifecycle_tests::before_commit();
        let mut broker = CAP_BROKER.lock();
        match consume {
            Some(token) => broker
                .consume_live_account_grant(token, delivery, binding, self.global_tick)
                .then_some(token),
            None => {
                let token = broker.mint_account_grant(binding, self.global_tick);
                if token.is_some() {
                    broker.account_grants.last_mut()?.1.delivery = Some(delivery);
                }
                #[cfg(feature = "cap_issuance_test")]
                if token.is_some() {
                    super::gate::ACCOUNT_COMMITS
                        .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                token
            }
        }
    }
}
