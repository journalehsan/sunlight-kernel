//! Restricted retirement diagnostic. Uses ordinary Account IPC and M1/M2 calls.
//! Eager mapping withdrawal is measured, never bypassed to force retention.
use super::*;

pub(crate) struct Held {
    owner: Actor,
    foreign: Actor,
    ep: EndpointId,
    channel: fl::Handle,
}

fn observe(label: &str, expected: u64) {
    let command = alloc::format!("[R5] observe {}", label);
    let bytes = unsafe {
        sunlight_libc::sys::syscall2(
            sunlight_libc::sys::SYS_DEBUG_LOG,
            command.as_ptr() as u64,
            command.len() as u64,
        )
    };
    assert_eq!(bytes, expected, "restricted diagnostic session charge");
}

fn setup(order: &str, expected: u64) -> Held {
    let ep = sunlight_ipc::endpoint_create();
    let public = sunlight_ipc::endpoint_bind(ep.0);
    let owner = spawn(ep, public);
    let foreign = spawn(ep, public);
    let proposal = expect(owner, AUTHORIZE, fl::EMPTY, 60000, 0);
    let channel = fl::Handle {
        token: proposal.words[1],
        generation: proposal.words[2],
    };
    let consent = authorize(1, channel, 0).expect("restricted peer Account consent");
    expect(owner, 2, channel, consent.token, 0);
    expect(owner, 3, channel, 0, 0);
    assert_eq!(fl::operate(3, channel, 1), Ok(0));
    expect(foreign, 3, channel, 1, 1);
    let owner_va = expect(owner, 8, channel, 0, 0).words[1];
    let peer_va = fl::operate(8, channel, 3).expect("restricted peer mapping");
    expect(foreign, 8, channel, 3, 1);
    // Populate the actual views while both authorized processes are live.
    expect(owner, 20, channel, owner_va | 1, 0);
    let value: u8;
    unsafe {
        core::arch::asm!("mov {value}, byte ptr [{va}]", value=out(reg_byte) value,
            va=in(reg) peer_va, options(nostack, readonly));
    }
    assert_eq!(value, 0x5a);
    observe(&alloc::format!("r54-{}-attached", order), expected);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R54] event=attached order={} generation={} owner={} peer={} foreign={} owner_va={} peer_va={} owner_cleanup=held",
        order, channel.generation, owner.pid, sunlight_libc::getpid(), foreign.pid, owner_va, peer_va
    ));
    Held {
        owner,
        foreign,
        ep,
        channel,
    }
}

fn close(held: Held) {
    stop(held.foreign);
    assert!(sunlight_ipc::endpoint_destroy(held.ep));
    mapping::census(540);
}

pub(crate) fn cleanup_first() {
    let held = setup("cleanup-first", 8192);
    stop(held.owner);
    assert_eq!(fl::operate(8, held.channel, 4), Err(fl::Error::Terminal));
    observe("r54-cleanup-first-reclaimed", 4096);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R54] event=cleanup-first-complete generation={} owner={} session_retirement=not_requested",
        held.channel.generation, held.owner.pid
    ));
    close(held);
}

pub(crate) fn prepare() -> Held {
    setup("retirement-first", 16384)
}

pub(crate) fn finish(held: Held) {
    assert_eq!(fl::operate(3, held.channel, 1), Err(fl::Error::Terminal));
    assert_eq!(fl::operate(8, held.channel, 4), Err(fl::Error::Terminal));
    assert!(authorize(0, fl::EMPTY, 30000).is_err());
    // Do not touch the withdrawn VA: the ordinary retirement sweep removed it.
    observe("r54-retired-owner-held", 12288);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R54] event=retired-owner-held generation={} owner={} attachment=denied mapping=denied proposal=denied restricted_charge=0 required_retention=UNQUALIFIED",
        held.channel.generation, held.owner.pid
    ));
    stop(held.owner);
    observe("r54-retired-owner-cleaned", 12288);
    close(held);
    sunlight_ipc::debug_log(
        "[R54] event=diagnostic-complete qualification=UNQUALIFIED fresh_sessions_per_order=0",
    );
}

// R55 uses one selected ordering per isolated guest, with ordinary Account
// membership. The cleanup-first peer is a separate child and is fully reaped.
pub(crate) fn r55_cleanup_peer() -> fl::Handle {
    mapping::census(550);
    let held = setup("cleanup-first", 8192);
    let channel = held.channel;
    stop(held.owner);
    assert_eq!(fl::operate(8, channel, 4), Err(fl::Error::Terminal));
    observe("r55-cleanup-first-reclaimed", 4096);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R55] event=cleanup-mappings-complete generation={} owner={} peer={}",
        channel.generation,
        held.owner.pid,
        sunlight_libc::getpid()
    ));
    close(held);
    channel
}

pub(crate) fn r55_prepare() -> Held {
    mapping::census(550);
    setup("retirement-first", 16384)
}

pub(crate) fn r55_denied(channel: fl::Handle) {
    assert_eq!(fl::operate(3, channel, 1), Err(fl::Error::Terminal));
    assert_eq!(fl::operate(8, channel, 4), Err(fl::Error::Terminal));
    let stale = fl::Handle {
        token: channel.token,
        generation: channel.generation.checked_add(1).unwrap(),
    };
    assert_eq!(fl::operate(3, stale, 1), Err(fl::Error::Stale));
    assert!(authorize(0, fl::EMPTY, 30000).is_err());
    assert!(sunlight_ipc::shm_create(4096, 0).is_err());
    sunlight_ipc::debug_log(&alloc::format!("[R55] event=retired-denied generation={} attachment=denied mapping=denied proposal=denied reservation=denied stale_channel=denied", channel.generation));
}

pub(crate) fn r55_finish(held: Held) {
    r55_denied(held.channel);
    observe("r55-retired-owner-held", 12288);
    sunlight_ipc::debug_log(&alloc::format!(
        "[R55] event=retired-owner-held generation={} owner={}",
        held.channel.generation,
        held.owner.pid
    ));
    stop(held.owner);
    observe("r55-retired-owner-cleaned", 12288);
    close(held);
}
