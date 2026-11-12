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
 * AF_INET/SOCK_STREAM descriptors attached to net_server report real listener
 * accept, stream read/EOF and write-capacity readiness. POLLERR/POLLHUP are
 * unconditional; POLLNVAL remains the kernel descriptor-table result. Socket
 * OOB data produces POLLPRI. Pipes, TTY input, eventfd, VFS files and epoll
 * descriptors retain the kernel readiness scan. epoll is not bridged to TCP.
 * No signal interruption is implemented.
 * Invalid non-NULL user mappings retain the native EIO copy-error result. */
int poll(struct pollfd *fds, nfds_t nfds, int timeout);
#ifdef __cplusplus
}
#endif
#endif
