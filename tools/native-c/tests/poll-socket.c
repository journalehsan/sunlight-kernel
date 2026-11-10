#include <errno.h>
#include <poll.h>
#include <sys/epoll.h>
#include <sys/socket.h>
#include <unistd.h>

static long call(long n, long a, long b, long c) {
    __asm__ volatile("syscall" : "+a"(n) : "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory");
    return n;
}

int main(void) {
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv)) return 1;

    struct pollfd p = {sv[1], POLLIN, -1};
    if (poll(&p, 1, 0) != 0 || p.revents) return 2;
    p.events = POLLOUT;
    if (poll(&p, 1, 0) != 1 || p.revents != POLLOUT) return 3;
    p.events = POLLPRI;
    if (poll(&p, 1, 0) != 0 || p.revents) return 4;

    if (send(sv[0], "Q", 1, MSG_OOB) != 1) return 5;
    p.events = POLLPRI;
    if (poll(&p, 1, 0) != 1 || p.revents != POLLPRI) return 6;
    p.events = POLLIN;
    if (poll(&p, 1, 0) != 0 || p.revents) return 7;

    char byte = 0;
    errno = 0;
    if (recv(sv[1], &byte, 1, MSG_OOB) != 1 || byte != 'Q') return 8;
    p.events = POLLPRI;
    if (poll(&p, 1, 0) != 0 || p.revents) return 9;
    errno = 0;
    if (recv(sv[1], &byte, 1, MSG_OOB) != -1 || errno != EINVAL) return 10;

    int ep = epoll_create1(0);
    if (ep < 0) return 11;
    struct epoll_event interest = {.events = EPOLLPRI, .data = 0x1234};
    if (epoll_ctl(ep, EPOLL_CTL_ADD, sv[1], &interest)) return 12;
    p = (struct pollfd){ep, POLLIN, -1};
    if (poll(&p, 1, 0) != 0 || p.revents) return 13;

    if (send(sv[0], "R", 1, MSG_OOB) != 1) return 14;
    p = (struct pollfd){ep, POLLPRI, -1};
    if (poll(&p, 1, 0) != 1 || p.revents != POLLPRI) return 15;
    struct epoll_event out = {0};
    if (epoll_wait(ep, &out, 1, 0) != 1 || out.events != EPOLLPRI ||
        out.data != 0x1234) return 16;
    if (recv(sv[1], &byte, 1, MSG_OOB) != 1 || byte != 'R') return 17;
    if (epoll_wait(ep, &out, 1, 0) != 0) return 18;

    interest.events = EPOLLIN;
    if (epoll_ctl(ep, EPOLL_CTL_MOD, sv[1], &interest)) return 19;
    if (send(sv[0], "N", 1, 0) != 1) return 20;
    if (epoll_wait(ep, &out, 1, 0) != 1 || out.events != EPOLLIN) return 21;
    if (recv(sv[1], &byte, 1, 0) != 1 || byte != 'N') return 22;

    if (call(41, sv[0], 0, 0)) return 23;
    p = (struct pollfd){sv[1], POLLIN, -1};
    if (poll(&p, 1, 0) != 1 || p.revents != (POLLIN | POLLHUP)) return 24;
    if (call(41, sv[1], 0, 0) || call(41, ep, 0, 0)) return 25;
    return 0;
}
