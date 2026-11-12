#ifndef SUNLIGHT_CXX_SYS_SYSCALL_H
#define SUNLIGHT_CXX_SYS_SYSCALL_H
/* Private adapter for the two syscall-shaped operations used by the vendored
 * LLVM cxa_guard_impl.h platform layer.
 *
 * This is not a public libc syscall() wrapper. It is reachable only from the
 * guard translation unit, and unknown numbers return -1 instead of pretending
 * to succeed. The SYS_* values are private selector tags, not numbers passed to
 * the kernel. SYS_gettid is answered from a native per-thread identity; the
 * futex operations map onto the native private-address-space futex syscall
 * (155) through guard_support.cpp. */
#include <stdarg.h>
#include <stdint.h>

typedef int pid_t;

#define SYS_gettid 186
#define SYS_futex 202

#ifdef __cplusplus
extern "C" {
uint32_t __sl_cxx_guard_thread_id(void);
long __sl_cxx_native_futex_wait(int* address, int expected);
long __sl_cxx_native_futex_wake(int* address, int count);

static inline long syscall(long number, ...) {
    if (number == SYS_gettid)
        return static_cast<long>(__sl_cxx_guard_thread_id());
    va_list arguments;
    va_start(arguments, number);
    int* address = va_arg(arguments, int*);
    int operation = va_arg(arguments, int);
    int value = va_arg(arguments, int);
    va_end(arguments);
    if (number == SYS_futex) {
        if (operation == 0)
            return __sl_cxx_native_futex_wait(address, value);
        if (operation == 1)
            return __sl_cxx_native_futex_wake(address, value);
    }
    return -1;
}
}
#endif

#endif
