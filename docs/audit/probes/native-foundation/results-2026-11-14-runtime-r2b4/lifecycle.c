#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>
static unsigned ready, exiting, done;
static unsigned long owner, tids[2];
static int count, mode;
static pthread_key_t key;
static unsigned long raw(unsigned long nr,unsigned long a,unsigned long b) {
    unsigned long r; __asm__ volatile("syscall":"=a"(r):"a"(nr),"D"(a),"S"(b):"rcx","r11","memory"); return r;
}
static unsigned load(unsigned *p) { return __atomic_load_n(p,__ATOMIC_ACQUIRE); }
static void tick(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void wait_ready(unsigned *p,unsigned n) {
    for(unsigned i=0;i<30000&&load(p)<n;i++) tick();
    if(load(p)<n) __builtin_trap();
}
static void destructor(void *p) {
    printf("W tls actor=%lu tid=%lu\n",(unsigned long)(uintptr_t)p,raw(33,0,0));
    if((uintptr_t)p==3) __atomic_store_n(&exiting,1,__ATOMIC_RELEASE);
}
static void *worker(void *p) {
    unsigned id=(unsigned)(uintptr_t)p;
    tids[id]=raw(33,0,0);
    if(pthread_setspecific(key,(void *)(uintptr_t)(id+1))) __builtin_trap();
    printf("W alive actor=%u tid=%lu owner=%lu mode=%d\n",id,tids[id],owner,mode);
    __atomic_fetch_add(&ready,1,__ATOMIC_RELEASE);
    wait_ready(&exiting,1);
    for(unsigned n=0;n<20;n++) tick();
    /* Observation-only query retains its public rejection result. The kernel
       trace must independently show Finished + no execution pin/current/queue
       before the following worker progress; elapsed time alone qualifies nothing. */
    if(raw(156,owner,0)!=(unsigned long)-1) __builtin_trap();
    printf("W progress actor=%u tid=%lu after-owner-query\n",id,tids[id]);
    if(mode==2 && id==0) {
        puts("W forced retained-owner SIGKILL");
        raw(72,owner,9);
        for(unsigned n=0;n<30000;n++) tick();
        __builtin_trap();
    }
    if(id==1) {
        wait_ready(&done,1);
        for(unsigned n=0;n<30000;n++) {
            if(raw(156,tids[0],0)==1) {
                puts("W first-borrower-reaped final-borrower-exiting");
                return 0;
            }
            tick();
        }
        __builtin_trap();
    }
    __atomic_store_n(&done,1,__ATOMIC_RELEASE);
    return 0;
}
int main(int argc,char **argv) {
    if(argc!=2) return 1;
    mode=argv[1][0]-'0'; count=mode==0?1:2; owner=raw(33,0,0);
    if(pthread_key_create(&key,destructor)||pthread_setspecific(key,(void *)3)) return 2;
    for(int i=0;i<count;i++) { pthread_t t; if(pthread_create(&t,0,worker,(void *)(uintptr_t)i)) return 3; }
    wait_ready(&ready,(unsigned)count);
    printf("W initial pthread_exit owner=%lu workers=%d\n",owner,count);
    pthread_exit(0);
}
