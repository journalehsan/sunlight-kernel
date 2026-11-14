#ifndef SUNLIGHT_ERRNO_H
#define SUNLIGHT_ERRNO_H
#ifdef __cplusplus
extern "C" {
#endif
int *__errno_location(void);
#define errno (*__errno_location())
#define EPERM 1
#define ENOENT 2
#define EIO 5
#define E2BIG 7
#define EOVERFLOW 75
#define EBADF 9
#define EAGAIN 11
#define EWOULDBLOCK EAGAIN
/* Reserved native C errno numbers, not promises of signal/device support. */
#define ESRCH 3
#define EINTR 4
#define ENXIO 6
#define ECHILD 10
#define ENODEV 19
#define ESPIPE 29
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EBUSY 16
#define EEXIST 17
#define EISDIR 21
#define ENOTDIR 20
#define EILSEQ 84
#define EINVAL 22
#define EDEADLK 35
#define EDOM 33
#define ERANGE 34
#define ENOSPC 28
#define EPIPE 32
#define ENAMETOOLONG 36
#define ENOSYS 38
#define ENOTTY 25
#define EOPNOTSUPP 95
#define ENOTSUP EOPNOTSUPP
#define EMFILE 24
#define EPROTONOSUPPORT 93
#define ESOCKTNOSUPPORT 94
#define EAFNOSUPPORT 97
#define EADDRINUSE 98
#define ENETUNREACH 101
#define ECONNABORTED 103
#define ECONNRESET 104
#define EISCONN 106
#define ENOBUFS 105
#define ENOTCONN 107
#define ETIMEDOUT 110
#define ECONNREFUSED 111
#define EHOSTUNREACH 113
#define EALREADY 114
#define EINPROGRESS 115
#ifdef __cplusplus
}
#endif
#endif
