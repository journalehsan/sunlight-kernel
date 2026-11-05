//! IPC/channel lifecycle bridge. Native callers already hold SCHEDULER and
//! CAP_BROKER. No new lock, user target, or independently reusable reply grant.
use super::{channel::Terminal, channel_contract::Phase, CapabilityBroker};
use crate::process::{IpcCallOutcome, IpcReplyTarget};

impl CapabilityBroker {
    pub(crate) fn hybrid_reply_allowed(&self, target: IpcReplyTarget) -> bool {
        self.channels.rows.iter().all(|row| {
            if row.contract.hybrid_reply != Some(target) {
                return true;
            }
            row.live()
                && row.contract.phase == Phase::Ready
                && row.consumer.is_some_and(|peer| {
                    row.contract.policy_identity.is_some_and(|id| {
                        self.channels
                            .policies
                            .resolve(id, peer, row.contract.service)
                            .is_ok()
                    })
                })
        })
    }

    /// Called by the existing IPC terminal decision while both lifecycle
    /// guards remain held. Cancellation also fences the parent channel.
    pub(crate) fn hybrid_reply_finished(
        &mut self,
        target: IpcReplyTarget,
        outcome: IpcCallOutcome,
    ) {
        for row in &mut self.channels.rows {
            if row.contract.hybrid_reply == Some(target) {
                row.contract.hybrid_reply = None;
                if !matches!(outcome, IpcCallOutcome::ReplyDelivered(_)) {
                    row.retire(Terminal::Revoked);
                }
                #[cfg(all(debug_assertions, feature = "fastlane_contract_test"))]
                crate::serial_println!("[FL-HYBRID-K] event=terminal channel={} caller={} ci={} endpoint={} call_generation={} outcome={:?} pending_reply=0", row.handle.generation, target.call.pid, target.call.incarnation, target.endpoint_id, target.call.generation, outcome);
            }
        }
    }
}

#[cfg(all(debug_assertions, feature = "fastlane_contract_test"))]
impl crate::sched::Scheduler {
    /// Bound by the eight existing channel rows. Retired reply metadata is
    /// drained independently of whether restricted backing was ever mapped.
    pub(crate) fn fastlane_hybrid_sweep(&mut self, caps: &mut CapabilityBroker) {
        caps.expire_sessions(self.global_tick);
        caps.channels.expire(self.global_tick);
        for index in 0..caps.channels.rows.len() {
            let Some(target) = caps.channels.rows[index].contract.hybrid_reply else {
                continue;
            };
            if caps.hybrid_reply_allowed(target) {
                continue;
            }
            if !crate::ipc::reply_target_is_live(target, self) {
                crate::serial_println!("[FL-HYBRID-K] event=retired channel={} caller={} ci={} endpoint={} call_generation={} outcome=CallerRetired pending_reply=0",caps.channels.rows[index].handle.generation,target.call.pid,target.call.incarnation,target.endpoint_id,target.call.generation);
            }
            crate::ipc::withdraw_hybrid_reply(target, self, caps);
            // A caller exit/exec may already have withdrawn its pending call.
            caps.channels.rows[index].contract.hybrid_reply = None;
        }
    }
}
