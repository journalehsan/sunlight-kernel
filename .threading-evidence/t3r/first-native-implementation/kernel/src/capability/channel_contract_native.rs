use super::{
    channel::{Channel, Error, Handle, State, Terminal},
    channel_contract::{Phase, Transport},
    delivery::RecipientIdentity,
    CapabilityToken, CAP_BROKER,
};
use crate::sched::Scheduler;
use sunlight_ipc::account_session::SessionRef;

impl Channel {
    fn contract_note(&self, event: &str, result: u64, reason: &str) {
        let service = self.consumer.unwrap_or(self.creator);
        crate::serial_println!("[FL-M61B-K] event={} g={} service={} si={} sv={} caller={} ci={} cv={} policy={} policy_id={} policy_generation={} channel_generation={} role={} op={} transport={} fallback={} phase={} result={} reason={}",
            event, self.handle.generation, service.pid, service.incarnation, service.vm_generation,
            self.creator.pid, self.creator.incarnation, self.creator.vm_generation,
            self.contract.policy as u64, self.contract.policy_identity.map_or(0, |id| id.policy_id),
            self.contract.policy_identity.map_or(0, |id| id.generation), self.handle.generation, self.contract.service as u64,
            self.contract.operation, self.contract.transport.map_or(0, |value| value as u64),
            u64::from(self.contract.fallback), self.contract.phase as u64, result, reason);
    }

    pub(super) fn contract_retire(&mut self, reason: Terminal) {
        if let Some(outcome) = self.contract.retire() {
            self.contract_note(
                if outcome == "failure" {
                    "terminal"
                } else {
                    "lifecycle"
                },
                reason as u64,
                outcome,
            );
        }
    }
}

impl Scheduler {
    pub(crate) fn fastlane_contract(&mut self, h: Handle, op: u64, arg: u64) -> Result<u64, Error> {
        self.fastlane_mapping_sweep();
        self.fastlane_channel(5, h, 0, SessionRef { id: 0, epoch: 0 })?;
        let current = self.current_process();
        let who = RecipientIdentity::of(current);
        if !self.channel_vm_exclusive(who) {
            return Err(Error::Denied);
        }
        let mut caps = CAP_BROKER.lock();
        // Resolve before borrowing the row mutably. Only an authenticated creator
        // on a paired channel can cause lazy issuance for the trusted peer.
        let existing = {
            let row = caps.channels.row_mut(h)?;
            row.contract
                .policy_identity
                .map(|id| (id, row.consumer, row.contract.service))
        };
        if let Some((identity, consumer, service)) = existing {
            let peer = consumer.ok_or(Error::Incomplete)?;
            let process = self.channel_process(peer).ok_or(Error::Stale)?;
            if process.contract_service != Some(service) {
                return Err(Error::Denied);
            }
            caps.channels.policies.resolve(identity, peer, service)?;
        }
        let resolved = if op == 31 {
            let row = caps.channels.row_mut(h)?;
            if !row.live() {
                return Err(Error::Terminal);
            }
            if row.creator != who {
                return Err(Error::Denied);
            }
            if row.state != State::Paired {
                return Err(Error::Duplicate);
            }
            let consumer = row.consumer.ok_or(Error::Incomplete)?;
            let peer = self.channel_process(consumer).ok_or(Error::Stale)?;
            let service = peer.contract_service.ok_or(Error::Denied)?;
            let identity = caps.channels.policies.for_service(consumer, service)?;
            caps.channels
                .policies
                .resolve(identity, consumer, service)?;
            Some((service, identity))
        } else {
            None
        };
        let row = caps.channels.row_mut(h)?;
        if !row.live() {
            return Err(Error::Terminal);
        }
        if op == 35 {
            // Exact version/opcode envelope; reject reserved high bits rather
            // than truncating a caller-supplied operation identity.
            if arg >> 24 != 0 {
                return Err(Error::Denied);
            }
            let operation = row
                .contract
                .service
                .wire_operation((arg >> 16) as u8, arg as u16)?;
            if row.consumer != Some(who)
                || row.contract.phase != Phase::Ready
                || operation != row.contract.operation
            {
                return Err(Error::Denied);
            }
            let reply = current.ipc_reply_target.ok_or(Error::Denied)?;
            if !crate::ipc::reply_target_is_live(reply, self) {
                return Err(Error::Denied);
            }
            let target = reply.call.pid;
            let requester = self
                .grant_requester_for_role(target, current.contract_service.is_some())
                .ok_or(Error::Denied)?;
            if RecipientIdentity::of(requester) != row.creator {
                return Err(Error::Denied);
            }
            if !row
                .contract
                .service
                .operation_authorized(operation, requester.trusted_auth_broker)
            {
                return Err(Error::Denied);
            }
            if row.contract.hybrid_transform {
                if row.contract.hybrid_started {
                    return Err(Error::Duplicate);
                }
                row.contract.hybrid_started = true;
                row.contract.hybrid_reply = Some(reply);
                row.contract.hybrid_endpoint = Some(reply.endpoint_id);
                crate::serial_println!("[FL-HYBRID-K] event=authorized channel={} policy={} pg={} caller={} ci={} cv={} service={} si={} sv={} operation=65552 request=FastLaneRequired reply=KernelIPC endpoint={} call_generation={} call_incarnation={}",
                    h.generation, row.contract.policy_identity.unwrap().policy_id, row.contract.policy_identity.unwrap().generation,
                    row.creator.pid, row.creator.incarnation, row.creator.vm_generation, who.pid, who.incarnation, who.vm_generation,
                    reply.endpoint_id, reply.call.generation, reply.call.incarnation);
            }
            return row
                .contract
                .transport
                .map(|value| value as u64)
                .ok_or(Error::Incomplete);
        }
        if op == 34 {
            return Ok(row.contract.transport.map_or(0, |value| value as u64)
                | (row.contract.phase as u64) << 8);
        }
        if who != row.creator || op == 36 {
            return Err(Error::Denied);
        }
        match op {
            31 => {
                if row.state != State::Paired {
                    return Err(Error::Duplicate);
                }
                let (catalog, identity) = resolved.ok_or(Error::Denied)?;
                row.contract.policy_identity = Some(identity);
                let result = row
                    .contract
                    .begin(catalog, arg, current.trusted_auth_broker);
                if result == Err(Error::Duplicate) {
                    return Err(Error::Duplicate);
                }
                row.contract_note(
                    "begin",
                    result.as_ref().err().map_or(0, |error| *error as u64),
                    if row.contract.fallback {
                        "initial-offer"
                    } else {
                        "contract"
                    },
                );
                match result {
                    Ok(transport) => Ok(transport as u64),
                    Err(error) => {
                        row.retire(Terminal::Revoked);
                        Err(error)
                    }
                }
            }
            32 => {
                if row.contract.phase != Phase::Preparing {
                    return Err(if row.contract.phase == Phase::Ready {
                        Error::Duplicate
                    } else {
                        Error::Terminal
                    });
                }
                let transport = row.contract.transport.ok_or(Error::Incomplete)?;
                let complete = match transport {
                    Transport::Kernel => {
                        row.state == State::Paired
                            && row.consumer_receipt == Some(CapabilityToken(arg))
                    }
                    Transport::FastLane => {
                        arg == 0
                            && row.state == State::Attached
                            && row.native_ring == [true; 2]
                            && row.mapped.iter().flatten().all(|mapped| *mapped)
                    }
                };
                if !complete {
                    row.retire(Terminal::Revoked);
                    return Err(Error::Incomplete);
                }
                if transport == Transport::Kernel {
                    row.consumer_receipt = None;
                    row.state = State::Created;
                }
                row.contract.finish()?;
                row.contract_note("terminal", 0, "success");
                Ok(transport as u64)
            }
            33 => {
                row.retire(Terminal::Revoked);
                Ok(0)
            }
            _ => Err(Error::Denied),
        }
    }
}
