#include <poll.h>
#include <sys/select.h>
#include <time.h>
#include <errno.h>
#include <stdio.h>
#include <unistd.h>
#include <stdint.h>
/* Pipe creation/closure use the existing native ABI; they are not new C APIs. */
static long call(long n, long a, long b, long c) {
    __asm__ volatile("syscall" : "+a"(n) : "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory");
    return n;
}
static long long now(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) return -1;
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}
extern void *sl_std_spawn(void *(*entry)(void *), void *arg, size_t stack);
extern void sl_std_join(void *handle);
static void *delayed_write(void *arg) {
    struct timespec delay = {0, 30000000};
    if (nanosleep(&delay, 0) || write(*(int *)arg, "Y", 1) != 1) return (void *)1;
    return 0;
}
int main(void) {
    errno = 77;
    if (poll(0,0,0) || errno != 77) return 1;
    if (poll(0,257,0) != -1 || errno != EINVAL) return 2;
    if (poll(0,1,0) != -1 || errno != EFAULT) return 3;
    struct pollfd p[5] = {{-1, -1, -1}, {-999, POLLIN, -1}, {4095,0,-1}, {1,POLLOUT,-1}, {2,POLLOUT,-1}};
    if (poll(p,5,0) != 3 || p[0].revents || p[1].revents || p[2].revents != POLLNVAL || p[3].revents != POLLOUT || p[4].revents != POLLOUT) return 4;
    long long start = now();
    if (poll(0,0,35) || now()-start < 30 || now()-start > 2000) return 5;
    p[0] = (struct pollfd){-8, POLLIN, -1};
    start = now();
    if (poll(p,1,25) || p[0].revents || now()-start < 20) return 6;
    int fds[2];
    if (call(47,(long)fds,0,0)) return 7;
    p[0] = (struct pollfd){fds[0],POLLIN,-1};
    p[1] = (struct pollfd){fds[1],POLLOUT,-1};
    if (poll(p,2,0) != 1 || p[0].revents || p[1].revents != POLLOUT) return 8;
    start = now();
    if (poll(p,1,25) || p[0].revents || now()-start < 20) return 9;
    if (write(fds[1],"X",1) != 1) return 10;
    if (poll(p,2,-123) != 2 || p[0].revents != POLLIN) return 11;
    call(41,fds[1],0,0);
    if (poll(p,2,0) != 2 || p[0].revents != (POLLIN|POLLHUP) || p[1].revents != POLLNVAL) return 12;
    char c;
    if (call(42,fds[0],(long)&c,1) != 1 || c != 'X') return 13;
    if (poll(p,1,0) != 1 || p[0].revents != (POLLIN|POLLHUP)) return 14;
    p[0].events = 0;
    if (poll(p,1,0) != 1 || p[0].revents != POLLHUP) return 15;
    call(41,fds[0],0,0);
    if (call(47,(long)fds,0,0)) return 16;
    call(41,fds[0],0,0);
    p[0] = (struct pollfd){fds[1],0,-1};
    if (poll(p,1,0) != 1 || p[0].revents != POLLERR) return 17;
    call(41,fds[1],0,0);
    FILE *f = tmpfile();
    if (!f) return 18;
    p[0] = (struct pollfd){fileno(f),POLLIN|POLLOUT|POLLPRI,-1};
    if (poll(p,1,0) != 1 || p[0].revents != (POLLIN|POLLOUT)) return 19;
    fclose(f);
    fd_set r,w,e;
    FD_ZERO(&r); FD_ZERO(&w); FD_ZERO(&e);
    FD_SET(1,&w); FD_SET(2,&w); FD_CLR(2,&w);
    if (!FD_ISSET(1,&w) || FD_ISSET(2,&w)) return 20;
    struct timeval tv = {0,0};
    if (select(3,&r,&w,&e,&tv) != 1 || !FD_ISSET(1,&w)) return 21;
    FD_SET(255,&r);
    if (select(256,&r,0,0,&tv) != -1 || errno != EBADF) return 22;
    tv.tv_usec = 1000000;
    if (select(0,0,0,0,&tv) != -1 || errno != EINVAL) return 23;
    tv = (struct timeval){0,25001}; start = now();
    if (select(0,0,0,0,&tv) || now()-start < 20) return 24;
    if (call(47,(long)fds,0,0)) return 25;
    p[0] = (struct pollfd){fds[0],POLLIN,-1};
    void *thread = sl_std_spawn(delayed_write, &fds[1], 65536);
    if (!thread) return 26;
    start = now();
    if (poll(p,1,-7) != 1 || p[0].revents != POLLIN || now()-start < 20) return 27;
    sl_std_join(thread);
    if (call(42,fds[0],(long)&c,1) != 1 || c != 'Y') return 28;
    thread = sl_std_spawn(delayed_write, &fds[1], 65536);
    if (!thread) return 29;
    if (poll(p,1,500) != 1 || p[0].revents != POLLIN) return 30;
    sl_std_join(thread);
    if (call(42,fds[0],(long)&c,1) != 1 || c != 'Y') return 31;
    /* A notification with no requested event must not reset the deadline. */
    p[0].events = POLLPRI;
    thread = sl_std_spawn(delayed_write, &fds[1], 65536);
    if (!thread) return 32;
    start = now();
    if (poll(p,1,100) || p[0].revents || now()-start < 90 || now()-start > 1000) return 33;
    sl_std_join(thread);
    call(41,fds[0],0,0); call(41,fds[1],0,0);
    if (call(47,(long)fds,0,0)) return 34;
    call(41,fds[1],0,0);
    FD_ZERO(&e); FD_SET(fds[0], &e);
    tv = (struct timeval){0,35000}; start = now();
    if (select(fds[0]+1,0,0,&e,&tv) || FD_ISSET(fds[0],&e) || now()-start < 30) return 35;
    call(41,fds[0],0,0);
    return 0;
}
