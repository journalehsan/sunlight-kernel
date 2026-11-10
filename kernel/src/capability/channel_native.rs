//! Native admission. SCHEDULER is held across identity/session/borrower checks
//! and broker mutation; no user pointer is read under these locks.
use super::{
    channel::{Error, Handle, Terminal},
    delivery::RecipientIdentity,
    CAP_BROKER,
};
use crate::{process::Process, sched::Scheduler};
use sunlight_ipc::account_session::SessionRef;
impl Scheduler {
    pub(super) fn channel_participant_terminal(
        &self,
        who: RecipientIdentity,
        issuer: bool,
    ) -> Option<Terminal> {
        match self.processes.iter().find(|process| RecipientIdentity::of(process) == who) {
            Some(process) if process.is_exiting() => Some(Terminal::ProcessRetired),
            Some(process) if if issuer { process.trusted_auth_broker } else { process.uid == 0 } => None,
            _ => Some(Terminal::IdentityChanged),
        }
    }

    pub(super) fn channel_process(&self, who: RecipientIdentity) -> Option<&Process> {
        self.processes
            .iter()
            .find(|p| RecipientIdentity::of(p) == who && !p.is_exiting())
    }
    pub(super) fn channel_vm_exclusive(&self, who: RecipientIdentity) -> bool {
        self.channel_process(who).is_some_and(|owner| {
            owner.owns_address_space
                && !owner.is_linux_compat()
                && !self.processes.iter().any(|p| {
                    p.pid != owner.pid
                        && p.address_space.identity() == owner.address_space.identity()
                        && p.state != crate::process::ProcessState::Reaped
                })
        })
    }
    pub(crate) fn fastlane_vm_member(&self) -> bool {
        let mut broker = CAP_BROKER.lock();
        broker.channels.expire(self.global_tick);
        broker
            .channels
            .vm_member(self.current_process().address_space.identity().generation)
    }
    pub(crate) fn fastlane_channel(
        &self,
        op: u64,
        h: Handle,
        arg: u64,
        session: SessionRef,
    ) -> Result<(u64, u64), Error> {
        let current = self.current_process();
        if current.is_exiting() {
            return Err(Error::Denied);
        }
        let who = RecipientIdentity::of(current);
        #[cfg(all(debug_assertions, feature = "fastlane_contract_test"))]
        if matches!(op, 2 | 3) {
            let mut caps = CAP_BROKER.lock();
            caps.channels.row_mut(h)?.contract.fastlane_setup()?;
        }
        let mut broker = CAP_BROKER.lock();
        broker.expire_sessions(self.global_tick);
        broker.channels.expire(self.global_tick);
        // Fence immediately on logical termination, before delayed physical reap.
        // Successful exec additionally invokes retire_recipient_grants before
        // activating the new VM. Failed exec never reaches that hook.
        for row in &mut broker.channels.rows {
            row.retire_unavailable(|who, issuer| self.channel_participant_terminal(who, issuer));
        }
        let result = if op <= 1 {
            let target = current.ipc_reply_target.ok_or(Error::Denied)?.call.pid;
            let recipient = self
                .grant_requester_for_role(target, current.trusted_auth_broker)
                .ok_or(Error::Denied)?;
            let identity = RecipientIdentity::of(recipient);
            let (owner, issuer) = broker
                .account_session_participants(session)
                .ok_or(Error::Denied)?;
            if issuer != who
                || !self
                    .channel_process(owner)
                    .is_some_and(|p| p.trusted_session_service)
                || !self.channel_vm_exclusive(identity)
            {
                return Err(Error::Denied);
            }
            if op == 0 {
                broker
                    .channel_propose(who, identity, session, recipient.uid, self.global_tick, arg)
                    .map(|h| (h.token.0, h.generation))
            } else {
                broker
                    .channel_consent(h, who, identity, session, recipient.uid, self.global_tick)
                    .map(|receipt| (receipt.0, h.generation))
            }
        } else {
            if !self.channel_vm_exclusive(who) {
                return Err(Error::Denied);
            }
            match op {
                2 => broker
                    .channel_create(h, who, super::CapabilityToken(arg), self.global_tick)
                    .map(|_| (0, 0)),
                3 => broker
                    .channel_attach(h, who, arg, self.global_tick)
                    .map(|_| (0, 0)),
                4 => broker.channel_revoke(h, who).map(|_| (0, 0)),
                5 => broker
                    .channel_status(h, who, self.global_tick)
                    .map(|s| (s, 0)),
                // Exact-creator, non-authorizing coordination receipt.
                23 => super::channel_race::observed(&broker, h, who).map(|v| (u64::from(v), 0)),
                #[cfg(all(debug_assertions, feature = "fastlane_lifecycle_trace"))]
                60 => super::lifecycle_trace::arm(&broker,h,who,arg,1).map(|_|(0,0)),
                #[cfg(all(debug_assertions, feature = "fastlane_lifecycle_trace"))]
                63 => super::lifecycle_trace::arm(&broker,h,who,arg,2).map(|_|(0,0)),
                #[cfg(all(debug_assertions, feature = "fastlane_lifecycle_trace"))]
                61 => super::lifecycle_trace::dump(&broker,h,who).map(|v|(v,0)),
                7 => super::channel_race::arm(&broker, h, who, arg).map(|_| (0, 0)),
                // Read-only bounded global inventory, available only in fixture images.
                6 => Ok((
                    broker.channels.live_count() as u64,
                    broker.channels.rows.len() as u64,
                )),
                _ => Err(Error::Denied),
            }
        };
        // Independent identity witness for successful authorization/attachment.
        // All fields are nonsecret lifecycle identities; never print receipts.
        if op <= 3 && result.is_ok() {
            let generation = if op == 0 {
                result.as_ref().unwrap().1
            } else {
                h.generation
            };
            if let Some(row) = broker
                .channels
                .rows
                .iter()
                .find(|r| r.handle.generation == generation)
            {
                let c = row.consumer.unwrap_or(RecipientIdentity {
                    pid: 0,
                    incarnation: 0,
                    vm_generation: 0,
                });
                crate::serial_println!("[FL-BIND] op={} generation={} issuer={} issuer_inc={} issuer_vm={} session={} epoch={} producer={} producer_inc={} producer_vm={} consumer={} consumer_inc={} consumer_vm={} actor={} actor_inc={} actor_vm={} role={} state={} cpu={}",
                    op, generation, row.issuer.pid, row.issuer.incarnation, row.issuer.vm_generation,
                    row.session.id, row.session.epoch, row.creator.pid, row.creator.incarnation,
                    row.creator.vm_generation, c.pid, c.incarnation, c.vm_generation,
                    who.pid, who.incarnation, who.vm_generation, if op == 3 { arg } else { u64::MAX },
                    row.state as u64, crate::sched::current_cpu_id());
            }
        }
        // Never print tokens or secret/random identity material. Generation is
        // a nonsecret boot-local correlation value, not authorization.
        crate::serial_println!(
            "[FL-AUTH] op={} pid={} inc={} vm={} generation={} result={} cpu={}",
            op,
            who.pid,
            who.incarnation,
            who.vm_generation,
            if op == 0 {
                result.as_ref().map_or(0, |r| r.1)
            } else {
                h.generation
            },
            result.as_ref().err().map_or(0, |e| *e as u64),
            crate::sched::current_cpu_id()
        );
        result
    }
}
