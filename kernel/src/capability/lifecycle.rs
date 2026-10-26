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
use sunlight_ipc::accounts::GrantBinding;
use sunlight_ipc::accounts::GrantScope;

impl Scheduler {
    /// Test-only observation, with the same authenticated request lookup as
    /// account_grant. This neither reserves metadata nor authorizes a mint.
    #[cfg(feature = "native_session_race_test")]
    pub(crate) fn observe_n4_pending(&self, pid: usize, session: u64, epoch: u64) -> bool {
        let Some(recipient) = self.grant_requester(pid) else {
            return false;
        };
        let Some(delivery) = self.grant_delivery(recipient) else {
            return false;
        };
        let broker = CAP_BROKER.lock();
        let reference = sunlight_ipc::account_session::SessionRef { id: session, epoch };
        let Some((owner, issuer)) = broker.account_session_participants(reference) else {
            return false;
        };
        if issuer != RecipientIdentity::of(self.current_process())
            || !self.session_participant_live(owner, false)
            || !self.session_participant_live(issuer, true)
        {
            return false;
        }
        super::session_race::locked_event(
            "authenticated-requester",
            recipient,
            session,
            epoch,
            u64::from(recipient.uid),
        );
        super::session_race::locked_event(
            "n4-precommit",
            self.current_process(),
            session,
            epoch,
            0,
        );
        crate::serial_println!("[SESSION-E3] seq={} checkpoint=request-binding recipient={} recipient_inc={} recipient_vm={} binding_vm={} issuer={} issuer_inc={} issuer_vm={} request_generation={} session={} epoch={} cpu={}", super::session_race::sequence(), delivery.recipient.pid, delivery.recipient.incarnation, delivery.recipient.vm_generation, recipient.address_space.identity().generation, delivery.issuer_pid, delivery.issuer_incarnation, delivery.issuer_vm_generation, delivery.request, session, epoch, crate::sched::current_cpu_id());
        super::session_race::inventory(&broker);
        true
    }

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

    fn grant_requester_for_role(&self, pid: usize, trusted_role: bool) -> Option<&Process> {
        let issuer = self.current_process();
        if !trusted_role
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

    fn grant_requester(&self, pid: usize) -> Option<&Process> {
        self.grant_requester_for_role(pid, self.current_process().trusted_auth_broker)
    }

    fn grant_delivery(&self, recipient: &Process) -> Option<GrantDelivery> {
        let issuer = self.current_process();
        Some(GrantDelivery {
            recipient: RecipientIdentity::of(recipient),
            issuer_pid: issuer.pid,
            issuer_incarnation: issuer.foreground.incarnation,
            issuer_vm_generation: issuer.address_space.identity().generation,
            request: issuer.ipc_reply_target?.call.generation,
            delivered: false,
        })
    }

    /// No payload recipient: only this service's actual synchronous IPC caller.
    #[cfg(test)]
    pub(crate) fn delegate_vfs_grants(
        &self,
        proof: CapabilityToken,
        path: Option<&str>,
    ) -> Option<heapless::Vec<CapabilityToken, 2>> {
        self.try_delegate_vfs_grants(proof, path).ok()
    }

    pub(crate) fn try_delegate_vfs_grants(
        &self,
        proof: CapabilityToken,
        path: Option<&str>,
    ) -> Result<heapless::Vec<CapabilityToken, 2>, super::CapError> {
        let pid = self
            .current_process()
            .ipc_reply_target
            .ok_or(super::CapError::InvalidCaller)?
            .call
            .pid;
        let recipient = self
            .grant_requester(pid)
            .ok_or(super::CapError::InvalidCaller)?;
        let delivery = self
            .grant_delivery(recipient)
            .ok_or(super::CapError::InvalidCaller)?;
        #[cfg(test)]
        super::lifecycle_tests::before_commit();
        let tokens =
            CAP_BROKER
                .lock()
                .delegate_vfs_grants(proof, delivery, path, self.global_tick)?;
        #[cfg(feature = "production_grant_test")]
        super::production_gate::committed(self.current_process(), delivery, &tokens);
        Ok(tokens)
    }

    pub(crate) fn consume_vfs_grant(&self, token: CapabilityToken, path: &str) -> bool {
        let issuer = self.current_process();
        let Some(target) = issuer.ipc_reply_target else {
            return false;
        };
        let Some(recipient) =
            self.grant_requester_for_role(target.call.pid, issuer.trusted_vfs_service)
        else {
            return false;
        };
        let identity = RecipientIdentity::of(recipient);
        let allowed = CAP_BROKER
            .lock()
            .consume_vfs_grant(token, identity, path, self.global_tick);
        #[cfg(feature = "production_grant_test")]
        super::production_gate::consumed(issuer, identity, allowed);
        allowed
    }

    pub(crate) fn cancel_vfs_grant(&self, token: CapabilityToken) {
        let process = self.current_process();
        let mut caps = CAP_BROKER.lock();
        #[cfg(feature = "production_grant_test")]
        let before = caps.recipient_grant_count(process.pid);
        caps.cancel_vfs_grant(token, RecipientIdentity::of(process));
        #[cfg(feature = "production_grant_test")]
        super::production_gate::owner_cancelled(process.pid, before, &caps);
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
        #[cfg(feature = "native_session_race_test")]
        super::session_race::locked_event(
            if token.is_some() {
                "login-proof-committed"
            } else {
                "login-proof-rejected"
            },
            self.current_process(),
            0,
            0,
            0,
        );
        #[cfg(feature = "cap_issuance_test")]
        if token.is_some() {
            super::gate::AUTH_COMMITS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        token
    }

    /// Authenticate all participants while lifecycle serialization remains held.
    fn session_participant_live(&self, identity: RecipientIdentity, account: bool) -> bool {
        self.processes.iter().any(|p| {
            RecipientIdentity::of(p) == identity
                && !p.is_exiting()
                && if account {
                    p.trusted_auth_broker
                } else {
                    p.trusted_session_service
                }
        })
    }

    pub(crate) fn register_account_session(
        &self,
        proof: CapabilityToken,
        pid: usize,
        deadline: u64,
    ) -> Result<(sunlight_ipc::account_session::SessionRef, u32, u32), sunlight_ipc::accounts::Error>
    {
        use sunlight_ipc::accounts::Error;
        let owner = self.current_process();
        let recipient = self
            .grant_requester_for_role(pid, owner.trusted_session_service)
            .ok_or(Error::Denied)?;
        if !recipient.trusted_tty_session_service {
            return Err(Error::Denied);
        }
        let request = self.grant_delivery(recipient).ok_or(Error::Denied)?;
        let mut broker = CAP_BROKER.lock();
        #[cfg(feature = "native_session_race_test")]
        super::session_race::inventory(&broker);
        // Retry after a successful register is reconciled by the saved call,
        // even though the proof was consumed. No payload selects an issuer.
        let issuer = broker
            .registration_issuer(RecipientIdentity::of(owner), request)
            .or_else(|| {
                broker
                    .auth_session_grants
                    .iter()
                    .find(|g| g.token == proof)
                    .and_then(|g| g.delivery)
                    .and_then(|d| {
                        self.processes.iter().find(|p| {
                            p.pid == d.issuer_pid
                                && p.foreground.incarnation == d.issuer_incarnation
                                && p.address_space.identity().generation == d.issuer_vm_generation
                        })
                    })
                    .map(RecipientIdentity::of)
            })
            .filter(|i| self.session_participant_live(*i, true))
            .ok_or(Error::Denied)?;
        let result = broker.register_account_session(
            RecipientIdentity::of(owner),
            issuer,
            request,
            proof,
            self.global_tick,
            deadline,
        );
        #[cfg(feature = "native_session_race_test")]
        match result {
            Ok((reference, _, _)) => super::session_race::locked_event(
                "registered",
                owner,
                reference.id,
                reference.epoch,
                0,
            ),
            Err(error) => super::session_race::locked_event(
                "registration-rejected",
                owner,
                0,
                0,
                error as u64,
            ),
        }
        result
    }

    pub(crate) fn transition_account_session(
        &self,
        reference: sunlight_ipc::account_session::SessionRef,
        operation: u64,
    ) -> Result<(), sunlight_ipc::accounts::Error> {
        use sunlight_ipc::accounts::Error;
        let owner = self.current_process();
        if !owner.trusted_session_service
            || owner.is_exiting()
            || owner.foreground.incarnation == 0
            || owner.address_space.identity().generation == 0
        {
            return Err(Error::Denied);
        }
        let mut broker = CAP_BROKER.lock();
        if operation == sunlight_ipc::account_session::ACTIVATE {
            let (_, issuer) = broker
                .account_session_participants(reference)
                .ok_or(Error::SessionChanged)?;
            if !self.session_participant_live(issuer, true) {
                return Err(Error::SessionChanged);
            }
        }
        #[cfg(feature = "native_session_race_test")]
        let existed = broker.account_session_participants(reference).is_some();
        let result = broker.transition_account_session(
            RecipientIdentity::of(owner),
            reference,
            operation,
            self.global_tick,
        );
        #[cfg(feature = "native_session_race_test")]
        super::session_race::locked_event(
            if result.is_ok() && existed {
                "transition-committed"
            } else if result.is_ok() {
                "transition-idempotent"
            } else {
                "transition-rejected"
            },
            owner,
            reference.id,
            reference.epoch,
            operation,
        );
        #[cfg(feature = "native_session_race_test")]
        if result.is_ok() && existed && operation == sunlight_ipc::account_session::REVOKE {
            super::session_race::committed(reference.id, reference.epoch, 2);
        }
        #[cfg(feature = "native_session_race_test")]
        if operation == sunlight_ipc::account_session::REVOKE {
            super::session_race::inventory(&broker);
        }
        result
    }

    pub(crate) fn account_grant(
        &self,
        pid: usize,
        scope: GrantScope,
        consume: Option<CapabilityToken>,
    ) -> Result<CapabilityToken, sunlight_ipc::accounts::Error> {
        use sunlight_ipc::accounts::Error;
        let recipient = self.grant_requester(pid).ok_or(Error::Denied)?;
        let delivery = self.grant_delivery(recipient).ok_or(Error::Denied)?;
        #[cfg(feature = "native_session_race_test")]
        super::session_race::locked_event(
            "authenticated-requester",
            recipient,
            scope.session,
            scope.session_generation,
            u64::from(recipient.uid),
        );
        #[cfg(feature = "native_session_race_test")]
        crate::serial_println!("[SESSION-E3] seq={} checkpoint=request-binding recipient={} recipient_inc={} recipient_vm={} binding_vm={} issuer={} issuer_inc={} issuer_vm={} request_generation={} session={} epoch={} cpu={}", super::session_race::sequence(), delivery.recipient.pid, delivery.recipient.incarnation, delivery.recipient.vm_generation, recipient.address_space.identity().generation, delivery.issuer_pid, delivery.issuer_incarnation, delivery.issuer_vm_generation, delivery.request, scope.session, scope.session_generation, crate::sched::current_cpu_id());
        let reference = sunlight_ipc::account_session::SessionRef {
            id: scope.session,
            epoch: scope.session_generation,
        };
        let mut broker = CAP_BROKER.lock();
        #[cfg(feature = "native_session_race_test")]
        super::session_race::inventory(&broker);
        let (owner, issuer) = broker
            .account_session_participants(reference)
            .ok_or(Error::SessionAuthorityUnavailable)?;
        if !self.session_participant_live(owner, false)
            || !self.session_participant_live(issuer, true)
        {
            broker.retire_sessions_for(if !self.session_participant_live(owner, false) {
                owner.pid
            } else {
                issuer.pid
            });
            return Err(Error::SessionChanged);
        }
        let result = broker.commit_account_session_grant(
            RecipientIdentity::of(self.current_process()),
            delivery,
            recipient.uid,
            GrantBinding {
                pid: recipient.pid as u64,
                process_generation: recipient.address_space.identity().generation,
                scope,
            },
            consume,
            self.global_tick,
        );
        #[cfg(feature = "native_session_race_test")]
        super::session_race::locked_event(
            match (consume.is_some(), result.is_ok()) {
                (true, true) => "consume-committed",
                (true, false) => "consume-rejected",
                (false, true) => "mint-committed",
                (false, false) => "mint-rejected",
            },
            self.current_process(),
            reference.id,
            reference.epoch,
            result.err().map_or(0, |e| e as u64),
        );
        #[cfg(feature = "native_session_race_test")]
        if let Ok(token) = result {
            super::session_race::grant(token, reference.id, reference.epoch, consume.is_some());
        }
        #[cfg(feature = "native_session_race_test")]
        super::session_race::inventory(&broker);
        #[cfg(feature = "native_session_race_test")]
        super::session_race::committed(
            reference.id,
            reference.epoch,
            if consume.is_some() { 4 } else { 1 },
        );
        result
    }

    #[cfg(test)]
    pub(super) fn grant_delivery_for_test(&self) -> GrantDelivery {
        self.grant_delivery(&self.processes[1]).unwrap()
    }

    /// Historical delivery/retirement mechanics only. This deliberately omits
    /// session authorization and is absent from every native/production build.
    /// Tests using it are not evidence of Account issuance authorization.
    #[cfg(test)]
    pub(super) fn unchecked_account_grant_for_delivery_test(
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
                token
            }
        }
    }
}
