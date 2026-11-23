#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdint.h>
#include <sys/select.h>
#include <sys/syscall.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

static volatile sig_atomic_t caught, handler_mask, count, handler_tid;
static long target_tid;
static sigset_t original, temporary;
static atomic_int armed, done;
static pthread_t waiter;
static char syscall_path[96];

static int mask_is(int intr, int usr1, int usr2, int term)
{
    sigset_t mask;
    return sigprocmask(SIG_SETMASK, NULL, &mask) == 0 &&
        sigismember(&mask, SIGINT) == intr &&
        sigismember(&mask, SIGUSR1) == usr1 &&
        sigismember(&mask, SIGUSR2) == usr2 &&
        sigismember(&mask, SIGTERM) == term;
}

static void handler(int sig)
{
    int saved = errno;
    handler_mask = mask_is(1, 0, 1, 1);
    handler_tid = syscall(SYS_gettid);
    count++;
    caught = sig;
    errno = saved;
}

/* Observe the target's actual Linux syscall, not merely its intent to wait.
 * ENOENT on a guest without this procfs entry is unobserved, not a pass. */
static void *sender(void *unused)
{
    (void)unused;
    int fd = open(syscall_path, O_RDONLY);
    if (fd < 0) return NULL;
    while (!atomic_load(&armed) && !atomic_load(&done))
        sched_yield();
    for (int i = 0; i < 10000 && !atomic_load(&done); i++) {
        char buf[256];
        long nr;
        if (lseek(fd, 0, SEEK_SET) < 0) break;
        ssize_t n = read(fd, buf, sizeof(buf) - 1);
        if (n <= 0) break;
        buf[n] = 0;
        if (sscanf(buf, "%ld", &nr) == 1 && nr == SYS_pselect6) {
            int isolated = mask_is(1, 1, 0, 0);
            int sent = pthread_kill(waiter, SIGINT);
            printf("sender observed=%ld tid=%ld isolated=%d send=%d\n",
                   nr, syscall(SYS_gettid), isolated, sent);
            close(fd);
            return isolated && sent == 0 ? &armed : NULL;
        }
        sched_yield();
    }
    close(fd);
    return NULL;
}

static int check(const char *label, int rc, int err, int interrupted)
{
    // Snapshot handler state before issuing a mask query or printing.
    int c = count, t = handler_tid, m = handler_mask, sig = caught;
    int restored = mask_is(1, 1, 0, 0);
    printf("%s rc=%d errno=%d caught=%d handler_mask=%d restored=%d count=%d target=%ld handler=%d\n",
           label, rc, err, sig, m, restored, c, target_tid, t);
    return restored && (interrupted
        ? rc == -1 && err == EINTR && sig == SIGINT && m && c == 1 && t == target_tid
        : rc == 0 && !sig && !c);
}

static void *before_wait(void *unused)
{
    (void)unused;
    int isolated = mask_is(1, 1, 0, 0);
    int sent = pthread_kill(waiter, SIGINT);
    printf("sender before registration tid=%ld isolated=%d send=%d\n",
           syscall(SYS_gettid), isolated, sent);
    return isolated && sent == 0 ? &armed : NULL;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    target_tid = syscall(SYS_gettid);
    struct sigaction action = {.sa_handler = handler, .sa_flags = SA_RESTART};
    sigemptyset(&action.sa_mask);
    sigaddset(&action.sa_mask, SIGUSR2);
    sigemptyset(&original);
    sigaddset(&original, SIGINT);
    sigaddset(&original, SIGUSR1);
    sigemptyset(&temporary);
    sigaddset(&temporary, SIGTERM);
    if (sigaction(SIGINT, &action, NULL) ||
        pthread_sigmask(SIG_SETMASK, &original, NULL) || raise(SIGINT) || caught)
        return 2;

    const struct timespec short_wait = {0, 200000000}, long_wait = {4, 0};
    const struct timespec zero = {0, 0};
    errno = 0;
    int rc = pselect(0, NULL, NULL, NULL, &short_wait, &temporary);
    int ok = check("pending", rc, errno, 1);
    if (!ok) return 1;

    waiter = pthread_self();
    pthread_t worker;
    void *observed;
    caught = handler_mask = count = handler_tid = 0;
    if (pthread_create(&worker, NULL, before_wait, NULL) ||
        pthread_join(worker, &observed) || !observed || count)
        return 9;
    errno = 0;
    rc = pselect(0, NULL, NULL, NULL, &short_wait, &temporary);
    ok &= check("before-registration", rc, errno, 1);

    caught = handler_mask = count = handler_tid = 0;
    errno = 0;
    rc = pselect(0, NULL, NULL, NULL, &short_wait, &temporary);
    ok &= check("timeout", rc, errno, 0);

    struct termios saved, raw;
    if (tcgetattr(0, &saved)) return 5;
    raw = saved;
    cfmakeraw(&raw);
    raw.c_cc[VMIN] = raw.c_cc[VTIME] = 0;
    if (tcsetattr(0, TCSANOW, &raw)) return 6;
    snprintf(syscall_path, sizeof(syscall_path), "/proc/self/task/%ld/syscall",
             syscall(SYS_gettid));
    if (pthread_create(&worker, NULL, sender, NULL)) return 3;
    puts("MASKED-SELECT-WAITING");
    atomic_store(&armed, 1);
    errno = 0;
    rc = pselect(0, NULL, NULL, NULL, &long_wait, &temporary);
    int err = errno;
    ok &= check("waiting", rc, err, 1); // before join or any retry
    atomic_store(&done, 1);
    if (pthread_join(worker, &observed)) return 4;
    ok &= observed != NULL && count == 1;

    puts("MASKED-SELECT-INJECT");
    fd_set rd;
    FD_ZERO(&rd);
    FD_SET(0, &rd);
    rc = pselect(1, &rd, NULL, NULL, &long_wait, &temporary);
    char key = 0;
    ssize_t n = rc == 1 ? read(0, &key, 1) : 0;
    int restored = mask_is(1, 1, 0, 0);
    printf("tty rc=%d ready=%d n=%zd key=%d restored=%d\n",
           rc, FD_ISSET(0, &rd), n, key, restored);
    ok &= rc == 1 && FD_ISSET(0, &rd) && n == 1 && key == 'x' && restored;
    n = read(0, &key, 1);
    printf("tty empty read=%zd\n", n);
    ok &= n == 0;
    if (tcsetattr(0, TCSANOW, &saved)) return 7;

    int bad = dup(0);
    if (bad < 0 || bad >= FD_SETSIZE || close(bad)) return 8;
    FD_ZERO(&rd);
    FD_SET(bad, &rd);
    errno = 0;
    rc = pselect(bad + 1, &rd, NULL, NULL, &zero, &temporary);
    err = errno;
    restored = mask_is(1, 1, 0, 0);
    printf("badfd rc=%d errno=%d unchanged=%d restored=%d\n",
           rc, err, FD_ISSET(bad, &rd), restored);
    ok &= rc == -1 && err == EBADF && FD_ISSET(bad, &rd) && restored;
    const struct { uintptr_t ptr; size_t size; } wrong_size = {(uintptr_t)&temporary, 7},
        wrong_mask = {1, 8}, null_mask = {0, 7};
    const void *args[] = {(void *)1, &wrong_size, &wrong_mask, &null_mask};
    const int errors[] = {EFAULT, EINVAL, EFAULT, 0};
    for (unsigned i = 0; i < sizeof(args) / sizeof(args[0]); i++) {
        struct timespec ts = zero;
        errno = 0;
        rc = syscall(SYS_pselect6, 0, 0, 0, 0, &ts, args[i]);
        err = errno;
        restored = mask_is(1, 1, 0, 0);
        printf("descriptor case=%u rc=%d errno=%d restored=%d\n", i, rc, err, restored);
        ok &= rc == (errors[i] ? -1 : 0) && err == errors[i] && restored;
    }
    return ok ? 0 : 1;
}
