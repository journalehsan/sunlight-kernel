#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <signal.h>
#include <stdio.h>
#include <sys/select.h>
#include <sys/syscall.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

static volatile sig_atomic_t caught;
static void handler(int sig) { caught = sig; }

static int64_t now_ms(void)
{
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0)
        return -1;
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

int main(void)
{
    struct termios orig, raw;
    if (!isatty(0) || tcgetattr(0, &orig) != 0)
        return 10;
    raw = orig;
    cfmakeraw(&raw);
    if (tcsetattr(0, TCSANOW, &raw) != 0)
        return 11;
    setvbuf(stdout, NULL, _IONBF, 0);

    fd_set rd, wr, ex;
    FD_ZERO(&rd);
    FD_ZERO(&ex);
    FD_SET(0, &rd);
    FD_SET(0, &ex);
    struct timespec ts = {0, 200000000};
    int64_t start = now_ms();
    int rc = pselect(1, &rd, NULL, &ex, &ts, NULL);
    int64_t elapsed = now_ms() - start;
    printf("select idle rc=%d read=%d except=%d elapsed_ms=%" PRId64 "\n",
           rc, FD_ISSET(0, &rd), FD_ISSET(0, &ex), elapsed);
    int ok = rc == 0 && !FD_ISSET(0, &rd) && !FD_ISSET(0, &ex) && elapsed >= 180;

    FD_ZERO(&wr);
    FD_SET(1, &wr);
    ts = (struct timespec){0, 0};
    rc = pselect(2, NULL, &wr, NULL, &ts, NULL);
    printf("select writable rc=%d stdout=%d\n", rc, FD_ISSET(1, &wr));
    ok &= rc == 1 && FD_ISSET(1, &wr);

    int bad = dup(0);
    if (bad < 0 || bad >= FD_SETSIZE || close(bad) != 0) {
        tcsetattr(0, TCSANOW, &orig);
        return 12;
    }
    FD_ZERO(&rd);
    FD_SET(bad, &rd);
    errno = 0;
    rc = pselect(bad + 1, &rd, NULL, NULL, &ts, NULL);
    printf("select invalid rc=%d errno=%d unchanged=%d\n", rc, errno, FD_ISSET(bad, &rd));
    ok &= rc == -1 && errno == EBADF && FD_ISSET(bad, &rd);

    puts("SELECT-INJECT");
    FD_ZERO(&rd);
    FD_SET(0, &rd);
    FD_SET(0, &ex);
    ts = (struct timespec){8, 0};
    start = now_ms();
    rc = pselect(1, &rd, NULL, &ex, &ts, NULL);
    elapsed = now_ms() - start;
    char key = 0;
    ssize_t n = rc == 1 ? read(0, &key, 1) : 0;
    printf("select key rc=%d read=%d except=%d elapsed_ms=%" PRId64 " n=%zd char=%d\n",
           rc, FD_ISSET(0, &rd), FD_ISSET(0, &ex), elapsed, n, key);
    ok &= rc == 1 && FD_ISSET(0, &rd) && !FD_ISSET(0, &ex) && n == 1 && key == 'x'
          && elapsed < 7000;
    tcsetattr(0, TCSANOW, &orig);

    const unsigned long arg[2] = {0, 0};
    ts = (struct timespec){0, 0};
    long null_mask = syscall(SYS_pselect6, 0, NULL, NULL, NULL, &ts, arg);
    errno = 0;
    long invalid_arg = syscall(SYS_pselect6, 0, NULL, NULL, NULL, &ts, 1UL);
    int arg_error = errno;
    printf("select r9 null_mask=%ld invalid_arg=%ld errno=%d\n", null_mask, invalid_arg, arg_error);
    ok &= null_mask == 0 && invalid_arg == -1 && arg_error == EFAULT;

    struct sigaction sa = {.sa_handler = handler};
    sigset_t blocked, empty, actual;
    sigemptyset(&sa.sa_mask);
    sigemptyset(&blocked);
    sigaddset(&blocked, SIGINT);
    sigemptyset(&empty);
    if (sigaction(SIGINT, &sa, NULL) != 0 ||
        sigprocmask(SIG_BLOCK, &blocked, NULL) != 0 || raise(SIGINT) != 0)
        return 13;
    ts = (struct timespec){0, 200000000};
    errno = 0;
    rc = pselect(0, NULL, NULL, NULL, &ts, &empty);
    int err = errno;
    if (sigprocmask(SIG_SETMASK, NULL, &actual) != 0)
        return 14;
    printf("select mask rc=%d errno=%d caught=%d restored=%d\n",
           rc, err, caught, sigismember(&actual, SIGINT));
    ok &= rc == -1 && sigismember(&actual, SIGINT) == 1 &&
          ((err == EINTR && caught == SIGINT) || (err == EOPNOTSUPP && !caught));
    return ok ? 0 : 1;
}
