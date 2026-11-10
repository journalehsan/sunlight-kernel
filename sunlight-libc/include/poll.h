#ifndef SUNLIGHT_POLL_H
#define SUNLIGHT_POLL_H
/* Native LP64 ABI, shared directly with syscall 159. */
typedef unsigned long nfds_t;
struct pollfd { int fd; short events; short revents; };
#define POLLIN   0x001
#define POLLPRI  0x002
#define POLLOUT  0x004
#define POLLERR  0x008
#define POLLHUP  0x010
#define POLLNVAL 0x020
#ifdef __cplusplus
extern "C" {
#endif
/* Up to 256 entries. Negative fds are ignored. Any negative timeout waits
 * indefinitely; positive timeouts use the native 10 ms scheduler clock.
 * Socket OOB data produces POLLPRI. Socket and epoll descriptors participate
 * in the same readiness scan as pipes, TTY input, eventfd and VFS files.
 * No signal interruption is implemented.
 * Invalid non-NULL user mappings retain the native EIO copy-error result. */
int poll(struct pollfd *fds, nfds_t nfds, int timeout);
#ifdef __cplusplus
}
#endif
#endif
