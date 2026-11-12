#include <stdint.h>

namespace {
// Native futex syscall 155, mirrored from sunlight-libc/src/sys.rs:
// rdi=u32 address, rsi=operation (0 wait, 1 wake, 2 consume result),
// rdx=value/count, r10=absolute monotonic deadline (u64::MAX = indefinite).
constexpr long kNativeFutex = 155;

inline long native_futex(uint32_t* address, uint32_t operation, uint32_t value,
                         uint64_t deadline) {
    long result;
    register uint64_t deadline_register asm("r10") = deadline;
    asm volatile("syscall"
                 : "=a"(result)
                 : "a"(kNativeFutex), "D"(address), "S"(operation), "d"(value),
                   "r"(deadline_register)
                 : "rcx", "r11", "memory");
    return result;
}

// A unique 32-bit identity per native thread. Constant-initialized TLS needs
// no guard or destructor, so this cannot recurse through the guard runtime.
uint32_t next_guard_thread_id = 1;
thread_local uint32_t guard_thread_id;
}

extern "C" uint32_t __sl_cxx_guard_thread_id(void) {
    if (guard_thread_id == 0)
        guard_thread_id = __atomic_fetch_add(&next_guard_thread_id, 1, __ATOMIC_RELAXED);
    return guard_thread_id;
}

extern "C" long __sl_cxx_native_futex_wait(int* address, int expected) {
    auto* word = reinterpret_cast<uint32_t*>(address);
    long result = native_futex(word, 0, static_cast<uint32_t>(expected), ~uint64_t{0});
    if (result == 0)
        result = native_futex(word, 2, 0, 0);
    return result;
}

extern "C" long __sl_cxx_native_futex_wake(int* address, int count) {
    return native_futex(reinterpret_cast<uint32_t*>(address), 1,
                        static_cast<uint32_t>(count), 0);
}
