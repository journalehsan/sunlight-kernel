#ifndef SUNLIGHT_LIBC_RUNTIME_H
#define SUNLIGHT_LIBC_RUNTIME_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Existing native std ABI, not POSIX write/exit. A negative write result is
 * an encoded error; it does not promise POSIX -1 plus errno semantics.
 * Startup initializes the canonical runtime before calling main(argc, argv).
 * This header makes no C TLS, threading, or atexit contract. */
intptr_t sl_std_write(uint32_t fd, const unsigned char *bytes, size_t len);
#ifdef __cplusplus
[[noreturn]] void sl_std_exit(int32_t status);
#else
_Noreturn void sl_std_exit(int32_t status);
#endif

#ifdef __cplusplus
}
#endif

#endif
