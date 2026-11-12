#ifndef SUNLIGHT_UNISTD_H
#define SUNLIGHT_UNISTD_H
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Native x86-64 LP64 ABI: int fd, unsigned long size_t, signed long ssize_t.
 * Deliberately a first slice: no _POSIX_VERSION or feature-option claims.
 * These base declarations do not depend on _GNU_SOURCE/_POSIX_C_SOURCE. */
#ifndef SUNLIGHT_SSIZE_T_DEFINED
#define SUNLIGHT_SSIZE_T_DEFINED
typedef long ssize_t;
#endif
#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2
/* One native transfer, with short counts returned unchanged; no retry loop.
 * Failure returns -1 and sets errno; success leaves errno unchanged.
 * Counts above signed size range fail with EINVAL. A NULL buffer is accepted
 * only for count zero. Zero count follows the native no-transfer contract;
 * it does not establish that a nonnegative descriptor exists or is writable.
 * Arbitrary bad non-NULL user memory retains the native generic EIO result. */
int isatty(int fd);
int close(int fd);
ssize_t read(int fd, void *buffer, size_t count);
ssize_t write(int fd, const void *buffer, size_t count);
#ifdef __cplusplus
}
#endif
#endif
