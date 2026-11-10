//! Opt-in headless ELF fixture. Uses production exec, descriptor I/O and reaping.
//! The host supplies a generated manifest; no new syscall or loader exception.
use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass as Class};
use crate::process::address_space::ReclaimStats;
use crate::process::fd_table::{CapRights, FileHandle};
use crate::process::{Process, ProcessState};
use crate::sched::{Scheduler, SCHEDULER};
use alloc::vec::Vec;
use sunlight_fs::FsError;
use x86_64::VirtAddr;

struct Case {
    name: &'static str,
    elf: &'static [u8],
    args: &'static [&'static [u8]],
    input: &'static [u8],
}
include!(env!("SUNLIGHT_NATIVE_C_MANIFEST"));

pub const INITRAMFS: &[sunlight_fs::RamEntry] = &[
    sunlight_fs::RamEntry::dir("/", 0, 0, sunlight_fs::vfs::mode::DIR_755),
    sunlight_fs::RamEntry::dir("/tmp", 0, 0, sunlight_fs::vfs::mode::DIR_1777),
];

const PID: usize = 100;
const IMAGE: &str = "/tmp/native-c.elf";
const STREAMS: [&str; 3] = [
    "/tmp/native-c.stdin",
    "/tmp/native-c.stdout",
    "/tmp/native-c.stderr",
];
const WORK: &str = "/tmp/native-c-work";
const OUTPUT_LIMIT: usize = 1024 * 1024;
struct State {
    next: usize,
    slot: Option<usize>,
    baseline: [u64; 3],
    names: usize,
    handles: usize,
}
static STATE: spin::Mutex<Option<State>> = spin::Mutex::new(None);

fn frames() -> [u64; 3] {
    [Class::RamFsFileData, Class::UserPrivate, Class::PageTable].map(class_frame_count)
}
fn tmp_names() -> usize {
    let mut count = 0;
    crate::KERNEL_VFS
        .lock()
        .as_mut()
        .unwrap()
        .read_dir("/tmp", &mut |_| {
            count += 1;
            true
        })
        .unwrap();
    count
}
fn handles() -> usize {
    crate::KERNEL_VFS
        .lock()
        .as_ref()
        .unwrap()
        .open_handle_count()
}
fn store(path: &str, data: &[u8], mode: u16) -> sunlight_fs::vfs::FileHandle {
    let mut guard = crate::KERNEL_VFS.lock();
    let vfs = guard.as_mut().unwrap();
    let handle = vfs.create_file_exclusive(path, 1000, 1000, mode).unwrap();
    let mut offset = 0;
    while offset < data.len() {
        let n = vfs.write(handle, offset, &data[offset..]).unwrap();
        assert!(n > 0);
        offset += n;
    }
    vfs.close(handle).unwrap();
    vfs.open(path).unwrap()
}
fn read_file(path: &str, limit: usize) -> Vec<u8> {
    let mut guard = crate::KERNEL_VFS.lock();
    let vfs = guard.as_mut().unwrap();
    let size = vfs.stat(path).unwrap().size;
    assert!(size <= limit, "native C capture exceeds limit");
    let mut bytes = alloc::vec![0; size];
    let handle = vfs.open(path).unwrap();
    let mut offset = 0;
    while offset < size {
        let n = vfs.read(handle, offset, &mut bytes[offset..]).unwrap();
        assert!(n > 0);
        offset += n;
    }
    vfs.close(handle).unwrap();
    bytes
}
fn remove(path: &str) {
    let mut guard = crate::KERNEL_VFS.lock();
    match guard.as_mut().unwrap().unlink(path) {
        Ok(()) | Err(FsError::NotFound) => (),
        other => panic!("native C cleanup: {:?}", other),
    }
}
fn launch(sched: &mut Scheduler, state: &mut State, hhdm: VirtAddr) {
    let number = state.next;
    let case = &CASES[number];
    state.baseline = frames();
    state.names = tmp_names();
    state.handles = handles();
    // Transport bytes through the actual writable guest root, then load the
    // bytes read back from that file. PT_LOAD validation remains unchanged.
    let image = store(IMAGE, case.elf, 0o755);
    crate::KERNEL_VFS
        .lock()
        .as_mut()
        .unwrap()
        .close(image)
        .unwrap();
    let bytes = read_file(IMAGE, 64 * 1024 * 1024);
    assert_eq!(bytes, case.elf);
    remove(IMAGE);
    let streams = STREAMS.map(|path| {
        store(
            path,
            if path == STREAMS[0] { case.input } else { &[] },
            0o600,
        )
    });
    let mut pmm = crate::PMM.lock();
    let mut process = unsafe { Process::new(PID + number, 0, case.name, &mut pmm, hhdm) };
    process.uid = 1000;
    process.gid = 1000;
    process.fd_table.take_all_handles(); // discard the three serial defaults
    for (fd, handle) in streams.into_iter().enumerate() {
        let rights = if fd == 0 {
            CapRights::READ
        } else {
            CapRights::WRITE
        } | CapRights::FSTAT;
        assert_eq!(
            process
                .fd_table
                .open(
                    FileHandle::vfs(handle.0),
                    CapRights::new(rights),
                    if fd == 0 { 0 } else { 1 }
                )
                .unwrap(),
            fd as i32
        );
    }
    let argv: Vec<&[u8]> = core::iter::once(IMAGE.as_bytes())
        .chain(case.args.iter().copied())
        .collect();
    let result = crate::process::spawn::exec_into_process(
        &bytes,
        &mut process,
        &mut pmm,
        hhdm,
        &argv,
        &[b"TMPDIR=/tmp"],
        false,
    );
    if let Err(error) = result {
        crate::serial_println!("[NATIVE-C-LOAD-FAIL] case={} error={:?}", number, error);
        quit(false);
    }
    let owned = pmm.owned_frame_count(process.pid as u32);
    drop(pmm);
    let slot = sched.add_process(process);
    state.slot = Some(slot);
    sched.enqueue_ready(slot);
    crate::serial_println!(
        "[NATIVE-C-START] case={} name={} pid={} bytes={} owned_frames={} uid=1000",
        number,
        case.name,
        PID + number,
        bytes.len(),
        owned
    );
}

fn support(sched: &mut Scheduler, hhdm: VirtAddr, pid: usize, name: &str, bytes: &[u8]) {
    let mut pmm = crate::PMM.lock();
    let mut process = unsafe { Process::new(pid, 0, name, &mut pmm, hhdm) };
    process.system_allocation = true;
    let entry = crate::process::elf_loader::load_elf(bytes, &mut process, &mut pmm, hhdm)
        .expect("native C support ELF");
    crate::process::spawn::map_user_stack(&mut process, &mut pmm, hhdm).unwrap();
    process.init_context(entry, crate::process::layout::USER_STACK_TOP);
    process.set_initial_args(0, 0, 0, 0);
    drop(pmm);
    let slot = sched.add_process(process);
    sched.enqueue_ready(slot);
}

pub fn boot(hhdm: VirtAddr) -> ! {
    assert!(!CASES.is_empty());
    let state = State {
        next: 0,
        slot: None,
        baseline: [0; 3],
        names: 0,
        handles: 0,
    };
    crate::serial_println!(
        "[NATIVE-C-RUNNER] cases={} cpus={} entropy={}",
        CASES.len(),
        crate::sched::ONLINE_CORES.load(core::sync::atomic::Ordering::Acquire),
        crate::entropy::is_ready()
    );
    // Use the actual nameserver with a zero spawn token (registry only), plus
    // the actual rand service. libc getrandom(flags=0) must keep its production
    // capability-IPC path and fail-closed behavior.
    crate::sched::with_scheduler(|s| {
        support(s, hhdm, 1, "init", INIT_ELF);
        support(s, hhdm, 2, "rand_service", RAND_ELF);
    });
    *STATE.lock() = Some(state);
    crate::sched::enter_first_process()
}

pub fn before_reap(p: &Process) {
    if p.pid < PID || p.pid >= PID + CASES.len() {
        return;
    }
    let fds = (0..256).filter(|&fd| p.fd_table.get(fd).is_some()).count();
    crate::serial_println!(
        "[NATIVE-C-REAP-BEGIN] case={} fds={} file_frames={}",
        p.pid - PID,
        fds,
        class_frame_count(Class::RamFsFileData)
    );
}
pub fn after_reap(p: &Process, stats: &ReclaimStats) {
    if p.pid < PID || p.pid >= PID + CASES.len() {
        return;
    }
    let owned = crate::PMM.lock().owned_frame_count(p.pid as u32);
    assert_eq!(owned, 0, "native C process-owned frames leaked");
    assert!(p.kernel_stack.is_none() && !p.exit_cleanup_pending);
    assert!((0..256).all(|fd| p.fd_table.get(fd).is_none()));
    crate::serial_println!("[NATIVE-C-REAP-END] case={} exit={} user_frames={} page_tables={} owned_frames={} fds=0 stack=0", p.pid - PID, p.exit_code, stats.user_frames, stats.page_tables, owned);
}

fn emit(number: usize, fd: usize, bytes: &[u8]) {
    // Hex framing preserves binary output and keeps guest stdout from forging
    // runner records. One serial lock per bounded record, including its prefix.
    for chunk in bytes.chunks(128) {
        let mut hex = [0u8; 256];
        for (i, byte) in chunk.iter().enumerate() {
            hex[2 * i] = b"0123456789abcdef"[(byte >> 4) as usize];
            hex[2 * i + 1] = b"0123456789abcdef"[(byte & 15) as usize];
        }
        crate::serial_println!(
            "[NATIVE-C-STREAM] case={} fd={} hex={}",
            number,
            fd,
            core::str::from_utf8(&hex[..chunk.len() * 2]).unwrap()
        );
    }
}
pub fn idle() {
    // Called with local IRQs disabled. APs may enter concurrently; preserve
    // SCHEDULER -> fixture -> VFS -> PMM order and never block behind an IRQ.
    let Some(mut sched) = SCHEDULER.try_lock() else {
        return;
    };
    let Some(mut guard) = STATE.try_lock() else {
        return;
    };
    let Some(state) = guard.as_mut() else {
        return;
    };
    let Some(slot) = state.slot else {
        let ready = sched
            .processes
            .iter()
            .find(|p| p.pid == 2)
            .is_some_and(|p| {
                p.state == ProcessState::BlockedOnIpc
                    && p.pending_call.is_none()
                    && p.ipc_endpoint.is_some_and(|ep| {
                        crate::capability::CAP_BROKER.lock().endpoint_owner(ep) == Some(2)
                    })
            });
        if ready {
            crate::serial_println!(
                "[NATIVE-C-SUPPORT] nameserver=production rand=production receive_wait=ready"
            );
            let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
            launch(&mut sched, state, hhdm);
        }
        return;
    };
    if sched.processes[slot].state != ProcessState::Reaped {
        return;
    }
    let code = sched.processes[slot].exit_code;
    let out = read_file(STREAMS[1], OUTPUT_LIMIT);
    let err = read_file(STREAMS[2], OUTPUT_LIMIT);
    emit(state.next, 1, &out);
    emit(state.next, 2, &err);
    for path in STREAMS {
        remove(path);
    }
    remove(WORK);
    let after = frames();
    let clean = after == state.baseline && tmp_names() == state.names && handles() == state.handles;
    crate::serial_println!("[NATIVE-C-RESULT] case={} exit={} stdout={} stderr={} clean={} file_before={} file_after={} private_before={} private_after={} pt_before={} pt_after={} handles_before={} handles_after={}", state.next, code, out.len(), err.len(), u8::from(clean), state.baseline[0], after[0], state.baseline[1], after[1], state.baseline[2], after[2], state.handles, handles());
    assert!(clean, "native C file/page/handle cleanup failed");
    state.next += 1;
    if state.next == CASES.len() {
        crate::serial_println!("[NATIVE-C-DONE] cases={}", CASES.len());
        quit(true);
    }
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
    launch(&mut sched, state, hhdm);
}
fn quit(success: bool) -> ! {
    unsafe {
        x86_64::instructions::port::Port::<u32>::new(0xf4).write(if success { 0x10 } else { 0x11 })
    };
    loop {
        x86_64::instructions::hlt();
    }
}
