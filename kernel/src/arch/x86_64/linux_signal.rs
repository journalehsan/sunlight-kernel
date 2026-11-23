//! Bounded Linux x86-64 caught signal delivery.
use super::{
    cpu::FxState,
    userspace::{USER_CS as CS, USER_SS as SS},
};
use crate::memory::user;
use crate::process::{
    signal::{SigAction, SigHandler, Signal},
    Process, ProcessState,
};
use crate::sched::{context::iretq_to_context, Scheduler};
use x86_64::VirtAddr;

pub(crate) const CATCHABLE: [Signal; 2] = [Signal::SIGINT, Signal::SIGUSR1];

pub(crate) fn slot(signal: Signal) -> Option<usize> {
    CATCHABLE.iter().position(|&candidate| candidate == signal)
}

#[derive(Clone, Copy)]
pub(crate) struct MaskedWait {
    pub original: u64,
    pub temporary: u64,
}
const SA_RESTORER: u32 = 0x0400_0000;
const SUPPORTED_ACTION_FLAGS: u32 = SA_RESTORER | 0x1000_0000 | 4; // RESTART, SIGINFO
const SELECTORS: u64 = CS | (SS << 48);
const FLAGS_REQUIRED: u64 = 0x202;
const DF: u64 = 1 << 10;
const RF: u64 = 1 << 16;
// The timer's hardware frame can carry RF. IRET can restore this transient
// ring-3 resume flag; it is not passed to the handler.
const FLAGS_ALLOWED: u64 = 0xed7 | RF; // arithmetic flags, IF, DF, RF, fixed bit 1
const UC_FLAGS: u64 = 6; // UC_SIGCONTEXT_SS | UC_STRICT_RESTORE_SS, no XSAVE
const UC_STACK_FLAGS: usize = 32;
const SC: usize = 48; // pretcode + ucontext prefix
const GREGS: usize = 18;
const FP_PTR: usize = SC + 23 * 8;
const MASK: usize = SC + 32 * 8;
const INFO: usize = MASK + 8;
const FRAME_SIZE: usize = INFO + 128;
const RED_ZONE: u64 = 128;
// Linux greg order through RIP, then RFLAGS. Each points into the trusted
// timer_entry/iretq_to_context layout (15 GPRs and five IRET words).
const TO_IRET: [usize; GREGS] = [9, 8, 7, 6, 3, 2, 1, 0, 10, 11, 4, 5, 12, 14, 13, 18, 15, 17];

#[derive(Default)]
pub(crate) struct ThreadState {
    pending: [Option<(u32, u32)>; 2],
    pub(crate) wait: Option<MaskedWait>,
    active: Option<Active>,
}

struct Active {
    frame: u64,
    fp_address: u64,
    fp: FxState,
}

impl ThreadState {
    pub(crate) fn queue(&mut self, slot: usize, pid: u32, uid: u32) {
        // Standard signals coalesce. Retain the first sender's siginfo.
        self.pending[slot].get_or_insert((pid, uid));
    }
}

pub(crate) fn supported_action(flags: u32, restorer: u64) -> bool {
    flags & SA_RESTORER != 0 && flags & !SUPPORTED_ACTION_FLAGS == 0 && restorer != 0
}

fn put(buf: &mut [u8], at: usize, value: u64) {
    buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn get(buf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
}

fn valid_flags(flags: u64) -> bool {
    flags & !FLAGS_ALLOWED == 0 && flags & FLAGS_REQUIRED == FLAGS_REQUIRED
}

fn normal_stack(process: &Process) -> bool {
    process
        .linux_state()
        .is_some_and(|state| state.altstack[1] == 2)
}

fn stack_pointer(process: &Process, hhdm: VirtAddr, sp: u64) -> Result<(), ()> {
    user::UserAddress::new(sp).map_err(|_| ())?;
    user::validate_process_write(process, hhdm, sp.checked_sub(1).ok_or(())?, 1)
        .map_err(|e| error("stack pointer", e))
}

fn deliver(
    process: &mut Process,
    signal: Signal,
    action: SigAction,
    restorer: u64,
    handler: u64,
    sender: (u32, u32),
    regs: &mut [u64; 20],
    restore_mask: Option<u64>,
) -> Result<(), ()> {
    let hhdm = user::current_hhdm().map_err(|_| ())?;
    if regs[16] != CS || regs[19] != SS || !valid_flags(regs[17]) {
        if option_env!("SUNLIGHT_HELIOS_SIG01_TRACE") == Some("1") {
            trace("rejected entry", process.pid, regs);
            crate::serial_println!("[SIG01] cs={:#x} ss={:#x}", regs[16], regs[19]);
        }
        return Err(());
    }
    if !normal_stack(process) {
        return Err(());
    }
    user::validate_process_execute(process, hhdm, handler).map_err(|e| error("handler", e))?;
    user::validate_process_execute(process, hhdm, restorer).map_err(|e| error("restorer", e))?;
    stack_pointer(process, hhdm, regs[18])?;
    let fp_address = regs[18].checked_sub(RED_ZONE + 512).ok_or(())? & !63;
    let frame = (fp_address.checked_sub(FRAME_SIZE as u64).ok_or(())? & !15)
        .checked_sub(8)
        .ok_or(())?;
    let mut wire = [0; FRAME_SIZE];
    put(&mut wire, 0, restorer);
    put(&mut wire, 8, UC_FLAGS);
    put(&mut wire, UC_STACK_FLAGS, 2); // SS_DISABLE
    for (i, &slot) in TO_IRET.iter().enumerate() {
        put(&mut wire, SC + i * 8, regs[slot]);
    }
    put(&mut wire, SC + GREGS * 8, SELECTORS); // zero FS/GS selectors
    put(&mut wire, FP_PTR, fp_address);
    let mask = process.signal_state.get_blocked_mask();
    put(&mut wire, MASK, restore_mask.unwrap_or(mask));
    put(&mut wire, INFO, signal as u64); // si_signo, si_errno
    put(&mut wire, INFO + 8, u64::from((-6i32) as u32)); // SI_TKILL
    put(
        &mut wire,
        INFO + 16,
        u64::from(sender.0) | (u64::from(sender.1) << 32),
    );

    let mut fp = FxState::initial();
    // SAFETY: current task owns the hardware FP registers; the kernel has
    // FP/SIMD disabled. Zero initialization covers FXSAVE's reserved bytes.
    unsafe { fp.save() };
    user::validate_process_write(process, hhdm, frame, (fp_address - frame) as usize + 512)
        .map_err(|e| error("frame", e))?;
    user::copy_to_process_bytes(process, hhdm, fp_address, fp.bytes()).map_err(|_| ())?;
    user::copy_to_process_bytes(process, hhdm, frame, &wire).map_err(|_| ())?;
    process.linux_signal.active = Some(Active {
        frame,
        fp_address,
        fp,
    });
    process
        .signal_state
        .set_blocked_mask(mask | action.mask | (1 << (signal as u32 - 1)));
    if option_env!("SUNLIGHT_HELIOS_SIG01_TRACE") == Some("1") {
        trace(if restore_mask.is_some() { "pselect-delivery" } else { "timer-delivery" }, process.pid, regs);
    }
    regs[10] = signal as u64; // RDI
    regs[11] = frame + INFO as u64; // RSI
    regs[12] = frame + 8; // RDX
    regs[14] = 0; // no vector variadic arguments
    regs[15] = handler;
    // SysV handler entry requires DF clear.
    regs[17] &= !(DF | RF);
    // SAFETY: this is still the current task's FP context; the interrupted
    // image was saved before installing the handler context.
    unsafe { FxState::initial().restore() };
    regs[18] = frame;
    Ok(())
}

/// Called only with IRQs masked and SCHEDULER held before scheduling can
/// swap the current task, address space or FP owner.
pub(crate) unsafe fn timer_boundary(sched: &mut Scheduler, saved_rsp: u64) {
    let raw = saved_rsp as *mut u64;
    // A supervisor interrupt cannot deliver, including a parked syscall.
    if unsafe { raw.add(16).read() } != CS {
        return;
    }
    let Some(idx) = sched.current_process_index() else {
        return;
    };
    let task = &sched.processes[idx];
    if task.state != ProcessState::Running || task.is_exiting() || task.linux_signal.wait.is_some() {
        return;
    }
    // SAFETY: CPL3 selected the complete hardware IRET frame.
    let regs = unsafe { &mut *raw.cast::<[u64; 20]>() };
    if let Err(code) = deliver_pending(sched, idx, regs, None) {
        let pid = sched.processes[idx].pid;
        sched.terminate_process_by_pid(pid, code, "linux caught signal delivery");
    }
}

/// Shared by timer delivery and the masked syscall completion. Called with
/// IRQs masked and SCHEDULER held, on the target task's own FP context.
pub(crate) fn deliver_pending(
    sched: &mut Scheduler,
    idx: usize,
    regs: &mut [u64; 20],
    restore_mask: Option<u64>,
) -> Result<bool, i32> {
    for (slot, signal) in CATCHABLE.into_iter().enumerate() {
        let task = &sched.processes[idx];
        if task.is_exiting() || task.linux_signal.active.is_some()
            || task.signal_state.is_blocked(signal)
        {
            continue;
        }
        let Some(sender) = task.linux_signal.pending[slot] else {
            continue;
        };
        let state = &sched.processes[sched.shared_process_index(idx)].signal_state;
        let action = state.get_handler(signal);
        let restorer = state.linux_restorers[slot];
        let task = &mut sched.processes[idx];
        task.linux_signal.pending[slot] = None;
        match action.handler {
            SigHandler::Ignore => continue,
            SigHandler::Default => return Err(signal.default_exit_code()),
            SigHandler::UserHandler(handler) if supported_action(action.flags, restorer) => {
                deliver(task, signal, action, restorer, handler, sender, regs, restore_mask)
                    .map_err(|()| 139)?;
                return Ok(true);
            }
            SigHandler::UserHandler(_) => return Err(139),
        }
    }
    Ok(false)
}

pub(crate) fn masked_wait_supported(process: &Process) -> bool {
    normal_stack(process) && process.linux_signal.active.is_none()
}

fn decode(wire: &[u8; FRAME_SIZE], fp_address: u64) -> Result<[u64; 20], ()> {
    if get(wire, 8) != UC_FLAGS
        || get(wire, 16) != 0
        || get(wire, UC_STACK_FLAGS - 8) != 0
        || get(wire, UC_STACK_FLAGS) != 2
        || get(wire, UC_STACK_FLAGS + 8) != 0
        || get(wire, SC + GREGS * 8) != SELECTORS
        || get(wire, FP_PTR) != fp_address
    {
        return Err(());
    }
    let mut regs = [0; 20];
    for (i, &slot) in TO_IRET.iter().enumerate() {
        regs[slot] = get(wire, SC + i * 8);
    }
    if !valid_flags(regs[17]) {
        return Err(());
    }
    regs[16] = CS;
    regs[19] = SS;
    Ok(regs)
}

pub(crate) fn sigreturn(user_rsp: u64) -> Result<core::convert::Infallible, ()> {
    let hhdm = user::current_hhdm().map_err(|_| ())?;
    let regs = {
        let mut sched = crate::sched::SCHEDULER.lock();
        let process = sched.current_process_mut();
        let active = process.linux_signal.active.as_ref().ok_or(())?;
        if user_rsp.checked_sub(8) != Some(active.frame)
            || process.is_exiting()
            || !normal_stack(process)
        {
            return Err(());
        }
        let mut wire = [0; FRAME_SIZE];
        user::copy_from_process_bytes(process, hhdm, active.frame, &mut wire).map_err(|_| ())?;
        let regs = decode(&wire, active.fp_address)?;
        user::validate_process_execute(process, hhdm, regs[15]).map_err(|_| ())?;
        stack_pointer(process, hhdm, regs[18])?;

        // This milestone exposes a read-only legacy FP image. Compare all
        // bytes against the trusted snapshot, rejecting XSAVE and any edits
        // including MXCSR before executing FXRSTOR on kernel-owned storage.
        let mut fp_wire = [0; 512];
        user::copy_from_process_bytes(process, hhdm, active.fp_address, &mut fp_wire)
            .map_err(|_| ())?;
        if &fp_wire != active.fp.bytes() {
            return Err(());
        }
        let active = process.linux_signal.active.take().unwrap();
        process.signal_state.set_blocked_mask(get(&wire, MASK));
        // SAFETY: IRQs are masked, this task owns hardware FP, and this
        // image came from FXSAVE on that task, never from user memory.
        unsafe { active.fp.restore() };
        if option_env!("SUNLIGHT_HELIOS_SIG01_TRACE") == Some("1") {
            trace("sigreturn", process.pid, &regs);
        }
        regs
    };
    // SAFETY: trusted local 15-GPR + five-word return frame, IRQs still
    // masked. Validation and commit completed; ordinary SYSRET is bypassed.
    unsafe { iretq_to_context(regs.as_ptr() as u64) }
}

fn trace(event: &str, tid: usize, regs: &[u64; 20]) {
    crate::serial_println!(
        "[SIG01] {} tid={} rip={:#x} flags={:#x} rsp={:#x} rcx={:#x} r11={:#x}",
        event,
        tid,
        regs[15],
        regs[17],
        regs[18],
        regs[13],
        regs[6]
    );
}

fn error(step: &str, error: user::UserMemoryError) {
    if option_env!("SUNLIGHT_HELIOS_SIG01_TRACE") == Some("1") {
        crate::serial_println!("[SIG01] {} rejected: {:?}", step, error);
    }
}
