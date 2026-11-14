#include <pthread.h>
#include <stdio.h>
#include <time.h>
#include <stdint.h>
static unsigned alive, exiting;
static pthread_key_t key;
static unsigned load(unsigned *p) { return __atomic_load_n(p,__ATOMIC_ACQUIRE); }
static void store(unsigned *p) { __atomic_store_n(p,1,__ATOMIC_RELEASE); }
static void pause_one(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void cleanup(void *p) {
    if ((uintptr_t)p==1) { puts("I2 initial TLS cleanup entered"); store(&exiting); }
    else puts("I5 worker TLS cleanup completed");
}
static void *worker(void *p) {
    (void)p;
    if(pthread_setspecific(key,(void *)2)) __builtin_trap();
    puts("I1 worker independently alive"); store(&alive);
    for(unsigned n=0;n<30000 && !load(&exiting);n++) pause_one();
    if(!load(&exiting)) __builtin_trap();
    /* This deadline starts only after initial pthread_exit cleanup is observed.
       It is an observation window, not a claimed post-syscall barrier. */
    for(unsigned n=0;n<100;n++) pause_one();
    puts("I4 worker survival observed");
    return 0;
}
int main(void) {
    pthread_t t;
    if(pthread_key_create(&key,cleanup)||pthread_setspecific(key,(void *)1)||pthread_create(&t,0,worker,0)) return 1;
    for(unsigned n=0;n<30000 && !load(&alive);n++) pause_one();
    if(!load(&alive)) return 2;
    puts("I3 initial calling pthread_exit with live worker");
    pthread_exit(0);
}
