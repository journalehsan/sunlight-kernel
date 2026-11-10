#ifndef SUNLIGHT_SELECT_H
#define SUNLIGHT_SELECT_H
#define FD_SETSIZE 1024
typedef struct { unsigned long __bits[FD_SETSIZE / 64]; } fd_set;
struct timeval { long tv_sec; long tv_usec; };
/* Arguments must designate valid storage, and fd must be in [0,FD_SETSIZE). */
#define FD_ZERO(set) ((void)__builtin_memset((set), 0, sizeof(fd_set)))
#define FD_SET(fd,set) ((void)((set)->__bits[(fd)/64] |= 1UL << ((fd)%64)))
#define FD_CLR(fd,set) ((void)((set)->__bits[(fd)/64] &= ~(1UL << ((fd)%64))))
#define FD_ISSET(fd,set) (!!((set)->__bits[(fd)/64] & (1UL << ((fd)%64))))
#ifdef __cplusplus
extern "C" {
#endif
/* Native descriptors are bounded to 256. Timeout is rounded up to milliseconds
 * and left unchanged. NULL waits indefinitely. No signal interruption. */
int select(int nfds, fd_set *readfds, fd_set *writefds, fd_set *exceptfds,
           struct timeval *timeout);
#ifdef __cplusplus
}
#endif
#endif
