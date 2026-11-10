#include <sys/epoll.h>
#include <unistd.h>
static long call(long n, long a, long b, long c) {
    __asm__ volatile("syscall" : "+a"(n) : "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return n;
}
int main(void) {
    for (int i = 0; i < 80; ++i) {
        int ep = epoll_create1(0), f[2];
        struct epoll_event r = {.events = EPOLLIN, .data = 0x1122334455667788ULL};
        struct epoll_event w = {.events = EPOLLOUT, .data = 0x8877665544332211ULL}, out[2];
        if (ep < 0 || call(47, (long)f, 0, 0)) return 1;
        if (epoll_ctl(ep, EPOLL_CTL_ADD, f[0], &r) || epoll_ctl(ep, EPOLL_CTL_ADD, f[1], &w)) return 2;
        if (write(f[1], "X", 1) != 1 || epoll_wait(ep, out, 2, 0) != 2) return 3;
        if (out[0].events != EPOLLIN || out[0].data != r.data || out[1].events != EPOLLOUT || out[1].data != w.data) return 4;
        if (call(41, f[0], 0, 0) || call(41, f[1], 0, 0)) return 5;
        if (epoll_wait(ep, out, 2, 0)) return 6;
        if (call(47, (long)f, 0, 0) || epoll_wait(ep, out, 2, 0)) return 7;
        if (call(41, f[0], 0, 0) || call(41, f[1], 0, 0) || call(41, ep, 0, 0)) return 8;
    }
    return 0;
}
