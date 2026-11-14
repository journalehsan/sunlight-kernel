#include <sys/socket.h>
#include <sys/epoll.h>
#include <sys/select.h>
#include <poll.h>
#include <pthread.h>
#include <errno.h>
#include <unistd.h>
#include <string.h>
static long call(long n, long a, long b, long c) {
    __asm__ volatile("syscall" : "+a"(n) : "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return n;
}
static int inherited;
static void *worker(void *unused) {
    (void)unused;
    char c;
    if (recv(inherited, &c, 1, 0) != -1 || errno != ENOTCONN) return (void *)1;
    if (close(inherited)) return (void *)2;
    return 0;
}
int main(int argc, char **argv) {
    if (argc > 1 && !strcmp(argv[1], "leak")) {
        for (int i = 0; i < 100; ++i) if (socket(AF_INET, SOCK_STREAM, 0) < 0) return 1;
        return 0; /* Reaper must release these; independent kernel counter checks it. */
    }
    if (socket(AF_UNIX, SOCK_STREAM, 0) != -1 || errno != EAFNOSUPPORT) return 2;
    if (socket(-1, SOCK_STREAM, 0) != -1 || errno != EAFNOSUPPORT) return 3;
    if (socket(AF_INET, 2, 0) != -1 || errno != ESOCKTNOSUPPORT) return 4;
    if (socket(AF_INET, SOCK_STREAM, 17) != -1 || errno != EPROTONOSUPPORT) return 5;
    if (socket(AF_INET, SOCK_STREAM | 0x1000, 0) != -1 || errno != EINVAL) return 6;
    if (socket(AF_INET, SOCK_STREAM, -1) != -1 || errno != EPROTONOSUPPORT) return 7;
    if (send(-1, "x", 1, 0) != -1 || errno != EBADF) return 8;
    for (int i = 0; i < 300; ++i) {
        int fd = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, IPPROTO_TCP);
        if (fd < 0) return 9;
        if (call(49, fd, 1, 0) != 1 || !(call(49, fd, 3, 0) & SOCK_NONBLOCK)) return 10;
        char c;
        if (send(fd, "x", 1, 0) != -1 || errno != ENOTCONN) return 11;
        if (recv(fd, &c, 1, MSG_DONTWAIT) != -1 || errno != ENOTCONN) return 12;
        if (read(fd, &c, 1) != -1 || errno != ENOTCONN) return 13;
        if (write(fd, "x", 1) != -1 || errno != ENOTCONN) return 14;
        /* No pointer is consumed for unconnected I/O, including overflow. */
        if (send(fd, (void *)-2UL, 8, 0) != -1 || errno != ENOTCONN) return 15;
        if (recv(fd, (void *)1, 8, 0) != -1 || errno != ENOTCONN) return 16;
        if (send(fd, 0, 1, 0) != -1 || errno != EFAULT) return 17;
        struct pollfd p = {fd, POLLIN | POLLOUT, 0};
        if (poll(&p, 1, 0) != 1 || p.revents != POLLERR) return 18;
        int ep = epoll_create1(0);
        struct epoll_event e = {.events=EPOLLIN | EPOLLOUT, .data=123}, out;
        if (ep < 0 || epoll_ctl(ep, EPOLL_CTL_ADD, fd, &e)) return 19;
        /* An unattached inet placeholder is inert for epoll LT: the old
         * synthetic EPOLLERR was the pre-transport placeholder contract. */
        if (epoll_wait(ep, &out, 1, 0) != 0) return 20;
        if (epoll_wait(ep, &out, 1, 0) != 0) return 21;
        /* TCP ET/oneshot are unsupported and rejected, never silently armed. */
        e.events |= EPOLLET | EPOLLONESHOT;
        if (epoll_ctl(ep, EPOLL_CTL_MOD, fd, &e) != -1 || errno != EINVAL) return 22;
        e.events = EPOLLIN | EPOLLOUT | EPOLLET;
        if (epoll_ctl(ep, EPOLL_CTL_MOD, fd, &e) != -1 || errno != EINVAL) return 23;
        e.events = EPOLLIN | EPOLLOUT | EPOLLONESHOT;
        if (epoll_ctl(ep, EPOLL_CTL_MOD, fd, &e) != -1 || errno != EINVAL) return 39;
        if (epoll_wait(ep, &out, 1, 0) != 0) return 40;
        if (close(ep) || close(fd) || close(fd) != -1 || errno != EBADF) return 24;
    }
    int fds[256], n = 0;
    while (n < 256 && (fds[n] = socket(AF_INET, SOCK_STREAM, 0)) >= 0) ++n;
    if (n != 128 || errno != ENOBUFS) return 25;
    for (int i = 0; i < n; ++i) if (close(fds[i])) return 26;
    inherited = socket(AF_INET, SOCK_STREAM, 0);
    if (inherited < 0) return 27;
    n = 0;
    while (n < 256 && (fds[n] = (int)call(45, inherited, 0, 0)) >= 0) ++n;
    if (n != 252) return 28;
    if (socket(AF_INET, SOCK_STREAM, 0) != -1 || errno != EMFILE) return 29;
    for (int i = 0; i < n; ++i) if (close(fds[i])) return 30;
    pthread_t thread; void *result;
    if (pthread_create(&thread, 0, worker, 0) || pthread_join(thread, &result) || result) return 31;
    struct timeval zero = {0, 0};
    fd_set r, w;
    FD_ZERO(&r); FD_ZERO(&w); FD_SET(inherited, &r); FD_SET(inherited, &w);
    if (select(inherited + 1, &r, &w, 0, &zero) != 2 || !FD_ISSET(inherited, &r) || !FD_ISSET(inherited, &w)) return 35;
    if (send(inherited, 0, 0, 0) != -1 || errno != ENOTCONN) return 36;
    if (recv(inherited, 0, 0, 0) != -1 || errno != ENOTCONN) return 37;
    if (send(inherited, "x", 1, 0x1000) != -1 || errno != EINVAL) return 38;
    char c;
    if (recv(inherited, &c, 1, 0) != -1 || errno != ENOTCONN || close(inherited)) return 32;
    /* Verify both resource exhaustion paths rolled back every allocation. */
    for (int i = 0; i < 128; ++i) if ((fds[i] = socket(AF_INET, SOCK_STREAM, 0)) < 0) return 33;
    for (int i = 0; i < 128; ++i) if (close(fds[i])) return 34;
    return 0;
}
