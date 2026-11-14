#include <pthread.h>
#include <stdio.h>
#include <time.h>
static unsigned ready;
static void *worker(void *arg) {
    (void)arg;
    __atomic_store_n(&ready,1,__ATOMIC_RELEASE);
    struct timespec delay = {1,0}; nanosleep(&delay,0);
    puts("BORROWER SURVIVED INITIAL PTHREAD_EXIT");
    return 0;
}
int main(void) {
    pthread_t thread;
    if(pthread_create(&thread,0,worker,0)) return 1;
    while(!__atomic_load_n(&ready,__ATOMIC_ACQUIRE)) { struct timespec delay={0,1000000}; nanosleep(&delay,0); }
    puts("INITIAL PTHREAD_EXIT WITH LIVE BORROWER");
    pthread_exit(0);
}
