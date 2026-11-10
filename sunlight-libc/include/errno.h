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
#define EBADF 9
#define EAGAIN 11
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EISDIR 21
#define ENOTDIR 20
#define EINVAL 22
#define ENOSPC 28
#define EPIPE 32
#define ENOSYS 38
#ifdef __cplusplus
}
#endif
#endif
