#include <pthread.h>
#include <time.h>
#include <string.h>
#include <stdlib.h>
static_assert(sizeof(pthread_cond_t)==24);
static_assert(alignof(pthread_cond_t)==8);
static_assert(sizeof(pthread_condattr_t)==4);
static pthread_cond_t c=PTHREAD_COND_INITIALIZER;
static pthread_mutex_t m=PTHREAD_MUTEX_INITIALIZER;
extern "C" int main() { pthread_condattr_t a; int clock; pthread_condattr_init(&a); pthread_condattr_getclock(&a,&clock); pthread_condattr_setclock(&a,CLOCK_MONOTONIC); pthread_condattr_destroy(&a); pthread_cond_signal(&c); pthread_cond_broadcast(&c); pthread_cond_destroy(&c); return pthread_mutex_destroy(&m); }
