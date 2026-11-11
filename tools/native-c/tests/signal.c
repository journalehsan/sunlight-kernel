#include <signal.h>
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <string.h>
static volatile sig_atomic_t seen;
static volatile sig_atomic_t error;
static _Thread_local int tls = 71;
static void h(int s) {
    int saved = errno;
    if (s != SIGUSR1 || tls != 71) error = 1;
    ++seen;
    if (seen == 1) {
        if (raise(s) || raise(s) || seen != 1) error = 2;
        if (raise(SIGUSR2) != -1 || errno != ENOSYS) error = 3;
    }
    unsigned int mxcsr = 0x3f80;
    __asm__ volatile("ldmxcsr %0" : : "m"(mxcsr));
    __asm__ volatile("pxor %%xmm15, %%xmm15; fninit" : : : "xmm15");
    errno = saved;
}
static void exiting_handler(int s) {
    (void)s;
    __asm__ volatile("syscall" : : "a"(157L), "D"(31L) : "rcx", "r11", "memory");
}
static void *worker(void *unused) {
    (void)unused;
    errno = 0;
    return (void *)(uintptr_t)(raise(SIGUSR1) == -1 && errno == ENOSYS);
}
int main(int argc, char **argv) {
    if (argc > 1 && !strcmp(argv[1], "handler-exit")) {
        signal(SIGUSR1, exiting_handler); raise(SIGUSR1); return 92;
    }
    if (argc > 1 && !strcmp(argv[1], "bad-stack")) {
        signal(SIGUSR1, h);
        __asm__ volatile("mov $64, %%rsp; syscall" : : "a"(174L), "D"(1L), "S"((long)SIGUSR1) : "rcx", "r11", "memory");
        return 93;
    }
    if (argc > 1 && !strcmp(argv[1], "fatal")) { raise(SIGUSR1); return 90; }
    if (argc > 1 && !strcmp(argv[1], "forged")) {
        __asm__ volatile("syscall" : : "a"(174L), "D"(2L) : "rcx", "r11", "memory");
        return 91;
    }
    sigset_t set;
    if (sigemptyset(&set) || set || sigaddset(&set, SIGUSR1) ||
        sigismember(&set, SIGUSR1) != 1 || sigdelset(&set, SIGUSR1) || set) return 1;
    struct sigaction a = {h, 0, 0}, old;
    if (sigaction(SIGUSR1, &a, &old) || old.sa_handler != SIG_DFL) return 2;
    struct sigaction bad = {(sighandler_t)&set, 0, 0};
    if (sigaction(SIGUSR2, &bad, 0) != -1 || errno != EFAULT) return 17;
    a.sa_mask = 1;
    if (sigaction(SIGUSR1, &a, 0) != -1 || errno != EINVAL) return 15;
    a.sa_mask = 0;
    a.sa_flags = 1;
    errno = 0;
    if (sigaction(SIGUSR1, &a, 0) != -1 || errno != EINVAL) return 3;
    errno = 0;
    if (signal(SIGKILL, SIG_IGN) != SIG_ERR || errno != EINVAL) return 4;
    if (raise(0) != -1 || errno != EINVAL) return 5;
    errno = 49;
    unsigned int before, after;
    __asm__ volatile("stmxcsr %0" : "=m"(before));
    if (raise(SIGUSR1) || seen != 2 || error || errno != 49 || tls != 71) return 6;
    __asm__ volatile("stmxcsr %0" : "=m"(after));
    if (before != after) return 7;
    /* Test the syscall boundary directly: XMM registers are caller-saved in C. */
    uint64_t pattern = 0x3141592653589793ULL, observed;
    unsigned short control = 0x077f, restored;
    long result_code;
    __asm__ volatile("movq %[pattern], %%xmm15; fldcw %[control]; syscall; movq %%xmm15, %[observed]; fnstcw %[restored]"
        : "=a"(result_code), [observed] "=m"(observed), [restored] "=m"(restored)
        : "a"(174L), "D"(1L), "S"((long)SIGUSR1), [pattern] "m"(pattern), [control] "m"(control)
        : "rcx", "r11", "xmm15", "memory");
    if (result_code || observed != pattern || restored != control) return 14;
    for (int i = 0; i < 200; ++i) if (raise(SIGUSR1)) return 8;
    if (seen != 203) return 9;
    if (signal(SIGUSR1, SIG_IGN) != h || raise(SIGUSR1) || seen != 203) return 10;
    if (signal(SIGUSR2, SIG_IGN) != SIG_DFL || raise(SIGUSR2)) return 16;
    if (kill(1, SIGUSR1) != -1 || errno != ENOSYS) return 11;
    if (sigprocmask(SIG_BLOCK, &set, 0) != -1 || errno != ENOSYS) return 12;
    pthread_t thread;
    void *result = 0;
    if (pthread_create(&thread, 0, worker, 0) || pthread_join(thread, &result) || result != (void *)1) return 13;
    return 0;
}
