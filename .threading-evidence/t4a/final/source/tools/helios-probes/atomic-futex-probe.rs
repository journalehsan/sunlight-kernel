//! Freestanding ring-3 concurrency control; static stacks/TLS remain owner-owned.
#![no_std]
#![no_main]
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static STARTED: AtomicU32 = AtomicU32::new(0);
static RESUMED: AtomicU32 = AtomicU32::new(0);
static RELEASE: AtomicU32 = AtomicU32::new(0);
static OTHER: AtomicU32 = AtomicU32::new(0);
static WORD: AtomicU32 = AtomicU32::new(0);
static TRIGGER: AtomicU32 = AtomicU32::new(0);
static CASE: AtomicU64 = AtomicU64::new(0);
static ADDRESS: AtomicU64 = AtomicU64::new(0);
static RESULTS: [AtomicU64; 32] = [const { AtomicU64::new(u64::MAX) }; 32];
#[repr(align(16))]
struct Backing([u8; 32 * 32768]);
static mut STACKS: Backing = Backing([0; 32 * 32768]);
static mut TLS: [u64; 32] = [0; 32];
const MAPPED: u64 = 0x100000000;

unsafe fn sys(n: u64, a: u64, b: u64, c: u64, d: u64) -> u64 {
    let result: u64;
    core::arch::asm!("syscall",inlateout("rax")n=>result,in("rdi")a,in("rsi")b,in("rdx")c,in("r10")d,
        lateout("rcx")_,lateout("r11")_,options(nostack));
    result
}
fn yield_now() {
    query("T4A yield");
}
fn query(s: &str) -> u64 {
    unsafe { sys(99, s.as_ptr() as u64, s.len() as u64, 0, 0) }
}
fn fail() -> ! {
    query("T4A fail");
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
fn spin_until(mut f: impl FnMut() -> bool) {
    for _ in 0..200_000_000 {
        if f() {
            return;
        }
        core::hint::spin_loop();
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
fn arm(case: u64, address: u64) {
    let mut msg = Message {
        bytes: [0; 128],
        len: 0,
    };
    write!(
        &mut msg,
        "T4A arm {} {} {}",
        case, address, &TRIGGER as *const _ as u64
    )
    .unwrap();
    check(query(core::str::from_utf8(&msg.bytes[..msg.len]).unwrap()) == 0);
}
fn futex_at(address: u64, op: u64, count: u64, deadline: u64) -> u64 {
    let linux = CASE.load(Ordering::Acquire) >= 17;
    unsafe {
        sys(
            if linux { 202 } else { 155 },
            address,
            op | if linux { 0x80 } else { 0 },
            count,
            if linux && deadline == u64::MAX {
                0
            } else {
                deadline
            },
        )
    }
}
fn futex(op: u64, count: u64, deadline: u64) -> u64 {
    futex_at(ADDRESS.load(Ordering::Acquire), op, count, deadline)
}
fn mapped_word() -> &'static AtomicU32 {
    unsafe { &*(MAPPED as *const AtomicU32) }
}
fn map(linux: bool) {
    // Native mmap takes fd/offset in r8/r9; use all six arguments explicitly.
    let r: u64;
    unsafe {
        core::arch::asm!("syscall",inlateout("rax")(if linux {9u64} else {50u64})=>r,in("rdi")MAPPED,in("rsi")4096u64,
        in("rdx")3u64,in("r10")0x32u64,in("r8")u64::MAX,in("r9")0u64,
        lateout("rcx")_,lateout("r11")_,options(nostack));
    }
    check(r == MAPPED);
}
fn clock() -> u64 {
    let mut time = [0u64; 2];
    check(
        unsafe {
            sys(
                if CASE.load(Ordering::Acquire) >= 17 {
                    228
                } else {
                    88
                },
                1,
                time.as_mut_ptr() as u64,
                0,
                0,
            )
        } == 0,
    );
    time[0] * 1_000_000_000 + time[1]
}
#[unsafe(naked)]
extern "C" fn trampoline(_func: u64, _role: u64) -> ! {
    core::arch::naked_asm!("and rsp,-16","call {worker}","ud2",worker=sym worker);
}
extern "C" fn worker(_func: u64, role: u64) -> ! {
    let canary = 0x12345678abcdef00u64 ^ role;
    unsafe {
        core::arch::asm!("mov fs:[0],{}",in(reg)canary,options(nostack));
    }
    let epoch = CASE.load(Ordering::Acquire);
    let linux = epoch >= 17;
    let case = epoch % 17;
    if linux {
        check(query("T4A linux") == 0);
    }
    if case == 0 && role == 0 {
        let invalid = if linux { (-14i64) as u64 } else { u64::MAX - 5 };
        check(futex_at(0, 0, 0, u64::MAX) == invalid);
        check(futex_at(0xffff800000000000, 0, 0, u64::MAX) == invalid);
        check(futex_at(0x0001000000000000, 0, 0, u64::MAX) == invalid);
        check(
            futex_at(ADDRESS.load(Ordering::Acquire) + 1, 0, 0, u64::MAX)
                == if linux { (-22i64) as u64 } else { invalid },
        );
    }
    if case >= 15 {
        let address = if case == 16 && role >= 16 {
            &OTHER as *const _ as u64
        } else {
            &WORD as *const _ as u64
        };
        check(futex_at(address, 0, 0, u64::MAX) == 0);
        if !linux {
            check(unsafe { sys(155, address, 2, 0, 0) } == 0);
        }
        RESUMED.fetch_add(1, Ordering::AcqRel);
        until(|| RELEASE.load(Ordering::Acquire) == 1);
        RESULTS[role as usize].store(0, Ordering::Release);
    } else if role != 1 {
        if (1..=5).contains(&case) {
            until(|| STARTED.load(Ordering::Acquire) == 1);
        }
        let cycles = if case == 14 { 128 } else { 1 };
        for _ in 0..cycles {
            if case == 14 {
                WORD.store(0, Ordering::Release);
            }
            let deadline = if case == 8 {
                if linux {
                    1
                } else {
                    clock() + 100_000_000
                }
            } else if case == 9 && !linux {
                clock() + 100_000_000
            } else {
                u64::MAX
            };
            let result = futex(0, 0, deadline);
            if !linux && result == 0 {
                if case == 9 {
                    until(|| clock() >= deadline);
                }
                let receipt = unsafe { sys(155, ADDRESS.load(Ordering::Acquire), 2, 0, 0) };
                check(receipt == if case == 8 { 2 } else { 0 });
                check(
                    unsafe { sys(155, ADDRESS.load(Ordering::Acquire), 2, 0, 0) } == u64::MAX - 5,
                );
            }
            check(
                result
                    == if case == 0 || case == 1 {
                        if linux {
                            (-11i64) as u64
                        } else {
                            1
                        }
                    } else if case == 8 && linux {
                        (-38i64) as u64
                    } else {
                        0
                    },
            );
            RESULTS[role as usize].store(result, Ordering::Release);
        }
    } else {
        STARTED.store(1, Ordering::Release);
        if (1..=5).contains(&case) {
            spin_until(|| TRIGGER.load(Ordering::Acquire) == 1);
            WORD.store(1, Ordering::Release);
            check(futex(1, 1, 0) == if case == 1 { 0 } else { 1 });
        } else if case == 0 {
            check(futex(1, 1, 0) == 0);
        } else {
            let cycles = if case == 14 { 128 } else { 1 };
            for _ in 0..cycles {
                until(|| query("T4A ready") == 1);
                match case {
                    7 => {
                        check(futex(1, 0, 0) == 0);
                        check(futex(1, 1, 0) == 1);
                        check(query("T4A remaining") == 1);
                        check(futex(1, 1, 0) == 1);
                        check(futex(1, 99, 0) == 0);
                    }
                    8 => {
                        check(futex(1, 1, 0) == 0);
                    }
                    10 => {
                        check(query("T4A cancel") == 0);
                        check(futex(1, 1, 0) == 0);
                    }
                    11 => {
                        check(unsafe { sys(if linux { 11 } else { 51 }, MAPPED, 4096, 0, 0) } == 0);
                        map(linux);
                        check(futex(1, 1, 0) == 0);
                    }
                    12 => {
                        map(linux);
                        check(futex(1, 1, 0) == 0);
                    }
                    13 => {
                        check(query("T4A kill") == 0);
                        check(futex(1, 1, 0) == 0);
                    }
                    _ => {
                        // Publication happens before WAKE on the live shared word.
                        WORD.store(1, Ordering::Release);
                        check(futex(1, 1, 0) == 1);
                        if case == 9 {
                            check(query("T4A cancel") == 0);
                        }
                    }
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
        sys(if linux { 60 } else { 20 }, 0, 0, 0, 0);
    }
    fail()
}
#[unsafe(naked)]
#[no_mangle]
extern "C" fn _start() -> ! {
    core::arch::naked_asm!("and rsp,-16","call {main}","ud2",main=sym main);
}
extern "C" fn main() -> ! {
    check(query("T4A unsupported") == 0);
    map(false);
    map(false); // Warm the existing one-entry replacement/reclaim-candidate cache.
    check(query("T4A baseline") == 0);
    for epoch in 0..34u64 {
        let case = epoch % 17;
        let linux = epoch >= 17;
        CASE.store(epoch, Ordering::Release);
        WORD.store(u32::from(case == 0), Ordering::Release);
        TRIGGER.store(0, Ordering::Release);
        STARTED.store(0, Ordering::Release);
        RESUMED.store(0, Ordering::Release);
        RELEASE.store(0, Ordering::Release);
        OTHER.store(0, Ordering::Release);
        for result in &RESULTS {
            result.store(u64::MAX, Ordering::Release);
        }
        if case == 11 || case == 12 {
            mapped_word().store(0, Ordering::Release);
        }
        let address = if case == 11 || case == 12 {
            MAPPED
        } else {
            &WORD as *const _ as u64
        };
        ADDRESS.store(address, Ordering::Release);
        arm(epoch, address);
        let mut tids = [0u64; 32];
        let roles = if case >= 15 { 32 } else { 3 };
        for ordinal in 0..roles {
            let role = if case >= 15 {
                ordinal
            } else {
                [0usize, 2, 1][ordinal]
            };
            if role == 2 && case != 7 && case < 15 {
                continue;
            }
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
        if case >= 15 {
            if linux {
                check(query("T4A linux") == 0);
            }
            until(|| query("T4A ready-all") == 1);
            if case == 15 {
                if linux {
                    check(futex(1, u32::MAX as u64, 0) == (-22i64) as u64);
                }
                check(futex_at(&OTHER as *const _ as u64, 1, 99, 0) == 0);
                for (limit, expected) in [
                    (0, 0),
                    (1, 1),
                    (4, 4),
                    (
                        if linux {
                            i32::MAX as u64
                        } else {
                            u32::MAX as u64
                        },
                        27,
                    ),
                    (1, 0),
                ] {
                    let count = futex(1, limit, 0);
                    check(count == expected);
                    let mut msg = Message {
                        bytes: [0; 128],
                        len: 0,
                    };
                    write!(
                        &mut msg,
                        "T4A cardinality {} {} {}",
                        limit,
                        count,
                        RESUMED.load(Ordering::Acquire)
                    )
                    .unwrap();
                    check(query(core::str::from_utf8(&msg.bytes[..msg.len]).unwrap()) == 0);
                    let total = if limit == 0 {
                        0
                    } else if limit == 1 && expected == 1 {
                        1
                    } else if limit == 4 {
                        5
                    } else {
                        32
                    };
                    until(|| RESUMED.load(Ordering::Acquire) == total);
                }
            } else {
                check(
                    futex(
                        1,
                        if linux {
                            i32::MAX as u64
                        } else {
                            u32::MAX as u64
                        },
                        0,
                    ) == 16,
                );
                until(|| RESUMED.load(Ordering::Acquire) == 16);
                check(query("T4A remaining") == 16);
                check(futex(1, 1, 0) == 0);
                check(
                    futex_at(
                        &OTHER as *const _ as u64,
                        1,
                        if linux {
                            i32::MAX as u64
                        } else {
                            u32::MAX as u64
                        },
                        0,
                    ) == 16,
                );
                until(|| RESUMED.load(Ordering::Acquire) == 32);
            }
            RELEASE.store(1, Ordering::Release);
            if linux {
                check(query("T4A native") == 0);
            }
        }
        until(|| {
            tids.iter()
                .filter(|t| **t != 0)
                .all(|t| unsafe { sys(156, *t, 0, 0, 0) } == 1)
        });
        check(RESULTS[1].load(Ordering::Acquire) == 0);
        if case != 13 {
            check(RESULTS[0].load(Ordering::Acquire) != u64::MAX);
        }
        if case == 7 {
            check(RESULTS[2].load(Ordering::Acquire) == 0);
        }
        until(|| query("T4A verify") == 1);
    }
    query("T4A finish");
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
