#ifdef REVERSE_INCLUDES
#include <stddef.h>
#include <time.h>
#endif
#include <poll.h>
#include <sys/select.h>
#include <sys/epoll.h>
#include <sys/socket.h>
#include <poll.h>
#include <sys/select.h>
#ifndef REVERSE_INCLUDES
#include <stddef.h>
#include <time.h>
#endif
#ifdef __cplusplus
template<class A, class B> struct same { static const bool value = false; };
template<class A> struct same<A, A> { static const bool value = true; };
#define TYPE(expr, type) static_assert((same<decltype(expr), type>::value), #expr)
#define ASSERT static_assert
#else
#define TYPE(expr, type) _Static_assert(_Generic((expr), type: 1, default: 0), #expr)
#define ASSERT _Static_assert
#endif
TYPE((nfds_t)0, unsigned long);
TYPE(((struct pollfd*)0)->fd, int);
TYPE(((struct pollfd*)0)->events, short);
TYPE(((struct pollfd*)0)->revents, short);
TYPE(&poll, int (*)(struct pollfd *, nfds_t, int));
TYPE(&select, int (*)(int, fd_set *, fd_set *, fd_set *, struct timeval *));
TYPE(&socketpair, int (*)(int, int, int, int *));
TYPE(&send, ssize_t (*)(int, const void *, size_t, int));
TYPE(&recv, ssize_t (*)(int, void *, size_t, int));
TYPE(&epoll_wait, int (*)(int, struct epoll_event *, int, int));
ASSERT(sizeof(struct pollfd) == 8, "pollfd size");
ASSERT(__alignof__(struct pollfd) == 4, "pollfd alignment");
ASSERT(offsetof(struct pollfd, fd) == 0, "fd offset");
ASSERT(offsetof(struct pollfd, events) == 4, "events offset");
ASSERT(offsetof(struct pollfd, revents) == 6, "revents offset");
ASSERT(sizeof(nfds_t) == 8 && (nfds_t)-1 > 0, "unsigned LP64 count");
ASSERT(POLLIN == 1 && POLLPRI == 2 && POLLOUT == 4 && POLLERR == 8 && POLLHUP == 16 && POLLNVAL == 32, "event ABI");
ASSERT(EPOLLIN == 1 && EPOLLPRI == 2 && EPOLLOUT == 4 && EPOLLERR == 8 && EPOLLHUP == 16, "epoll ABI");
ASSERT(sizeof(struct epoll_event) == 16, "epoll event size");
ASSERT(__alignof__(struct epoll_event) == 8 && offsetof(struct epoll_event, data) == 8, "epoll alignment");
ASSERT(EPOLLET == 0x80000000u && EPOLLONESHOT == 0x40000000u, "epoll modes");
ASSERT(EPOLL_CTL_ADD == 1 && EPOLL_CTL_DEL == 2 && EPOLL_CTL_MOD == 3, "epoll commands");
TYPE(&epoll_create1, int (*)(int));
TYPE(&epoll_ctl, int (*)(int, int, int, struct epoll_event *));
ASSERT(MSG_OOB == 1 && MSG_DONTWAIT == 0x40, "message flags");
ASSERT(sizeof(fd_set) == 128 && sizeof(struct timeval) == 16, "select LP64");
int use_sets(int fd) {
    fd_set s;
    FD_ZERO(&s); FD_SET(fd, &s); FD_CLR(fd, &s);
    return FD_ISSET(fd, &s);
}
