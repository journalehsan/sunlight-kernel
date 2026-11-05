//! Bounded, read-only census. Caller owns SCHEDULER; then broker, then one
//! shard at a time. No reference escapes, no IPC is blocked awaiting a peer.
//! Userspace heaps/SessionStore are outside this snapshot's atomic boundary.
use super::{CapabilityToken, GrantDelivery, RecipientIdentity, CAP_BROKER};
use crate::sched::Scheduler;

static FINGERPRINTS: spin::Mutex<[u64; 512]> = spin::Mutex::new([0; 512]);

pub(super) fn fingerprint(token: CapabilityToken) -> usize {
    let mut words = FINGERPRINTS.lock();
    let index = words
        .iter()
        .position(|word| *word == token.0)
        .or_else(|| words.iter().position(|word| *word == 0))
        .expect("bounded grant observation registry exhausted");
    words[index] = token.0;
    index + 1
}

fn validate(sched: &Scheduler, delivery: GrantDelivery) -> (bool, bool) {
    let recipient = sched
        .processes
        .iter()
        .find(|p| RecipientIdentity::of(p) == delivery.recipient)
        .expect("orphan grant recipient incarnation/VM");
    if delivery.delivered {
        // Delivered recipient authority can outlive its issuer by policy.
        return (true, recipient.is_exiting());
    }
    let issuer = sched
        .processes
        .iter()
        .find(|p| {
            p.pid == delivery.issuer_pid
                && p.foreground.incarnation == delivery.issuer_incarnation
                && p.address_space.identity().generation == delivery.issuer_vm_generation
        })
        .expect("orphan pending issuer incarnation/VM");
    let retiring = recipient.is_exiting() || issuer.is_exiting();
    if !retiring {
        assert!(recipient
            .pending_call
            .is_some_and(|c| c.generation == delivery.request));
        assert!(issuer.trusted_auth_broker);
    }
    (false, retiring)
}

pub(super) fn snapshot(sched: &Scheduler, phase: &str) {
    let broker = CAP_BROKER.lock();
    let deliveries = broker
        .auth_session_grants
        .iter()
        .map(|g| ("auth", g.token, g.delivery))
        .chain(
            broker
                .account_grants
                .iter()
                .map(|(t, g)| ("account", *t, g.delivery)),
        )
        .chain(broker.vfs_caps.iter().map(|g| ("vfs", g.token, g.delivery)));
    let (mut pending, mut delivered, mut retiring, mut legacy) = (0, 0, 0, 0);
    for (class, token, delivery) in deliveries {
        let Some(d) = delivery else {
            legacy += 1;
            continue;
        };
        let (published, exiting) = validate(sched, d);
        pending += usize::from(!published);
        delivered += usize::from(published);
        retiring += usize::from(exiting);
        crate::serial_println!("[GRANT-F] row phase={} class={} fingerprint=boot-local-{} pid={} incarnation={} vm={} issuer={} issuer_inc={} issuer_vm={} request={} state={} retiring={} cpu={}", phase, class, fingerprint(token), d.recipient.pid, d.recipient.incarnation, d.recipient.vm_generation, d.issuer_pid, d.issuer_incarnation, d.issuer_vm_generation, d.request, if published { "delivered" } else { "pending" }, u8::from(exiting), crate::sched::current_cpu_id());
    }
    let mut pending_calls = 0;
    let mut reply_targets = 0;
    let mut deferred = 0;
    let mut tombstones = 0;
    let mut replies = 0;
    for process in &sched.processes {
        pending_calls += usize::from(process.pending_call.is_some());
        replies += usize::from(process.ipc_reply.is_some());
        for target in process
            .ipc_reply_target
            .iter()
            .chain(process.deferred_reply_targets.iter().map(|r| &r.target))
        {
            reply_targets += 1;
            let live = sched.processes.iter().any(|p| {
                p.pid == target.call.pid
                    && p.foreground.incarnation == target.call.incarnation
                    && !p.is_exiting()
                    && p.pending_call.is_some_and(|c| {
                        c.generation == target.call.generation
                            && c.endpoint_id == target.endpoint_id
                    })
            });
            tombstones += usize::from(!live);
        }
        deferred += process.deferred_reply_targets.len();
    }
    let mut queued = 0;
    let mut waiters = 0;
    let mut stale = 0;
    let mut capacity = 0;
    crate::ipc::for_all_shards(|bus| {
        let counts = bus.grant_reference_census(sched);
        queued += counts[0];
        waiters += counts[1];
        stale += counts[2];
        capacity += counts[3];
    });
    assert_eq!(stale, 0, "stale IPC queue/waiter identity");
    assert_eq!(
        broker.active_tokens(),
        pending + delivered + legacy + broker.capabilities.len() + broker.shared_regions.len()
    );
    let sessions = broker.session_accounting();
    let (history, history_capacity) = broker.token_history.usage();
    crate::serial_println!("[GRANT-F] census phase={} broker_live={} ipc_tokens={} shm={} pending={} delivered={} legacy={} retiring_subset={} sessions={} receipts={} pending_calls={} reply_targets={} deferred_subset={} tombstones_subset={} stored_replies={} queued_calls={} waiters={} stale_queue_refs={} ipc_capacity_bytes={} history={} history_capacity={} snapshot=scheduler-broker-shards userspace_heaps=UNMEASURED global_reconciliation=INCOMPLETE cpu={}", phase, broker.active_tokens(), broker.capabilities.len(), broker.shared_regions.len(), pending, delivered, legacy, retiring, sessions[0], sessions[6], pending_calls, reply_targets, deferred, tombstones, replies, queued, waiters, stale, capacity, history, history_capacity, crate::sched::current_cpu_id());
}
