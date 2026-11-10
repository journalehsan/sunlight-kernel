#ifndef SUNLIGHT_SYS_RANDOM_H
#define SUNLIGHT_SYS_RANDOM_H
#include <stddef.h>
#ifndef SUNLIGHT_SSIZE_T_DEFINED
#define SUNLIGHT_SSIZE_T_DEFINED
typedef long ssize_t;
#endif
#ifdef __cplusplus
extern "C" {
#endif
/* Native LP64: size_t is unsigned long, ssize_t is signed long, flags is
 * unsigned int (32 bits). Only flags == 0 is supported, through the OS rand
 * service. GRND_NONBLOCK and GRND_RANDOM are deliberately not advertised:
 * every nonzero flag is rejected with EINVAL, including libc's private Rust
 * NONCRYPTO bit. Service failures return -1/EIO; success preserves errno.
 * Zero length returns 0 without accessing buf or contacting the service.
 * Nonempty buffers must designate buflen writable bytes; NULL fails EFAULT.
 * This userspace IPC adapter cannot recover arbitrary mapping/protection
 * faults as EFAULT. Counts above the signed size range fail EINVAL. */
ssize_t getrandom(void *buf, size_t buflen, unsigned int flags);
#ifdef __cplusplus
}
#endif
#endif
