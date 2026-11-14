#include <pthread.h>
#include <stdio.h>
#include <time.h>
#include <stdint.h>
#define N 72
static unsigned ready, release_workers, destroyed;
static pthread_key_t key;
static unsigned load(unsigned *p) { return __atomic_load_n(p,__ATOMIC_ACQUIRE); }
static void wait_count(unsigned *p,unsigned want) {
    for(unsigned n=0;n<30000;n++) { if(load(p)==want) return; struct timespec t={0,1000000}; nanosleep(&t,0); }
    __builtin_trap();
}
static void cleanup(void *p) {
    if(!p) __builtin_trap();
    __atomic_add_fetch(&destroyed,1,__ATOMIC_RELEASE);
}
static void *worker(void *p) {
    if(pthread_setspecific(key,p)) __builtin_trap();
    __atomic_add_fetch(&ready,1,__ATOMIC_RELEASE);
    wait_count(&release_workers,1);
    return 0;
}
int main(void) {
    pthread_attr_t a; pthread_t ids[N];
    puts("D0 baseline created=0 ready=0 callbacks=0");
    if(pthread_key_create(&key,cleanup)||pthread_attr_init(&a)||pthread_attr_setdetachstate(&a,PTHREAD_CREATE_DETACHED)) return 1;
    for(uintptr_t n=0;n<N;n++) if(pthread_create(&ids[n],&a,worker,(void *)(n+1))) return 2;
    wait_count(&ready,N);
    printf("D1 created=%u ready=%u callbacks=%u gate=held\n",N,load(&ready),load(&destroyed));
    __atomic_store_n(&release_workers,1,__ATOMIC_RELEASE);
    wait_count(&destroyed,N);
    printf("D2 created=%u ready=%u callbacks=%u gate=released\n",N,load(&ready),load(&destroyed));
    /* No create after release: do not accidentally sweep the failing queue.
       Callbacks are not kernel-exit acknowledgments. Record a bounded window. */
    for(unsigned n=0;n<100;n++) { struct timespec t={0,1000000}; nanosleep(&t,0); }
    printf("D3 callbacks=%u joins=0 subsequent_creates=0 quiescence_ms=100\n",load(&destroyed));
    return pthread_key_delete(key);
}
