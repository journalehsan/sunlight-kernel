#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <errno.h>
#include <time.h>
#define N 64
struct control { unsigned state, detached; void *retval,*start,*arg;
    uintptr_t tid,stack_base,stack_size,tls_base,tls_size,handle,next;
    unsigned char joining,published; };
_Static_assert(sizeof(struct control)==96,"Control audit layout");
extern uintptr_t __r3_queue[];
static unsigned snapshot(const char *phase) {
    unsigned count=0; uintptr_t stack=0,tls=0;
    for(unsigned i=0;i<N;i++) if(__r3_queue[i]) {
        struct control *c=(struct control *)__r3_queue[i];
        if(c->state!=1||c->detached!=2) __builtin_trap();
        count++;stack+=c->stack_size;tls+=c->tls_size;
    }
    printf("CAPACITY phase=%s queued=%u control_bytes=%lu stack_bytes=%lu tls_bytes=%lu\n",phase,count,(unsigned long)count*sizeof(struct control),(unsigned long)stack,(unsigned long)tls);
    return count;
}
static uintptr_t tid;
static unsigned ready;
static unsigned long query(unsigned long nr,unsigned long arg) {
    unsigned long r; __asm__ volatile("syscall":"=a"(r):"a"(nr),"D"(arg):"rcx","r11","memory"); return r;
}
static void pause_one(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void *worker(void *p) {
    __atomic_store_n(&tid,query(33,0),__ATOMIC_RELEASE);
    __atomic_store_n(&ready,1,__ATOMIC_RELEASE);
    return p;
}
int main(void) {
    pthread_t t[N],extra; void *ret;
    for(unsigned i=0;i<N;i++) {
        __atomic_store_n(&ready,0,__ATOMIC_RELEASE);
        if(pthread_create(&t[i],0,worker,(void *)(uintptr_t)(i+1))) return 1;
        unsigned n=0;
        while(!__atomic_load_n(&ready,__ATOMIC_ACQUIRE)) { if(++n>30000) return 2; pause_one(); }
        n=0;
        while(query(156,__atomic_load_n(&tid,__ATOMIC_ACQUIRE))!=1) { if(++n>30000) return 3; pause_one(); }
    }
    if(pthread_create(&extra,0,worker,0)!=EAGAIN) return 4;
    puts("ADMISSION 64 retained joinable controls; kernel reaped=64; 65th create=EAGAIN");
    for(unsigned i=0;i<N;i++) if(pthread_detach(t[i])) return 5;
    if(snapshot("after-64-detaches")!=64) return 8;
    for(unsigned i=0;i<100;i++) pause_one();
    if(snapshot("quiescent-without-create")!=64) return 9;
    if(pthread_create(&extra,0,worker,(void *)99)||pthread_join(extra,&ret)||ret!=(void *)99) return 6;
    if(snapshot("explicit-create-sweep")!=0) return 10;
    for(unsigned i=0;i<N;i++) if(pthread_join(t[i],0)!=EINVAL||pthread_detach(t[i])!=EINVAL) return 7;
    puts("CAPACITY explicit-sweep released=64; subsequent create/join=PASS; 64 stale identities rejected");
    return 0;
}
