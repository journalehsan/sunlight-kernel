#include <errno.h>
#include <pthread.h>
#include <string.h>

#define THREADS 8
#define ITERATIONS 500

static _Thread_local int tls_slot = -1;
static pthread_mutex_t counter_lock = PTHREAD_MUTEX_INITIALIZER;
static int counter;
static volatile int detached_done;

struct job {
    int index;
    pthread_t self;
};

static void *worker(void *raw) {
    struct job *job = raw;
    tls_slot = job->index;
    job->self = pthread_self();
    for (int i = 0; i < ITERATIONS; i++) {
        if (pthread_mutex_lock(&counter_lock))
            return (void *)1;
        counter++;
        if (pthread_mutex_unlock(&counter_lock))
            return (void *)1;
    }
    if (tls_slot != job->index)
        return (void *)2;
    return (void *)(long)(job->index + 1);
}

static int basic(void) {
    struct job jobs[THREADS];
    pthread_t threads[THREADS];
    memset(jobs, 0, sizeof(jobs));
    for (int i = 0; i < THREADS; i++) {
        jobs[i].index = i;
        if (pthread_create(&threads[i], 0, worker, &jobs[i]))
            return 1;
    }
    for (int i = 0; i < THREADS; i++) {
        void *retval = 0;
        if (pthread_join(threads[i], &retval))
            return 2;
        if (retval != (void *)(long)(i + 1))
            return 3;
        if (!pthread_equal(threads[i], jobs[i].self))
            return 4;
    }
    if (counter != THREADS * ITERATIONS)
        return 5;
    if (tls_slot != -1)
        return 6;
    return 0;
}

static void *exiting_worker(void *raw) {
    (void)raw;
    pthread_exit((void *)0x1234);
}

static int exiting(void) {
    pthread_t thread;
    void *retval = 0;
    if (pthread_create(&thread, 0, exiting_worker, 0))
        return 1;
    if (pthread_join(thread, &retval))
        return 2;
    return retval == (void *)0x1234 ? 0 : 3;
}

static void *detached_worker(void *raw) {
    (void)raw;
    detached_done = 1;
    return (void *)7;
}

static void *sweep_worker(void *raw) {
    (void)raw;
    return 0;
}

static int detached(void) {
    pthread_attr_t attr;
    pthread_t thread;
    if (pthread_attr_init(&attr))
        return 1;
    if (pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED))
        return 2;
    if (pthread_create(&thread, &attr, detached_worker, 0))
        return 3;
    if (pthread_join(thread, 0) != EINVAL)
        return 4;
    while (!detached_done) {
    }
    /* A later create/join sweeps the detached control block. */
    pthread_t sweep;
    if (pthread_create(&sweep, 0, sweep_worker, 0))
        return 5;
    if (pthread_join(sweep, 0))
        return 6;
    return pthread_attr_destroy(&attr);
}

static int attributes(void) {
    pthread_attr_t attr;
    int state = -1;
    if (pthread_attr_init(&attr))
        return 1;
    if (pthread_attr_getdetachstate(&attr, &state))
        return 2;
    if (state != PTHREAD_CREATE_JOINABLE)
        return 3;
    if (pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED))
        return 4;
    if (pthread_attr_getdetachstate(&attr, &state))
        return 5;
    if (state != PTHREAD_CREATE_DETACHED)
        return 6;
    if (pthread_attr_setdetachstate(&attr, 99) != EINVAL)
        return 7;
    return pthread_attr_destroy(&attr);
}

int main(int argc, char **argv) {
    const char *mode = argc > 1 ? argv[1] : "basic";
    if (strcmp(mode, "basic") == 0)
        return basic();
    if (strcmp(mode, "exit") == 0)
        return exiting();
    if (strcmp(mode, "detached") == 0)
        return detached();
    if (strcmp(mode, "attr") == 0)
        return attributes();
    return 100;
}
