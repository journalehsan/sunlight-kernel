#include <pthread.h>
#include <stdatomic.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#define CHECK(x) do{if(!(x)){printf("timeout FAIL line=%d\n",__LINE__);exit(91);}}while(0)
static pthread_mutex_t m=PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c;
static atomic_int entered;
static int outcome;
static void sleep_ns(long n){struct timespec t={0,n};CHECK(!nanosleep(&t,0));}
static void *waiter(void *unused){(void)unused;CHECK(!pthread_mutex_lock(&m));struct timespec t;CHECK(!clock_gettime(CLOCK_MONOTONIC,&t));t.tv_nsec+=20000000;if(t.tv_nsec>=1000000000){t.tv_nsec-=1000000000;t.tv_sec++;}atomic_store(&entered,1);
 outcome=pthread_cond_timedwait(&c,&m,&t);CHECK(outcome==0||outcome==ETIMEDOUT);CHECK(pthread_mutex_trylock(&m)==EBUSY);CHECK(!pthread_mutex_unlock(&m));return 0;}
int main(void){pthread_condattr_t a;CHECK(!pthread_condattr_init(&a));CHECK(!pthread_condattr_setclock(&a,CLOCK_MONOTONIC));CHECK(!pthread_cond_init(&c,&a));int wakes=0,timeouts=0;
 for(int i=0;i<60;i++){atomic_store(&entered,0);pthread_t t;CHECK(!pthread_create(&t,0,waiter,0));while(!atomic_load(&entered))sleep_ns(10000000);
 CHECK(!pthread_mutex_lock(&m));CHECK(!pthread_mutex_unlock(&m));if(i%3)sleep_ns(i%3==1?10000000:40000000);
 CHECK(!pthread_cond_signal(&c));CHECK(!pthread_join(t,0));if(outcome==0)wakes++;else timeouts++;}
 CHECK(wakes>0&&timeouts>0);CHECK(!pthread_cond_destroy(&c));printf("PY1 signal/timeout stress PASS wakes=%d timeouts=%d; exact race UNQUALIFIED\n",wakes,timeouts);return 0;}
