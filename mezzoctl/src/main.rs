#![no_std]
#![no_main]

use sunlight_ipc::{ipc_call, nameserver_lookup, IpcMsg, LockState, MezzoMsg};

fn write_line(text: &str) {
    let _ = sunlight_libc::write(sunlight_libc::STDOUT, text.as_bytes());
    let _ = sunlight_libc::write(sunlight_libc::STDOUT, b"\n");
}

fn print_status(reply: IpcMsg) -> bool {
    if reply.label != MezzoMsg::REPLY {
        let mut output = heapless::String::<96>::new();
        use core::fmt::Write;
        let _ = write!(&mut output, "result=error\nerror_code={}", reply.words[0]);
        write_line(&output);
        return false;
    }
    // Compact status packing (register IPC, 4 words):
    // word0 = state | (recovery_attempts << 8) | (safe_mode << 16) | (last_failure << 32)
    // word1 = generation, word2 = presenter_pid, word3 = presenter_generation
    let packed = reply.words[0];
    let state = LockState::from_u64(packed & 0xff);
    let attempts = (packed >> 8) & 0xff;
    let safe = ((packed >> 16) & 0xff) != 0;
    let failure = packed >> 32;
    let fallback = matches!(
        state,
        LockState::LockedFallback | LockState::RecoveringPresenter
    );
    let authenticating = state == LockState::Authenticating;
    let mut output = heapless::String::<384>::new();
    use core::fmt::Write;
    let _ = write!(
        &mut output,
        "result=ok\nstate={:?}\ngeneration={}\npresenter_pid={}\npresenter_generation={}\nfallback={}\nlast_presenter_failure={}\nrecovery_attempts={}\nauthenticating={}\nsafe_mode={}",
        state,
        reply.words[1],
        reply.words[2],
        reply.words[3],
        fallback,
        failure,
        attempts,
        authenticating,
        safe,
    );
    write_line(&output);
    true
}

unsafe fn arg<'a>(argc: u64, argv: *const *const u8, index: usize) -> &'a str {
    if index >= argc as usize || argv.is_null() {
        return "";
    }
    let pointer = *argv.add(index);
    if pointer.is_null() {
        return "";
    }
    let mut length = 0;
    while length < 128 && *pointer.add(length) != 0 {
        length += 1;
    }
    core::str::from_utf8(core::slice::from_raw_parts(pointer, length)).unwrap_or("")
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let command = unsafe { arg(argc, argv, 1) };
    let action = unsafe { arg(argc, argv, 2) };
    let option = unsafe { arg(argc, argv, 3) };
    if command != "lock" {
        write_line("usage: mezzoctl lock [activate|status|recover] [--safe]");
        sunlight_libc::exit(2);
    }
    let Some(mezzo) = nameserver_lookup("mezzo") else {
        write_line("mezzoctl: mezzo service unavailable");
        sunlight_libc::exit(1);
    };
    let message = match action {
        "" | "activate" => IpcMsg::with_label(MezzoMsg::LOCK_ACTIVATE),
        "status" => IpcMsg::with_label(MezzoMsg::LOCK_STATUS),
        "recover" => IpcMsg::with_label(MezzoMsg::LOCK_RECOVER)
            .word(0, u64::from(option == "--safe") * MezzoMsg::RECOVER_SAFE),
        _ => {
            write_line("usage: mezzoctl lock [activate|status|recover] [--safe]");
            sunlight_libc::exit(2);
        }
    };
    let success = print_status(ipc_call(mezzo, message));
    sunlight_libc::exit(if success { 0 } else { 1 });
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sunlight_libc::exit(1)
}
