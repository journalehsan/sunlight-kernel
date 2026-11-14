#include <pthread.h>
#include <stdio.h>
#include <time.h>
static pthread_once_t control=PTHREAD_ONCE_INIT;
static unsigned entered, returned;
static void init(void) {
    __atomic_store_n(&entered,1,__ATOMIC_RELEASE);
    pthread_once(&control,init);
    __atomic_store_n(&returned,1,__ATOMIC_RELEASE);
}
static void *worker(void *arg) { (void)arg; pthread_once(&control,init); return 0; }
static void *progress(void *arg) { return arg; }
int main(void) {
    pthread_t blocked, witness; void *result;
    if(pthread_create(&blocked,0,worker,0)) return 1;
    while(!__atomic_load_n(&entered,__ATOMIC_ACQUIRE)) { struct timespec delay={0,1000000}; nanosleep(&delay,0); }
    if(pthread_create(&witness,0,progress,(void *)7) || pthread_join(witness,&result) || result!=(void *)7) return 2;
    struct timespec delay={0,50000000}; nanosleep(&delay,0);
    if(__atomic_load_n(&returned,__ATOMIC_ACQUIRE)) return 3;
    puts("recursive once: no completion in bounded window; independent thread progressed");
    /* Process exit releases the deliberately blocked worker; the outer guest
     * runner also imposes a 90-second host wall-clock bound. */
    return 0;
}
