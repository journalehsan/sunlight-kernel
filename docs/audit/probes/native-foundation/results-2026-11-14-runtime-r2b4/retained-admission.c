#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <time.h>
static unsigned alive, exiting;
static unsigned long owner;
static unsigned long raw(unsigned long nr,unsigned long a) {
    unsigned long r; __asm__ volatile("syscall":"=a"(r):"a"(nr),"D"(a),"S"(0UL):"rcx","r11","memory"); return r;
}
static void tick(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void wait(unsigned *p) {
    for(unsigned n=0;n<30000&&!__atomic_load_n(p,__ATOMIC_ACQUIRE);n++) tick();
    if(!__atomic_load_n(p,__ATOMIC_ACQUIRE)) __builtin_trap();
}
static void cleanup(void *p) { (void)p; __atomic_store_n(&exiting,1,__ATOMIC_RELEASE); }
static void *nested(void *p) { puts("A nested thread after original exit"); return p; }
static void *worker(void *p) {
    (void)p; __atomic_store_n(&alive,1,__ATOMIC_RELEASE); wait(&exiting);
    for(unsigned n=0;n<20;n++) tick();
    if(raw(156,owner)!=(unsigned long)-1 || raw(25,owner)!=1) __builtin_trap();
    puts("A retained group liveness=1");
    pthread_t t; void *value=0;
    if(pthread_create(&t,0,nested,(void *)42)||pthread_join(t,&value)||value!=(void *)42) __builtin_trap();
    puts("A post-owner create join PASS");
    return 0;
}
int main(void) {
    pthread_key_t key;pthread_t t;owner=raw(33,0);
    if(pthread_key_create(&key,cleanup)||pthread_setspecific(key,(void *)1)||pthread_create(&t,0,worker,0)) return 1;
    wait(&alive);pthread_exit(0);
}
