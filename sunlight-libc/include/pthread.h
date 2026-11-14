#ifndef SUNLIGHT_PTHREAD_H
#define SUNLIGHT_PTHREAD_H
/* Minimal native POSIX threads over the SunlightOS thread/futex ABI.
 *
 * pthread_t is an opaque handle. pthread_create uses the native thread spawn
 * path (one 2 MiB stack and private TLS image per thread). pthread_join waits
 * on a futex word published by the worker, then waits for kernel reaping and
 * releases the thread's stack and TLS mapping. Detached threads are reclaimed
 * lazily by a later pthread_create. Up to 64 live or unreclaimed controls
 * are admitted; exhaustion returns EAGAIN before spawn. Public handle tokens
 * never wrap or reuse; stale join/detach returns EINVAL. Tokens must never be
 * dereferenced. The pointer-sized opaque ABI is unchanged.
 *
 * Not provided: cancellation, scheduling/affinity attributes, robust mutexes, and joining the initial thread. */
#include <bits/time.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct __sunlight_pthread *pthread_t;
/* Native LP64 key: low 7 bits index, remaining bits generation. */
typedef unsigned long pthread_key_t;
/* Also available from limits.h; keep pthread.h freestanding/self-contained. */
#define PTHREAD_KEYS_MAX 128
#define PTHREAD_DESTRUCTOR_ITERATIONS 4
typedef struct { unsigned int __state; } pthread_once_t;
#define PTHREAD_ONCE_INIT { 0 }
int pthread_key_create(pthread_key_t *key, void (*destructor)(void *));
int pthread_key_delete(pthread_key_t key);
void *pthread_getspecific(pthread_key_t key);
int pthread_setspecific(pthread_key_t key, const void *value);
/* Recursive use of the same control cannot complete. Explicit pthread_exit
 * resets active initialization. Cancellation and C++ unwinding are unsupported.
 * Control objects must remain live and aligned while any caller uses them. */
int pthread_once(pthread_once_t *control, void (*routine)(void));

typedef struct {
    int __detachstate;
} pthread_attr_t;

#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_CREATE_DETACHED 1

typedef struct {
    unsigned int __lock;
} pthread_mutex_t;

#define PTHREAD_MUTEX_INITIALIZER { 0 }

/* Process-private only. Storage must remain live until all concurrent callers
 * return; destroy returns EBUSY while waiters (including reacquisition) exist.
 * Cancellation, process sharing and concurrent reinitialization are unsupported. */
typedef struct {
    pthread_mutex_t __gate;
    unsigned int __dead;
    void *__head;
    int __clock;
} pthread_cond_t;
typedef struct { int __clock; } pthread_condattr_t;
#define PTHREAD_COND_INITIALIZER { { 0 }, 0, 0, 0 }
int pthread_cond_init(pthread_cond_t *, const pthread_condattr_t *);
int pthread_cond_destroy(pthread_cond_t *);
int pthread_cond_wait(pthread_cond_t *, pthread_mutex_t *);
int pthread_cond_timedwait(pthread_cond_t *, pthread_mutex_t *, const struct timespec *);
int pthread_cond_signal(pthread_cond_t *);
int pthread_cond_broadcast(pthread_cond_t *);
int pthread_condattr_init(pthread_condattr_t *);
int pthread_condattr_destroy(pthread_condattr_t *);
int pthread_condattr_setclock(pthread_condattr_t *, clockid_t);
int pthread_condattr_getclock(const pthread_condattr_t *, clockid_t *);

int pthread_create(pthread_t *thread, const pthread_attr_t *attr,
                   void *(*start_routine)(void *), void *arg);
int pthread_join(pthread_t thread, void **retval);
void pthread_exit(void *retval) __attribute__((noreturn));
pthread_t pthread_self(void);
int pthread_equal(pthread_t left, pthread_t right);
int pthread_detach(pthread_t thread);

int pthread_attr_init(pthread_attr_t *attr);
int pthread_attr_destroy(pthread_attr_t *attr);
int pthread_attr_setdetachstate(pthread_attr_t *attr, int state);
int pthread_attr_getdetachstate(const pthread_attr_t *attr, int *state);

int pthread_mutex_init(pthread_mutex_t *mutex, const void *attr);
int pthread_mutex_destroy(pthread_mutex_t *mutex);
int pthread_mutex_lock(pthread_mutex_t *mutex);
int pthread_mutex_trylock(pthread_mutex_t *mutex);
int pthread_mutex_unlock(pthread_mutex_t *mutex);

#ifdef __cplusplus
}
#endif
#endif
