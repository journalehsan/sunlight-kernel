#ifndef SUNLIGHT_PTHREAD_H
#define SUNLIGHT_PTHREAD_H
/* Minimal native POSIX threads over the SunlightOS thread/futex ABI.
 *
 * pthread_t is an opaque handle. pthread_create uses the native thread spawn
 * path (one 2 MiB stack and private TLS image per thread). pthread_join waits
 * on a futex word published by the worker, then waits for kernel reaping and
 * releases the thread's stack and TLS mapping. Detached threads are reclaimed
 * lazily by a later pthread_create or pthread_join.
 *
 * Not provided: cancellation, scheduling/affinity attributes, condition
 * variables, robust mutexes, and joining the initial thread. */
#ifdef __cplusplus
extern "C" {
#endif

typedef struct __sunlight_pthread *pthread_t;

typedef struct {
    int __detachstate;
} pthread_attr_t;

#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_CREATE_DETACHED 1

typedef struct {
    unsigned int __lock;
} pthread_mutex_t;

#define PTHREAD_MUTEX_INITIALIZER { 0 }

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
