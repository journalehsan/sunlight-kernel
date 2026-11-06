//! Bounded native ring-3 T4B lifecycle fixture; static owner-owned stacks/TLS.
#![no_std]
#![no_main]
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
static TICKET: AtomicU64 = AtomicU64::new(0);
static ELECT: AtomicU64 = AtomicU64::new(u64::MAX);
static WORD: AtomicU32 = AtomicU32::new(0);
static OTHER: AtomicU32 = AtomicU32::new(0);
static CASE: AtomicU64 = AtomicU64::new(0);
static DEADLINE: AtomicU64 = AtomicU64::new(0);
static GO: AtomicU32 = AtomicU32::new(0);
static STOP: AtomicU32 = AtomicU32::new(0);
static PROGRESS: AtomicU64 = AtomicU64::new(0);
static RESULTS: [AtomicU64; 32] = [const { AtomicU64::new(u64::MAX) }; 32];
#[repr(align(16))]
struct Backing([u8; 32 * 32768]);
static mut STACKS: Backing = Backing([0; 32 * 32768]);
static mut TLS: [u64; 32] = [0; 32];
unsafe fn sys(n: u64, a: u64, b: u64, c: u64, d: u64) -> u64 {
    let result: u64;
    core::arch::asm!("syscall",inlateout("rax")n=>result,in("rdi")a,in("rsi")b,in("rdx")c,in("r10")d,
        lateout("rcx")_,lateout("r11")_,options(nostack));
    result
}
fn yield_now() {
    query("T4B yield");
}
fn query(s: &str) -> u64 {
    unsafe { sys(99, s.as_ptr() as u64, s.len() as u64, 0, 0) }
}
fn fail() -> ! {
    query("T4B fail");
    loop {
        yield_now();
    }
}
fn check(ok: bool) {
    if !ok {
        fail();
    }
}
fn until(mut f: impl FnMut() -> bool) {
    for _ in 0..200000 {
        if f() {
            return;
        }
        yield_now();
    }
    fail();
}
struct Message {
    bytes: [u8; 128],
    len: usize,
}
impl Write for Message {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.len + s.len() > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
        Ok(())
    }
}
fn message(args: fmt::Arguments<'_>) -> u64 {
    let mut m = Message {
        bytes: [0; 128],
        len: 0,
    };
    m.write_fmt(args).unwrap();
    query(core::str::from_utf8(&m.bytes[..m.len]).unwrap())
}
fn address(other: bool) -> u64 {
    if other {
        &OTHER as *const _ as u64
    } else {
        &WORD as *const _ as u64
    }
}
fn futex(other: bool, op: u64, count: u64, deadline: u64) -> u64 {
    unsafe { sys(155, address(other), op, count, deadline) }
}
fn clock() -> u64 {
    let mut t = [0u64; 2];
    check(unsafe { sys(88, 1, t.as_mut_ptr() as u64, 0, 0) } == 0);
    t[0] * 1_000_000_000 + t[1]
}
#[unsafe(naked)]
extern "C" fn trampoline(_: u64, _: u64) -> ! {
    core::arch::naked_asm!("and rsp,-16","call {worker}","ud2",worker=sym worker);
}
fn waker_calls() -> u64 {
    let mut total = 0;
    for limit in [0, 1, 4, 1, 4, 1, 32, 1] {
        let selected = futex(false, 1, limit, 0);
        check(selected <= limit);
        total += selected;
        check(message(format_args!("T4B cardinality {} {}", limit, selected)) == 0);
        PROGRESS.fetch_add(1, Ordering::AcqRel);
    }
    total
}
extern "C" fn worker(_: u64, role: u64) -> ! {
    let case = CASE.load(Ordering::Acquire);
    let canary = 0x12345678abcdef00u64 ^ role;
    unsafe {
        core::arch::asm!("mov fs:[0],{}",in(reg)canary,options(nostack));
    }
    let progress = case != 8 && role == 2;
    if progress {
        while STOP.load(Ordering::Acquire) == 0 {
            PROGRESS.fetch_add(1, Ordering::AcqRel);
            yield_now();
        }
        RESULTS[role as usize].store(0, Ordering::Release);
    } else if case == 8 {
        check(futex(false, 0, 0, u64::MAX) == 0);
        check(futex(false, 2, 0, 0) == 0);
        check(futex(false, 2, 0, 0) == u64::MAX - 5);
        let ticket = TICKET.fetch_add(1, Ordering::AcqRel);
        let result = if ticket == 0 {
            until(|| query("T4B waker") == 1);
            ELECT.store(role, Ordering::Release);
            until(|| GO.load(Ordering::Acquire) == 1);
            waker_calls()
        } else if ticket == 1 {
            check(futex(true, 0, 0, u64::MAX) == 0);
            check(futex(true, 2, 0, 0) == 0);
            0
        } else {
            0
        };
        RESULTS[role as usize].store(result, Ordering::Release);
    } else if role == 0 {
        let cycles = if case == 9 { 16 } else { 1 };
        for _ in 0..cycles {
            let deadline = if matches!(case, 0 | 1 | 2 | 3 | 5 | 9) {
                clock() + 200_000_000
            } else {
                u64::MAX
            };
            DEADLINE.store(deadline, Ordering::Release);
            check(futex(false, 0, 0, deadline) == 0);
            if case == 0 {
                until(|| clock() >= deadline);
            }
            let receipt = futex(false, 2, 0, 0);
            check(if case == 1 || case == 9 {
                receipt == 2
            } else if case == 2 || case == 5 {
                receipt == 0 || receipt == 2
            } else {
                receipt == 0
            });
            check(futex(false, 2, 0, 0) == u64::MAX - 5);
            RESULTS[0].store(receipt, Ordering::Release);
        }
    } else {
        until(|| query("T4B ready") == 1);
        match case {
            1 | 9 => {
                if case == 1 {
                    check(query("T4B cancel") == 0);
                }
                check(futex(false, 1, 1, 0) == 0);
            }
            3 | 10 => {
                check(
                    query(if case == 10 {
                        "T4B spurious"
                    } else {
                        "T4B cancel"
                    }) == 0,
                );
                check(futex(false, 1, 1, 0) == 0);
            }
            5 => {
                until(|| clock() >= DEADLINE.load(Ordering::Acquire));
                check(query("T4B cancel") == 0);
                check(futex(false, 1, 1, 0) == 0);
            }
            6 => {
                check(query("T4B kill") == 0);
                check(futex(false, 1, 1, 0) == 0);
            }
            _ => {
                if case == 2 {
                    until(|| clock() >= DEADLINE.load(Ordering::Acquire));
                }
                if case == 7 {
                    check(query("T4B migrate") == 0);
                }
                let selected = futex(false, 1, 1, 0);
                check(if case == 2 {
                    selected <= 1
                } else {
                    selected == 1
                });
                if case == 4 {
                    check(query("T4B cancel") == 0);
                }
            }
        }
        RESULTS[1].store(0, Ordering::Release);
    }
    let observed: u64;
    unsafe {
        core::arch::asm!("mov {},fs:[0]",out(reg)observed,options(nostack));
    }
    check(observed == canary);
    unsafe {
        sys(20, 0, 0, 0, 0);
    }
    fail()
}
#[unsafe(naked)]
#[no_mangle]
extern "C" fn _start() -> ! {
    core::arch::naked_asm!("and rsp,-16","call {main}","ud2",main=sym main);
}
fn joined(tid: u64) -> bool {
    unsafe { sys(156, tid, 0, 0, 0) == 1 }
}
extern "C" fn main() -> ! {
    check(query("T4B baseline") == 0);
    for case in 0..11 {
        CASE.store(case, Ordering::Release);
        WORD.store(0, Ordering::Release);
        OTHER.store(0, Ordering::Release);
        GO.store(0, Ordering::Release);
        TICKET.store(0, Ordering::Release);
        ELECT.store(u64::MAX, Ordering::Release);
        STOP.store(0, Ordering::Release);
        PROGRESS.store(0, Ordering::Release);
        for r in &RESULTS {
            r.store(u64::MAX, Ordering::Release);
        }
        check(message(format_args!("T4B arm {} {}", case, address(false))) == 0);
        let mut tids = [0u64; 32];
        let actors = if case == 8 { 32 } else { 3 };
        for ordinal in 0..actors {
            let role = if case == 8 {
                ordinal
            } else {
                [0, 2, 1][ordinal]
            };
            let stack = unsafe {
                core::ptr::addr_of_mut!(STACKS.0)
                    .cast::<u8>()
                    .add((role + 1) * 32768 - 32) as *mut u64
            };
            unsafe {
                stack.write(worker as *const () as u64);
                stack.add(1).write(role as u64);
            }
            let tls = unsafe { core::ptr::addr_of_mut!(TLS).cast::<u64>().add(role) as u64 };
            tids[role] = unsafe { sys(22, trampoline as *const () as u64, stack as u64, tls, 0) };
            check((tids[role] as i64) > 0);
        }
        let selected = if case == 8 {
            until(|| query("T4B ready") == 1);
            check(futex(false, 1, 0, 0) == 0);
            check(futex(false, 1, 2, 0) == 2);
            until(|| ELECT.load(Ordering::Acquire) != u64::MAX);
            until(|| message(format_args!("T4B other-ready {}", address(true))) == 1);
            GO.store(1, Ordering::Release);
            let owner_selected = waker_calls();
            let elected = ELECT.load(Ordering::Acquire) as usize;
            until(|| joined(tids[elected]));
            let total = 2 + owner_selected + RESULTS[elected].load(Ordering::Acquire);
            check(total == 32);
            check(query("T4B remaining") == 1);
            check(futex(false, 1, 1, 0) == 0);
            check(futex(true, 1, 1, 0) == 1);
            until(|| tids.iter().take(32).all(|tid| joined(*tid)));
            total
        } else {
            until(|| joined(tids[0]) && joined(tids[1]));
            if case != 6 {
                check(RESULTS[0].load(Ordering::Acquire) != u64::MAX);
            }
            check(PROGRESS.load(Ordering::Acquire) > 0);
            STOP.store(1, Ordering::Release);
            until(|| joined(tids[2]));
            0
        };
        check(message(format_args!("T4B verify")) == 1);
        // Progress and userspace-reported selection are separate from kernel selection.
        message(format_args!(
            "T4B progress {} {} {}",
            case,
            PROGRESS.load(Ordering::Acquire),
            selected
        ));
    }
    query("T4B finish");
    fail()
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail()
}
// Compiler memory intrinsics: freestanding implementation without allocator.
#[no_mangle]
unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    for i in 0..n {
        dst.add(i).write_volatile(src.add(i).read_volatile());
    }
    dst
}
#[no_mangle]
unsafe extern "C" fn memset(dst: *mut u8, value: i32, n: usize) -> *mut u8 {
    for i in 0..n {
        dst.add(i).write_volatile(value as u8);
    }
    dst
}
