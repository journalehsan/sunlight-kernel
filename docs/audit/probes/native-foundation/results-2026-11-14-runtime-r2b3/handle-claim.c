/* Test-only symbol aliases, checked against final ELF by build-handle-claim.py.
 * Registry reads take the actual runtime lock; no runtime code is patched. */
#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <errno.h>
#include <time.h>
struct control { unsigned state,detached; void *retval,*start,*arg;
    uintptr_t tid,stack_base,stack_size,tls_base,tls_size,handle,next;
    unsigned char joining,published; };
_Static_assert(sizeof(struct control)==96,"Control layout");
extern uintptr_t __r3_controls;
extern unsigned char __r3_lock;
static unsigned victim_ready,release_victim;
static void tick(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void lock_registry(void) {
    for(unsigned n=0;n<30000;n++) {
        unsigned char expected=0;
        if(__atomic_compare_exchange_n(&__r3_lock,&expected,1,0,__ATOMIC_ACQUIRE,__ATOMIC_RELAXED)) return;
        tick();
    }
    __builtin_trap();
}
static void unlock_registry(void) { __atomic_store_n(&__r3_lock,0,__ATOMIC_RELEASE); }
static uintptr_t wait_claim(pthread_t victim) {
    for(unsigned n=0;n<30000;n++) {
        uintptr_t tid=0;
        lock_registry();
        for(struct control *c=(struct control *)__r3_controls;c;c=(struct control *)c->next) {
            if(c->handle==(uintptr_t)victim && c->joining) { tid=c->tid; break; }
        }
        unlock_registry();
        if(tid) return tid;
        tick();
    }
    __builtin_trap();
}
static void *victim(void *p) {
    __atomic_store_n(&victim_ready,1,__ATOMIC_RELEASE);
    for(unsigned n=0;n<30000;n++) {
        if(__atomic_load_n(&release_victim,__ATOMIC_ACQUIRE)) return p;
        tick();
    }
    __builtin_trap();
}
static void *joiner(void *p) {
    void *result=0;
    int rc=pthread_join((pthread_t)p,&result);
    if(rc||result!=(void *)42) __builtin_trap();
    return 0;
}
int main(void) {
    for(unsigned round=0;round<32;round++) {
        pthread_t target,joining; void *result;
        __atomic_store_n(&victim_ready,0,__ATOMIC_RELEASE);
        __atomic_store_n(&release_victim,0,__ATOMIC_RELEASE);
        if(pthread_create(&target,0,victim,(void *)42)) return 1;
        for(unsigned n=0;!__atomic_load_n(&victim_ready,__ATOMIC_ACQUIRE);n++) { if(n>30000) return 2; tick(); }
        if(pthread_create(&joining,0,joiner,(void *)target)) return 3;
        uintptr_t tid=wait_claim(target);
        /* The actual join owns the Control and cannot finish: target is gated.
         * Both conflicting calls execute before that target may exit. */
        if(pthread_detach(target)!=EINVAL||pthread_join(target,0)!=EINVAL) return 4;
        if(round==0||round==31) printf("CLAIM round=%u target=%p tid=%lu joiner=%p joining=1 target=live detach=EINVAL second_join=EINVAL\n",round,(void *)target,(unsigned long)tid,(void *)joining);
        __atomic_store_n(&release_victim,1,__ATOMIC_RELEASE);
        if(pthread_join(joining,&result)||result) return 5;
        if(pthread_join(target,0)!=EINVAL||pthread_detach(target)!=EINVAL) return 6;
        lock_registry();
        unsigned empty=__r3_controls==0;
        unlock_registry();
        if(!empty) return 7;
    }
    puts("CLAIM 32 native reserved-join overlaps; exactly one join owner; registry controls=0 after each round");
    return 0;
}
