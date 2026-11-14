/* White-box probe of the unchanged static runtime. __r2b2_queue is bound by
 * the linker to pthread::DETACHED, then verified against the final ELF symbol.
 * No runtime writer exists while snapshots are read: all workers are Reaped,
 * the initial thread is the only creator, and no subsequent create has run. */
#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <time.h>
#define N 12
struct control {
    unsigned state, detached;
    void *retval, *start, *arg;
    uintptr_t tid, stack_base, stack_size, tls_base, tls_size;
};
_Static_assert(sizeof(struct control)==72,"Control audit layout");
extern uintptr_t __r2b2_queue[];
static unsigned ready, release_workers, destroyed;
static pthread_key_t key;
static unsigned load(unsigned *p) { return __atomic_load_n(p,__ATOMIC_ACQUIRE); }
static void tick(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void wait_count(unsigned *p,unsigned want) {
    for(unsigned n=0;n<30000;n++) { if(load(p)==want) return; tick(); }
    __builtin_trap();
}
static unsigned long reaped(uintptr_t tid) {
    unsigned long result;
    __asm__ volatile("syscall":"=a"(result):"a"(156UL),"D"(tid):"rcx","r11","memory");
    return result;
}
static void snapshot(unsigned wave,const char *phase,unsigned expected) {
    unsigned count=0; uintptr_t stacks=0,tls=0;
    for(unsigned n=0;n<64;n++) if(__r2b2_queue[n]) {
        struct control *c=(struct control *)__r2b2_queue[n];
        if(c->state!=1||c->detached!=2||reaped(c->tid)!=1) __builtin_trap();
        count++; stacks+=c->stack_size; tls+=c->tls_size;
    }
    printf("A wave=%u phase=%s queued=%u control_bytes=%lu stack_bytes=%lu tls_bytes=%lu\n",wave,phase,count,(unsigned long)count*sizeof(struct control),(unsigned long)stacks,(unsigned long)tls);
    if(count!=expected) __builtin_trap();
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
static void *sentinel(void *p) { return p; }
int main(void) {
    pthread_attr_t a; pthread_t ids[N];
    if(pthread_key_create(&key,cleanup)||pthread_attr_init(&a)||pthread_attr_setdetachstate(&a,PTHREAD_CREATE_DETACHED)) return 1;
    snapshot(0,"baseline",0);
    for(unsigned wave=1;wave<=4;wave++) {
        ready=release_workers=destroyed=0;
        for(uintptr_t n=0;n<N;n++) if(pthread_create(&ids[n],&a,worker,(void *)(n+1))) return 2;
        wait_count(&ready,N);
        snapshot(wave,"live-workers-gated",0);
        __atomic_store_n(&release_workers,1,__ATOMIC_RELEASE);
        wait_count(&destroyed,N);
        for(unsigned n=0;n<N;n++) {
            struct control *c=(struct control *)ids[n];
            unsigned attempts=0;
            while(reaped(c->tid)!=1) { if(++attempts>30000) __builtin_trap(); tick(); }
        }
        snapshot(wave,"all-kernel-reaped",N);
        for(unsigned n=0;n<100;n++) tick();
        snapshot(wave,"bounded-quiescence",N);
        printf("A wave=%u created=%u callbacks=%u kernel_reaped=%u joins=0\n",wave,N,load(&destroyed),N);
    }
    pthread_t s; void *ret;
    if(pthread_create(&s,0,sentinel,(void *)9)||pthread_join(s,&ret)||ret!=(void *)9) return 3;
    snapshot(5,"explicit-create-sweep-and-join",0);
    return pthread_key_delete(key);
}
