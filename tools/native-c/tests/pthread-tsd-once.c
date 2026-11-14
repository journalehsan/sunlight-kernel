#include <pthread.h>
#include <errno.h>
#include <string.h>
#include <stdio.h>
#include <stdint.h>
#include <time.h>
#define CHECK(x) do { if (!(x)) { printf("FAIL line=%d\n", __LINE__); return (void *)(uintptr_t)__LINE__; } } while (0)
static pthread_key_t key, deleted, repeat_key, reentrant;
static unsigned destructed, repeats, detached_done, entered, release_worker;
static unsigned racing, race_stop;
static unsigned once_count, once_entered, once_release, ready, started;
static int published;
static pthread_once_t once = PTHREAD_ONCE_INIT, abandon = PTHREAD_ONCE_INIT;
static pthread_once_t recursive = PTHREAD_ONCE_INIT, nested = PTHREAD_ONCE_INIT;
static void usleep(unsigned microseconds) { struct timespec ts = {0, (long)microseconds * 1000}; nanosleep(&ts, 0); }
static void wait_for(unsigned *word, unsigned target) { while (__atomic_load_n(word, __ATOMIC_ACQUIRE) < target) usleep(1000); }
static void destructor(void *p) {
    if (pthread_getspecific(key) != 0 || p == 0) __builtin_trap();
    __atomic_add_fetch(&destructed, 1, __ATOMIC_RELEASE);
}
static void repeat(void *p) {
    __atomic_add_fetch(&repeats, 1, __ATOMIC_RELEASE);
    if (pthread_setspecific(repeat_key, p)) __builtin_trap();
}
static void reenter(void *p) {
    pthread_key_t temporary;
    if (pthread_key_create(&temporary, 0) || pthread_setspecific(temporary, p) ||
        pthread_getspecific(temporary) != p || pthread_key_delete(temporary)) __builtin_trap();
    if (pthread_key_delete(reentrant)) __builtin_trap();
}
static void detached_destructor(void *p) { (void)p; __atomic_store_n(&detached_done, 1, __ATOMIC_RELEASE); }
static void *isolated(void *p) {
    CHECK(pthread_getspecific(key) == 0);
    CHECK(pthread_setspecific(key, p) == 0);
    CHECK(pthread_getspecific(key) == p);
    CHECK(pthread_setspecific(key, 0) == 0);
    CHECK(pthread_getspecific(key) == 0);
    CHECK(pthread_setspecific(key, p) == 0);
    return 0;
}
static void *explicit_exit(void *p) { pthread_setspecific(key, p); pthread_exit(p); }
static void *repeating(void *p) { pthread_setspecific(repeat_key, p); pthread_setspecific(reentrant, p); return 0; }
static void *delete_race(void *p) {
    CHECK(pthread_setspecific(deleted, p) == 0);
    __atomic_store_n(&entered, 1, __ATOMIC_RELEASE);
    wait_for(&release_worker, 1);
    CHECK(pthread_getspecific(deleted) == 0);
    CHECK(pthread_setspecific(deleted, p) == EINVAL);
    return 0;
}
static void *concurrent_delete(void *p) {
    CHECK(pthread_setspecific(deleted,p) == 0);
    __atomic_store_n(&racing,1,__ATOMIC_RELEASE);
    while (!__atomic_load_n(&race_stop,__ATOMIC_ACQUIRE)) {
        void *v = pthread_getspecific(deleted);
        CHECK(v == 0 || v == p);
        int rc = pthread_setspecific(deleted,p);
        CHECK(rc == 0 || rc == EINVAL);
    }
    CHECK(pthread_getspecific(deleted) == 0);
    return 0;
}
static void init(void) {
    __atomic_add_fetch(&once_count, 1, __ATOMIC_RELAXED);
    __atomic_store_n(&once_entered, 1, __ATOMIC_RELEASE);
    wait_for(&once_release, 1);
    published = 0x12345678;
}
static void *once_worker(void *p) {
    (void)p;
    __atomic_add_fetch(&started, 1, __ATOMIC_RELEASE);
    CHECK(pthread_once(&once, init) == 0);
    CHECK(published == 0x12345678);
    for (int i=0; i<10000; ++i) CHECK(pthread_once(&once, init) == 0);
    __atomic_add_fetch(&ready, 1, __ATOMIC_RELEASE);
    return 0;
}
static void abandoned_init(void) { __atomic_store_n(&entered, 1, __ATOMIC_RELEASE); wait_for(&release_worker, 1); pthread_exit((void *)77); }
static void recovered_init(void) { published = 42; }
static void *abandon_worker(void *p) { (void)p; pthread_once(&abandon, abandoned_init); return (void *)1; }
static void *recover_worker(void *p) { (void)p; __atomic_store_n(&started, 1, __ATOMIC_RELEASE); CHECK(pthread_once(&abandon, recovered_init) == 0); CHECK(published == 42); return 0; }
static void nested_init(void) { }
static void recurse(void) { if (pthread_once(&nested, nested_init)) __builtin_trap(); }
static void *registry_churn(void *p) {
    for (int i=0;i<200;i++) {
        pthread_key_t k, old;
        CHECK(pthread_key_create(&k, 0) == 0);
        CHECK(pthread_setspecific(k, p) == 0);
        old=k;
        CHECK(pthread_key_delete(k) == 0);
        CHECK(pthread_getspecific(old) == 0);
        CHECK(pthread_key_create(&k, 0) == 0);
        CHECK(pthread_getspecific(k) == 0);
        CHECK(pthread_setspecific(old, p) == EINVAL);
        CHECK(pthread_key_delete(k) == 0);
    }
    return 0;
}
static void deleted_destructor(void *p) { (void)p; __builtin_trap(); }
static void *run(void *unused) {
    (void)unused;
    pthread_t threads[12]; void *result;
    errno = 123;
    CHECK(pthread_key_create(&key, destructor) == 0);
    CHECK(pthread_setspecific(key, (void *)99) == 0);
    for (uintptr_t i=0;i<12;i++) CHECK(pthread_create(&threads[i],0,isolated,(void *)(i+1)) == 0);
    for (int i=0;i<12;i++) { CHECK(pthread_join(threads[i], &result) == 0); CHECK(result == 0); }
    CHECK(destructed == 12 && pthread_getspecific(key) == (void *)99);
    CHECK(pthread_create(&threads[0],0,explicit_exit,(void *)55) == 0);
    CHECK(pthread_join(threads[0],&result) == 0 && result == (void *)55 && destructed == 13);
    CHECK(pthread_setspecific(key, 0) == 0);
    CHECK(pthread_key_create(&repeat_key, repeat) == 0);
    CHECK(pthread_key_create(&reentrant, reenter) == 0);
    CHECK(pthread_create(&threads[0],0,repeating,(void *)1) == 0);
    CHECK(pthread_join(threads[0],&result) == 0 && result == 0 && repeats == PTHREAD_DESTRUCTOR_ITERATIONS);
    CHECK(pthread_key_delete(repeat_key) == 0);
    CHECK(pthread_key_create(&deleted, deleted_destructor) == 0);
    CHECK(pthread_create(&threads[0],0,delete_race,(void *)1) == 0);
    wait_for(&entered,1);
    CHECK(pthread_key_delete(deleted) == 0);
    pthread_key_t reused; CHECK(pthread_key_create(&reused, 0) == 0 && reused != deleted);
    CHECK(pthread_setspecific(reused, (void *)2) == 0);
    __atomic_store_n(&release_worker,1,__ATOMIC_RELEASE);
    CHECK(pthread_join(threads[0],&result) == 0 && result == 0);
    CHECK(pthread_key_delete(reused) == 0);
    puts("TLS deletion-before-exit and stale-generation witness PASS");
    CHECK(pthread_key_create(&deleted,0) == 0);
    CHECK(pthread_create(&threads[0],0,concurrent_delete,(void *)1) == 0);
    wait_for(&racing,1);
    CHECK(pthread_key_delete(deleted) == 0);
    for(int i=0;i<100;i++) {
        CHECK(pthread_key_create(&reused,0) == 0);
        CHECK(pthread_setspecific(reused,(void *)9) == 0);
        CHECK(pthread_key_delete(reused) == 0);
    }
    __atomic_store_n(&race_stop,1,__ATOMIC_RELEASE);
    CHECK(pthread_join(threads[0],&result) == 0 && result == 0);

    for (uintptr_t i=0;i<12;i++) CHECK(pthread_create(&threads[i],0,registry_churn,(void *)(i+1)) == 0);
    for (int i=0;i<12;i++) { CHECK(pthread_join(threads[i], &result) == 0 && result == 0); }
    for (int i=0;i<12;i++) CHECK(pthread_create(&threads[i],0,once_worker,0) == 0);
    wait_for(&started,12); wait_for(&once_entered,1); usleep(20000);
    CHECK(ready == 0 && once_count == 1);
    __atomic_store_n(&once_release,1,__ATOMIC_RELEASE);
    for (int i=0;i<12;i++) CHECK(pthread_join(threads[i], &result) == 0 && result == 0);
    CHECK(ready == 12 && once_count == 1);
    CHECK(pthread_once(&recursive,recurse) == 0);
    entered=release_worker=started=0;
    CHECK(pthread_create(&threads[0],0,abandon_worker,0) == 0); wait_for(&entered,1);
    CHECK(pthread_create(&threads[1],0,recover_worker,0) == 0); wait_for(&started,1); usleep(20000);
    __atomic_store_n(&release_worker,1,__ATOMIC_RELEASE);
    CHECK(pthread_join(threads[0],&result) == 0 && result == (void *)77);
    CHECK(pthread_join(threads[1],&result) == 0 && result == 0);
    puts("once contention publication nested-init abandonment PASS");
    CHECK(pthread_key_delete(key) == 0);
    CHECK(pthread_key_create(&key,detached_destructor) == 0);
    pthread_attr_t attr; CHECK(pthread_attr_init(&attr) == 0); CHECK(pthread_attr_setdetachstate(&attr,PTHREAD_CREATE_DETACHED) == 0);
    CHECK(pthread_create(&threads[0],&attr,isolated,(void *)1) == 0); wait_for(&detached_done,1);
    for (int i=0;i<80;i++) { CHECK(pthread_create(&threads[0],0,isolated,(void *)1) == 0); CHECK(pthread_join(threads[0],&result) == 0 && result == 0); }
    CHECK(pthread_key_delete(key) == 0);
    pthread_key_t keys[PTHREAD_KEYS_MAX];
    for(int i=0;i<PTHREAD_KEYS_MAX;i++) CHECK(pthread_key_create(&keys[i],0) == 0);
    CHECK(pthread_key_create(&key,0) == EAGAIN);
    for(int i=0;i<PTHREAD_KEYS_MAX;i++) CHECK(pthread_key_delete(keys[i]) == 0);
    CHECK(errno == 123);
    char buffer[128], small[2];
    CHECK(strcmp(strerror(EINVAL),"Invalid argument") == 0);
    CHECK(strcmp(strerror(-1),"Unknown error") == 0);
    CHECK(strerror(EINVAL) == strerror(EINVAL));
    CHECK(strerror_r(EINVAL,buffer,sizeof buffer) == 0 && strcmp(buffer,strerror(EINVAL)) == 0);
    CHECK(strerror_r(EINVAL,small,sizeof small) == ERANGE && small[1] == 0);
    CHECK(strerror_r(-1,buffer,sizeof buffer) == EINVAL && strcmp(buffer,"Unknown error") == 0);
    CHECK(strerror_r(0,small,0) == ERANGE && errno == 123);
    puts("TLS lifecycle exhaustion churn and strerror PASS");
    return 0;
}
int main(void) { return (int)(uintptr_t)run(0); }
