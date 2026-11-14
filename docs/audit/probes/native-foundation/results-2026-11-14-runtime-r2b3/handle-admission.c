#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <errno.h>
#include <time.h>
#define N 64
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
    for(unsigned i=0;i<N;i++) if(pthread_join(t[i],&ret)||ret!=(void *)(uintptr_t)(i+1)) return 5;
    if(pthread_create(&extra,0,worker,(void *)99)||pthread_join(extra,&ret)||ret!=(void *)99) return 6;
    for(unsigned i=0;i<N;i++) if(pthread_join(t[i],0)!=EINVAL||pthread_detach(t[i])!=EINVAL) return 7;
    puts("ADMISSION released=64; subsequent create/join=PASS; 64 stale identities rejected");
    return 0;
}
