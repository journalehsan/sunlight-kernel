#ifndef SUNLIGHT_SYS_EPOLL_H
#define SUNLIGHT_SYS_EPOLL_H
#ifdef __cplusplus
extern "C" {
#endif
#define EPOLLIN  0x001
#define EPOLLPRI 0x002
#define EPOLLOUT 0x004
#define EPOLLERR 0x008
#define EPOLLHUP 0x010
#define EPOLLONESHOT 0x40000000u
#define EPOLL_CLOEXEC 0x00080000
#define EPOLLET  0x80000000u
#define EPOLL_CTL_ADD 1
#define EPOLL_CTL_DEL 2
#define EPOLL_CTL_MOD 3
/* Native LP64 record: 16 bytes, data aligned to eight bytes. */
struct epoll_event {
    unsigned int events;
    unsigned long long data;
};
int epoll_create1(int flags);
int epoll_ctl(int epfd, int op, int fd, struct epoll_event *event);
int epoll_wait(int epfd, struct epoll_event *events, int maxevents, int timeout);
#ifdef __cplusplus
}
#endif
#endif
