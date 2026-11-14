#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <errno.h>
#include <time.h>
static pthread_key_t old_key, new_key, callback_key;
static unsigned phase, release_worker, callback_open, release_callback, callbacks;
static void wait_for(unsigned *p,unsigned value) {
    for(unsigned n=0;n<30000;n++) {
        if(__atomic_load_n(p,__ATOMIC_ACQUIRE)==value) return;
        struct timespec t={0,1000000}; nanosleep(&t,0);
    }
    __builtin_trap();
}
static void store(unsigned *p,unsigned n) { __atomic_store_n(p,n,__ATOMIC_RELEASE); }
static void forbidden(void *p) { (void)p; __builtin_trap(); }
static void *stale_worker(void *p) {
    if(pthread_setspecific(old_key,p)) __builtin_trap();
    printf("T1 holder=%p old=%lu index=%lu generation=%lu cell=populated\n",(void *)pthread_self(),old_key,old_key&127,old_key>>7);
    store(&phase,1); wait_for(&release_worker,1);
    if(pthread_getspecific(old_key)||pthread_setspecific(old_key,p)!=EINVAL||pthread_getspecific(new_key)) __builtin_trap();
    if(pthread_setspecific(new_key,(void *)7)) __builtin_trap();
    puts("T3 old_get=NULL old_set=EINVAL reused_get=NULL new_set=0");
    return 0;
}
static void callback(void *p) {
    pthread_key_t temporary;
    if(p!=(void *)3||pthread_getspecific(callback_key)||pthread_key_create(&temporary,0)||pthread_setspecific(temporary,(void *)4)||pthread_getspecific(temporary)!=(void *)4||pthread_key_delete(temporary)) __builtin_trap();
    /* Registry operations inside the callback prove it holds no registry lock. */
    __atomic_add_fetch(&callbacks,1,__ATOMIC_RELAXED);
    printf("T4 callback=%p key=%lu generation=%lu cell=cleared reentrancy=complete\n",(void *)pthread_self(),callback_key,callback_key>>7);
    store(&callback_open,1); wait_for(&release_callback,1);
    puts("T6 selected callback completed after delete/reuse");
}
static void *exiting_worker(void *p) {
    if(pthread_setspecific(callback_key,p)) __builtin_trap();
    pthread_exit((void *)8);
}
int main(void) {
    pthread_t t; void *result;
    if(pthread_key_create(&old_key,forbidden)||pthread_create(&t,0,stale_worker,(void *)1)) return 1;
    wait_for(&phase,1);
    if(pthread_key_delete(old_key)||pthread_key_create(&new_key,0)||old_key==new_key||(old_key&127)!=(new_key&127)) return 2;
    printf("T2 deleting=%p old=%lu reused=%lu old_generation=%lu new_generation=%lu holder=blocked\n",(void *)pthread_self(),old_key,new_key,old_key>>7,new_key>>7);
    store(&release_worker,1);
    if(pthread_join(t,&result)||result||pthread_key_delete(new_key)) return 3;
    if(pthread_key_create(&callback_key,callback)||pthread_create(&t,0,exiting_worker,(void *)3)) return 4;
    wait_for(&callback_open,1);
    if(pthread_key_delete(callback_key)||pthread_key_create(&new_key,0)||new_key==callback_key) return 5;
    printf("T5 delete/reuse while callback held old=%lu reused=%lu\n",callback_key,new_key);
    store(&release_callback,1);
    if(pthread_join(t,&result)||result!=(void *)8||callbacks!=1||pthread_key_delete(new_key)) return 6;
    puts("T7 joins=2 callbacks=1 outstanding_joinable=0 keys_live=0");
    return 0;
}
