#ifndef SUNLIGHT_SIGNAL_H
#define SUNLIGHT_SIGNAL_H
/* Native LP64 synchronous subset: initial-thread SIGUSR1/2 only.
 * No asynchronous sender, process targeting, masks, SA flags or alt stack.
 * Same-signal raise in a handler is deferred/coalesced until handler return.
 * Other-signal nesting and calls from workers fail with ENOSYS. */
#ifdef __cplusplus
extern "C" {
#endif
typedef int sig_atomic_t;
typedef unsigned long sigset_t;
typedef void (*sighandler_t)(int);
#define SIG_DFL ((sighandler_t)0)
#define SIG_IGN ((sighandler_t)1)
#define SIG_ERR ((sighandler_t)-1)
#define SIGUSR1 10
#define SIGUSR2 12
#define SIGKILL 9
#define SIGSTOP 19
struct sigaction {
    sighandler_t sa_handler;
    sigset_t sa_mask;
    unsigned long sa_flags;
};
int sigemptyset(sigset_t *);
int sigfillset(sigset_t *);
int sigaddset(sigset_t *, int);
int sigdelset(sigset_t *, int);
int sigismember(const sigset_t *, int);
int sigaction(int, const struct sigaction *, struct sigaction *);
sighandler_t signal(int, sighandler_t);
int raise(int);
/* Always fail with ENOSYS; these operations are outside this native subset. */
int kill(int, int);
int sigprocmask(int, const sigset_t *, sigset_t *);
#define SIG_BLOCK 0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2
#ifdef __cplusplus
}
#endif
#endif
