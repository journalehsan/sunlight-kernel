//! Test-only client launched by the existing Account session fixture.
//! The real noninteractive shell launches and waitpid-collects unchanged clients.
fn command(case: &str) {
    let script = alloc::format!(
        "touch /tmp/session-race-probe && /tmp/session-race-probe {}",
        case
    );
    let shell = sunlight_libc::spawn(b"/bin/sh", &[b"sh", b"-c", script.as_bytes()], None)
        .expect("Account-inherited shell launch");
    sunlight_ipc::debug_log(&alloc::format!(
        "[KFP-SEC] event=shell-started case={} controller={} shell={}",
        case,
        sunlight_libc::getpid(),
        shell
    ));
    // The shell waits for the original client. Collect its actual exit status;
    // terminal security markers alone never authorize a successful exit receipt.
    let status = super::reap(shell);
    sunlight_ipc::debug_log(&alloc::format!(
        "[KFP-SEC] event=shell-collected case={} shell={} status={}",
        case,
        shell,
        status
    ));
    sunlight_ipc::debug_log(&alloc::format!(
        "exit: /tmp/session-race-probe {} -> {}",
        case,
        status
    ));
    assert_eq!(status, 0, "original security shell/client exit");
}

pub fn run() -> ! {
    let suite = option_env!("KFP2A4_SUITE").expect("explicit security suite required");
    assert!(matches!(suite, "m2" | "m4" | "m5" | "r2" | "r4"));
    sunlight_ipc::debug_log(&alloc::format!(
        "[KFP-SEC] event=controller-ready suite={} pid={}",
        suite,
        sunlight_libc::getpid()
    ));
    match suite {
        "m2" => {
            command("FL");
            command("FL-M2");
        }
        "r4" => command("FL-LC-R4"),
        "r2" => command("FL-LC-R2"),
        "m4" => command("FL-M4"),
        "m5" => command("FL-M5"),
        _ => unreachable!(),
    }
    sunlight_ipc::debug_log(&alloc::format!("[KFP-SEC] event=complete suite={}", suite));
    // Keep the session owner alive while the host assesses receipts. Host
    // termination of QEMU is never a cleanup/recovery receipt.
    loop {
        sunlight_ipc::process_yield();
    }
}
