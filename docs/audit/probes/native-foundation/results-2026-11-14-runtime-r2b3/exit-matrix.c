#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>
static unsigned ready;
static int mode;
static unsigned long raw(unsigned long nr,unsigned long a,unsigned long b) {
    unsigned long r; __asm__ volatile("syscall":"=a"(r):"a"(nr),"D"(a),"S"(b):"rcx","r11","memory"); return r;
}
static void tick(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void callback(void) { puts("EXIT atexit callback"); }
static void *worker(void *p) {
    puts("EXIT live worker established");
    __atomic_store_n(&ready,1,__ATOMIC_RELEASE);
    if(mode==3) {
        puts("EXIT worker issuing external owner SIGKILL");
        unsigned long rc=raw(72,(uintptr_t)p,9);
        printf("EXIT kill return=%lu\n",rc);
        return (void *)(uintptr_t)rc;
    }
    for(unsigned i=0;i<30000;i++) tick();
    __builtin_trap();
}
int main(int argc,char **argv) {
    if(argc<2) return 1;
    mode=argv[1][0]-'0';
    pthread_t t;
    if(atexit(callback)||pthread_create(&t,0,worker,(void *)(uintptr_t)raw(33,0,0))) return 2;
    for(unsigned i=0;i<30000&&!__atomic_load_n(&ready,__ATOMIC_ACQUIRE);i++) tick();
    if(!__atomic_load_n(&ready,__ATOMIC_ACQUIRE)) return 3;
    if(mode==0) { puts("EXIT returning main"); return 17; }
    if(mode==1) { puts("EXIT calling exit"); exit(23); }
    if(mode==2) { puts("EXIT callback-free group syscall157 (not public _Exit)"); raw(157,29,0); return 4; }
    for(unsigned i=0;i<30000;i++) tick();
    return 5;
}
