//! Privileged interface to the authoritative kernel mount table.
#![no_std]
#![no_main]

use core::fmt::{self, Write};
use sunlight_ipc::storage as api;
use sunlight_libc::{crt0, exit, sys, write_all, STDOUT};

struct Output;
impl Write for Output {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        write_all(STDOUT, text.as_bytes()).map_err(|_| fmt::Error)
    }
}

fn control(op: u64, value: u64, pointer: u64) -> Result<u64, i64> {
    let result = unsafe {
        sys::syscall3(
            sunlight_ipc::SunlightSyscall::StorageCtl as u64,
            op,
            value,
            pointer,
        )
    } as i64;
    if result < 0 {
        Err(result)
    } else {
        Ok(result as u64)
    }
}
fn word(record: &[u8], index: usize) -> u64 {
    u64::from_le_bytes(record[index * 8..index * 8 + 8].try_into().unwrap())
}
fn number(value: &[u8]) -> Result<u64, i64> {
    core::str::from_utf8(value)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(-22)
}
fn mount(id: u64, path: &[u8]) -> Result<(), i64> {
    let mut target = [0u8; api::TARGET_MAX];
    if path.is_empty() || path.len() >= target.len() || path.contains(&0) {
        return Err(-22);
    }
    target[..path.len()].copy_from_slice(path);
    control(api::MOUNT_FAT32, id, target.as_ptr() as u64).map(|_| ())
}

fn run(args: &[&[u8]]) -> Result<(), i64> {
    match args {
        [b"devices"] => {
            let mut record = [0u8; api::DEVICE_RECORD_SIZE];
            for index in 0..129 {
                if control(api::ENUMERATE, index, record.as_mut_ptr() as u64)? == 0 {
                    break;
                }
                let _ = writeln!(
                    Output,
                    "id={} parent={} start={} blocks={} ops={} partition={} layout={} sector={}",
                    word(&record, 0),
                    word(&record, 1),
                    word(&record, 2),
                    word(&record, 3),
                    word(&record, 4),
                    word(&record, 5),
                    word(&record, 6),
                    word(&record, 7)
                );
            }
        }
        [b"mounts"] => {
            let mut record = [0u8; api::MOUNT_RECORD_SIZE];
            for index in 0..8 {
                if control(api::LIST_MOUNTS, index, record.as_mut_ptr() as u64)? == 0 {
                    break;
                }
                let len = word(&record, 3) as usize;
                if len >= api::TARGET_MAX {
                    return Err(-5);
                }
                let path = core::str::from_utf8(&record[32..32 + len]).map_err(|_| -5)?;
                let _ = writeln!(
                    Output,
                    "mount={} device={} protected={} {}",
                    word(&record, 0),
                    word(&record, 1),
                    word(&record, 2),
                    path
                );
            }
        }
        [b"mount", id, target] => mount(number(id)?, target)?,
        [b"unmount", id] => {
            control(api::UNMOUNT, number(id)?, 0)?;
        }
        #[cfg(feature = "storage-test")]
        [b"probe"] => probe()?,
        #[cfg(feature = "storage-test")]
        [b"denied"] => {
            // Executed via an alias without the per-binary StorageAdmin grant.
            for op in [
                api::ENUMERATE,
                api::LIST_MOUNTS,
                api::MOUNT_FAT32,
                api::UNMOUNT,
            ] {
                if control(op, 0, 0) != Err(-1) {
                    return Err(-5);
                }
            }
            let _ = writeln!(Output, "[STORAGE-RUNTIME] unauthorized PASS");
            sunlight_ipc::debug_log("[STORAGE-RUNTIME] unauthorized PASS");
        }
        _ => {
            let _ = writeln!(
                Output,
                "storagectl devices | mounts | mount ID /mnt | unmount MOUNT_ID"
            );
            return Err(-22);
        }
    }
    Ok(())
}

#[cfg(feature = "storage-test")]
fn probe() -> Result<(), i64> {
    fn check(ok: bool, step: &str) -> Result<(), i64> {
        if ok {
            Ok(())
        } else {
            let _ = writeln!(Output, "[STORAGE-RUNTIME] FAIL {}", step);
            Err(-5)
        }
    }
    let mut record = [0u8; api::MOUNT_RECORD_SIZE];
    check(
        control(api::LIST_MOUNTS, 0, record.as_mut_ptr() as u64)? == 1 && word(&record, 2) == 1,
        "root",
    )?;
    let root = word(&record, 0);
    check(control(api::UNMOUNT, root, 0) == Err(-1), "protected root")?;
    check(
        control(api::LIST_MOUNTS, 1, record.as_mut_ptr() as u64)? == 0,
        "initially unmounted",
    )?;
    let mut device = 0;
    for index in 0..129 {
        if control(api::ENUMERATE, index, record.as_mut_ptr() as u64)? == 0 {
            break;
        }
        if word(&record, 4) & 8 != 0 {
            device = word(&record, 0);
        }
    }
    check(device != 0, "device enumeration")?;
    check(mount(u64::MAX, b"/mnt") == Err(-2), "unknown device")?;
    check(mount(device, b"/state") == Err(-1), "state trust")?;
    mount(device, b"/mnt")?;
    check(mount(device, b"/mnt") == Err(-17), "duplicate mount")?;
    check(
        control(api::LIST_MOUNTS, 1, record.as_mut_ptr() as u64)? == 1
            && word(&record, 1) == device,
        "mounted source",
    )?;
    let mounted = word(&record, 0);
    let fd = sunlight_libc::open(b"/mnt/HELLO.TXT").map_err(|_| -5)?;
    let mut bytes = [0u8; 64];
    let n = sunlight_libc::read(fd, &mut bytes).map_err(|_| -5)?;
    check(&bytes[..n] == b"partition fixture\n", "fixture contents")?;
    sunlight_libc::close(fd).map_err(|_| -5)?;
    let fd = sunlight_libc::open_with_flags_mode(
        b"/mnt/PROBE.TXT",
        sunlight_libc::O_RDWR | sunlight_libc::O_CREAT | sunlight_libc::O_TRUNC,
        0o600,
    )
    .map_err(|_| -5)?;
    let content = b"guest durable contents\n";
    sunlight_libc::write_all(fd, content).map_err(|_| -5)?;
    sunlight_libc::lseek(fd, 0, 0).map_err(|_| -5)?;
    let n = sunlight_libc::read(fd, &mut bytes).map_err(|_| -5)?;
    check(&bytes[..n] == content, "exact write/read contents")?;
    sunlight_libc::file_sync(fd).map_err(|_| -5)?;
    sunlight_libc::dir_sync(b"/mnt").map_err(|_| -5)?;
    check(
        control(api::UNMOUNT, mounted, 0) == Err(-16),
        "busy open file",
    )?;
    let duplicate = sunlight_libc::dup(fd).map_err(|_| -5)?;
    sunlight_libc::close(fd).map_err(|_| -5)?;
    check(
        control(api::UNMOUNT, mounted, 0) == Err(-16),
        "busy duplicate descriptor",
    )?;
    sunlight_libc::close(duplicate).map_err(|_| -5)?;
    control(api::UNMOUNT, mounted, 0)?;
    check(control(api::UNMOUNT, mounted, 0) == Err(-2), "stale mount")?;
    mount(device, b"/mnt")?;
    let fd = sunlight_libc::open(b"/mnt/PROBE.TXT").map_err(|_| -5)?;
    let n = sunlight_libc::read(fd, &mut bytes).map_err(|_| -5)?;
    check(&bytes[..n] == content, "exact remount contents")?;
    sunlight_libc::close(fd).map_err(|_| -5)?;
    control(api::LIST_MOUNTS, 1, record.as_mut_ptr() as u64)?;
    check(word(&record, 0) != mounted, "mount incarnation")?;
    control(api::UNMOUNT, word(&record, 0), 0)?;
    let _ = writeln!(
        Output,
        "[STORAGE-RUNTIME] mount write read sync busy close unmount remount exact PASS"
    );
    sunlight_ipc::debug_log(
        "[STORAGE-RUNTIME] mount write read sync busy close unmount remount exact PASS",
    );
    Ok(())
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    let _ = writeln!(Output, "storagectl: panic");
    exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut pointers = [core::ptr::null(); 9];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut pointers) };
    let mut args = [&[][..]; 9];
    for index in 0..count {
        args[index] = unsafe {
            core::slice::from_raw_parts(pointers[index], crt0::cstr_len(pointers[index], 256))
        };
    }
    let args = if args
        .first()
        .is_some_and(|a| a.ends_with(b"storagectl") || a.ends_with(b"storage-unprivileged"))
    {
        &args[1..count]
    } else {
        &args[..count]
    };
    match run(args) {
        Ok(()) => exit(0),
        Err(errno) => {
            let _ = writeln!(Output, "storagectl: error {}", errno);
            exit(1);
        }
    }
}
