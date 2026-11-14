#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <time.h>
static pthread_key_t key;
static unsigned entered, release_callback, calls;
static void wait_for(unsigned *word) {
    while (!__atomic_load_n(word,__ATOMIC_ACQUIRE)) {
        struct timespec delay={0,1000000}; nanosleep(&delay,0);
    }
}
static void destructor(void *p) {
    if(p != (void *)1 || pthread_getspecific(key)) __builtin_trap();
    __atomic_add_fetch(&calls,1,__ATOMIC_RELAXED);
    __atomic_store_n(&entered,1,__ATOMIC_RELEASE);
    wait_for(&release_callback);
    /* Already selected callback stays safe after deletion and key reuse. */
}
static void *worker(void *p) {
    if(pthread_setspecific(key,p)) return (void *)1;
    return 0;
}
int main(void) {
    pthread_t thread; void *result; pthread_key_t replacement;
    if(pthread_key_create(&key,destructor) || pthread_create(&thread,0,worker,(void *)1)) return 1;
    wait_for(&entered);
    if(pthread_key_delete(key) || pthread_key_create(&replacement,0) || replacement == key) return 2;
    if(pthread_setspecific(replacement,(void *)9)) return 3;
    __atomic_store_n(&release_callback,1,__ATOMIC_RELEASE);
    if(pthread_join(thread,&result) || result || calls != 1 || pthread_getspecific(replacement) != (void *)9) return 4;
    if(pthread_key_delete(replacement)) return 5;
    puts("in-flight destructor delete/reuse witness PASS");
    return 0;
}
