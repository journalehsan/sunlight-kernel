use sunlight_ipc::{ipc_call_timeout, monotonic_millis, nameserver_lookup, sgp::SgpMsg, IpcMsg};

pub fn wait_for_desktop(timeout_ms: u64) -> bool {
    let start = monotonic_millis();
    loop {
        if nameserver_lookup("display_server").is_some_and(|display| {
            (0..16).any(|index| {
                ipc_call_timeout(
                    display,
                    IpcMsg::with_label(SgpMsg::LIST_WINDOWS).word(0, index),
                    100,
                )
                .is_ok_and(|reply| reply.words[0] != 0 && reply.words[3] & 0xff == 2)
            })
        }) {
            return true;
        }
        if monotonic_millis().wrapping_sub(start) >= timeout_ms {
            return false;
        }
        let delay = sunlight_libc::time::Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        unsafe { sunlight_libc::time::nanosleep(&delay, core::ptr::null_mut()) };
    }
}
