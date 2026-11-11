#ifdef REVERSE_INCLUDES
#include <stddef.h>
#include <pthread.h>
#include <signal.h>
#else
#include <signal.h>
#include <pthread.h>
#include <stddef.h>
#endif
#ifdef __cplusplus
#define CHECK static_assert
#else
#define CHECK _Static_assert
#endif
CHECK(sizeof(sig_atomic_t) == 4, "atomic int");
CHECK(sizeof(sigset_t) == 8, "native mask");
CHECK(sizeof(struct sigaction) == 24, "native action");
CHECK(offsetof(struct sigaction, sa_handler) == 0, "handler");
CHECK(offsetof(struct sigaction, sa_mask) == 8, "mask");
CHECK(offsetof(struct sigaction, sa_flags) == 16, "flags");
static int (*const action_fn)(int, const struct sigaction *, struct sigaction *) __attribute__((unused)) = sigaction;
static sighandler_t (*const signal_fn)(int, sighandler_t) __attribute__((unused)) = signal;
int main(void) { return 0; }
