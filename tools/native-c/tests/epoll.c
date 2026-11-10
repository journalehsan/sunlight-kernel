#include <sys/epoll.h>
#include <sys/socket.h>
#include <stddef.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>
#include <errno.h>
_Static_assert(sizeof(struct epoll_event) == 16, "size");
_Static_assert(_Alignof(struct epoll_event) == 8, "alignment");
_Static_assert(offsetof(struct epoll_event, data) == 8, "data");
static long call(long n, long a, long b, long c) {
    __asm__ volatile("syscall" : "+a"(n) : "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return n;
}
static long long now(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) return -1;
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}
extern void *sl_std_spawn(void *(*entry)(void *), void *arg, size_t stack);
extern void sl_std_join(void *handle);
static void *producer(void *arg) {
    struct timespec delay = {0, 30000000};
    nanosleep(&delay, 0);
    write(*(int *)arg, "Z", 1);
    return 0;
}
int main(void) {
    int ep = epoll_create1(EPOLL_CLOEXEC), f[2];
    struct epoll_event e = {.events = EPOLLIN, .data = 0x123456789abcdef0ULL}, out[2];
    if (ep < 0 || call(47, (long)f, 0, 0)) return 1;
    if (epoll_ctl(ep, EPOLL_CTL_ADD, f[0], &e)) return 2;
    if (epoll_ctl(ep, EPOLL_CTL_ADD, f[0], &e) != -1) return 3;
    long long start = now();
    if (epoll_wait(ep, out, 2, 35) || now() - start < 30 || now() - start > 2000) return 4;
    if (write(f[1], "AB", 2) != 2) return 5;
    if (epoll_wait(ep, out, 2, 0) != 1 || out[0].events != EPOLLIN || out[0].data != e.data) return 6;
    if (epoll_wait(ep, out, 2, 0) != 1) return 7;
    e.events = EPOLLIN | EPOLLET;
    if (epoll_ctl(ep, EPOLL_CTL_MOD, f[0], &e) || epoll_wait(ep, out, 2, 0) != 1 || epoll_wait(ep, out, 2, 0)) return 8;
    char bytes[2];
    if (call(42, f[0], (long)bytes, 2) != 2) return 9;
    void *thread = sl_std_spawn(producer, &f[1], 65536);
    if (!thread || epoll_wait(ep, out, 2, -1) != 1) return 10;
    sl_std_join(thread);
    if (call(42, f[0], (long)bytes, 1) != 1 || bytes[0] != 'Z') return 11;
    e.events = EPOLLIN | EPOLLONESHOT;
    if (epoll_ctl(ep, EPOLL_CTL_MOD, f[0], &e) || write(f[1], "C", 1) != 1) return 12;
    if (epoll_wait(ep, out, 2, 0) != 1 || epoll_wait(ep, out, 2, 0)) return 13;
    if (epoll_ctl(ep, EPOLL_CTL_MOD, f[0], &e) || epoll_wait(ep, out, 2, 0) != 1) return 14;
    if (epoll_ctl(ep, EPOLL_CTL_DEL, f[0], 0) || epoll_wait(ep, out, 2, 0)) return 15;
    e.events = EPOLLOUT; e.data = 99;
    if (epoll_ctl(ep, EPOLL_CTL_ADD, f[1], &e) || epoll_wait(ep, out, 2, 0) != 1 || out[0].events != EPOLLOUT || out[0].data != 99) return 16;
    if (call(41, f[0], 0, 0) || call(41, f[1], 0, 0) || call(41, ep, 0, 0)) return 17;
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv)) return 18;
    ep = epoll_create1(0); e.events = EPOLLIN | EPOLLET;
    if (ep < 0 || epoll_ctl(ep, EPOLL_CTL_ADD, sv[1], &e)) return 19;
    thread = sl_std_spawn(producer, &sv[0], 65536);
    if (!thread || epoll_wait(ep, out, 2, 500) != 1 || out[0].events != EPOLLIN) return 20;
    sl_std_join(thread);
    if (recv(sv[1], bytes, 1, 0) != 1 || epoll_wait(ep, out, 2, 0)) return 21;
    if (call(41, sv[0], 0, 0) || call(41, sv[1], 0, 0) || call(41, ep, 0, 0)) return 22;
    return 0;
}
