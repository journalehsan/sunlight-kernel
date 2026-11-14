#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <errno.h>
#include <time.h>
static unsigned release_worker;
static void pause_one(void) { struct timespec t={0,1000000}; nanosleep(&t,0); }
static void *worker(void *p) {
    while(!__atomic_load_n(&release_worker,__ATOMIC_ACQUIRE)) pause_one();
    return p;
}
int main(void) {
    pthread_t stale=0, current; void *result;
    __atomic_store_n(&release_worker,1,__ATOMIC_RELEASE);
    if(pthread_create(&stale,0,worker,(void *)7)||pthread_join(stale,&result)||result!=(void *)7) return 1;
    for(unsigned i=0;i<256;i++) {
        __atomic_store_n(&release_worker,0,__ATOMIC_RELEASE);
        if(pthread_create(&current,0,worker,(void *)(uintptr_t)(i+8))) return 2;
        if(current==stale) {
            printf("ABA public handle reused old=%p current=%p iteration=%u\n",(void *)stale,(void *)current,i);
            int rc=pthread_detach(stale);
            printf("ABA stale detach result=%d (EINVAL required)\n",rc);
            __atomic_store_n(&release_worker,1,__ATOMIC_RELEASE);
            return 3;
        }
        if(pthread_detach(stale)!=EINVAL) return 4;
        if(pthread_join(stale,0)!=EINVAL) return 5;
        __atomic_store_n(&release_worker,1,__ATOMIC_RELEASE);
        if(pthread_join(current,&result)||result!=(void *)(uintptr_t)(i+8)) return 6;
        if(i==0||i==255) printf("HANDLE generation old=%p current=%p stale rejected iteration=%u\n",(void *)stale,(void *)current,i);
        stale=current;
    }
    puts("HANDLE 256 reuse cycles: stale join/detach rejected; live joins intact");
    return 0;
}
