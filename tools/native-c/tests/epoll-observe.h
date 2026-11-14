/* Readiness adapter for running the unchanged pressure assertions through
 * native epoll. One persistent instance/registration, full-width user data. */
#include <sys/epoll.h>
#include <poll.h>
#include <errno.h>
#include <unistd.h>
static int epoll_observe(struct pollfd *record, unsigned long count, int timeout) {
    static int ep = -1, previous = -1;
    if (count != 1) { errno = EINVAL; return -1; }
    if (ep < 0) ep = epoll_create1(0);
    if (ep < 0) return -1;
    struct epoll_event interest = {.events = (unsigned)record->events,
        .data = 0xfedcba9876543210ULL}, result;
    if (previous != record->fd) {
        if (previous >= 0) (void)epoll_ctl(ep, EPOLL_CTL_DEL, previous, 0);
        previous = record->fd;
    }
    if (epoll_ctl(ep, EPOLL_CTL_MOD, record->fd, &interest) != 0) {
        if (errno != ENOENT || epoll_ctl(ep, EPOLL_CTL_ADD, record->fd, &interest)) return -1;
    }
    int n = epoll_wait(ep, &result, 1, timeout);
    record->revents = 0;
    if (n == 1) {
        if (result.data != interest.data) { errno = EINVAL; return -1; }
        record->revents = (short)result.events;
        if (result.events & (EPOLLERR | EPOLLHUP)) {
            interest.events = 0;
            struct epoll_event terminal;
            if (epoll_ctl(ep, EPOLL_CTL_MOD, record->fd, &interest) ||
                epoll_wait(ep, &terminal, 1, 0) != 1 ||
                terminal.data != interest.data ||
                !(terminal.events & (EPOLLERR | EPOLLHUP)) ||
                (terminal.events & ~(EPOLLERR | EPOLLHUP))) { errno = EINVAL; return -1; }
        }
    }
    return n;
}
#define poll epoll_observe
