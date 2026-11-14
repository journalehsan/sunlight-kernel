#include <pthread.h>
#include <time.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>
#define CHECK(x) do { if (!(x)) { printf("PY1 FAIL line=%d\n", __LINE__); exit(91); } } while(0)
_Static_assert(sizeof(pthread_cond_t)==24 && _Alignof(pthread_cond_t)==8,"cond ABI");
static pthread_mutex_t m=PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c=PTHREAD_COND_INITIALIZER;
static atomic_int registered, completed;
static int ready;
static void *worker(void *unused) {
 (void)unused; CHECK(!pthread_mutex_lock(&m)); atomic_fetch_add(&registered,1);
 while(!ready) CHECK(!pthread_cond_wait(&c,&m));
 CHECK(pthread_mutex_trylock(&m)==EBUSY); atomic_fetch_add(&completed,1);
 CHECK(!pthread_mutex_unlock(&m));return 0;
}
static void pause_short(void) { struct timespec t={0,10000000};CHECK(!nanosleep(&t,0)); }
static void wait_registered(int n) { for(int i=0;i<1000 && atomic_load(&registered)!=n;i++) pause_short(); CHECK(atomic_load(&registered)==n); }
static void deadline(struct timespec *t,int clock) {CHECK(!clock_gettime(clock,t));t->tv_nsec+=30000000;if(t->tv_nsec>=1000000000){t->tv_nsec-=1000000000;t->tv_sec++;}}
int main(void) {
 CHECK(pthread_cond_wait(0,&m)==EINVAL);CHECK(pthread_cond_signal(0)==EINVAL);
 CHECK(!pthread_cond_signal(&c)); /* signal before wait is not remembered */
 CHECK(!pthread_mutex_lock(&m));struct timespec t;deadline(&t,CLOCK_REALTIME);
 CHECK(pthread_cond_timedwait(&c,&m,&t)==ETIMEDOUT);CHECK(pthread_mutex_trylock(&m)==EBUSY);
 t.tv_nsec=1000000000;CHECK(pthread_cond_timedwait(&c,&m,&t)==EINVAL);CHECK(!pthread_mutex_unlock(&m));
 pthread_condattr_t a;int clock=-1;CHECK(!pthread_condattr_init(&a));CHECK(!pthread_condattr_getclock(&a,&clock)&&clock==CLOCK_REALTIME);
 CHECK(pthread_condattr_setclock(&a,7)==EINVAL);CHECK(!pthread_condattr_setclock(&a,CLOCK_MONOTONIC));
 CHECK(!pthread_cond_destroy(&c));CHECK(pthread_cond_signal(&c)==EINVAL);CHECK(!pthread_cond_init(&c,&a));CHECK(!pthread_condattr_destroy(&a));
 CHECK(!pthread_mutex_lock(&m));deadline(&t,CLOCK_MONOTONIC);CHECK(pthread_cond_timedwait(&c,&m,&t)==ETIMEDOUT);CHECK(!pthread_mutex_unlock(&m));
 for(int round=0;round<40;round++) {
  pthread_t ts[4]; ready=0;atomic_store(&registered,0);atomic_store(&completed,0);
  for(int i=0;i<4;i++)CHECK(!pthread_create(&ts[i],0,worker,0));
  wait_registered(4);CHECK(!pthread_mutex_lock(&m)); /* each registered waiter released m */
  CHECK(pthread_cond_destroy(&c)==EBUSY);
  CHECK(!pthread_cond_broadcast(&c)); /* predicate false: spurious notification */
  CHECK(!pthread_mutex_unlock(&m));pause_short();CHECK(atomic_load(&completed)==0);
  CHECK(!pthread_mutex_lock(&m));ready=1;CHECK(!pthread_cond_signal(&c));CHECK(!pthread_cond_broadcast(&c));CHECK(!pthread_mutex_unlock(&m));
  for(int i=0;i<4;i++)CHECK(!pthread_join(ts[i],0));CHECK(atomic_load(&completed)==4);
 }
 /* A single signal must complete a waiter without a broadcast. */
 ready=0;atomic_store(&registered,0);pthread_t one;CHECK(!pthread_create(&one,0,worker,0));wait_registered(1);
 CHECK(!pthread_mutex_lock(&m));ready=1;CHECK(!pthread_cond_signal(&c));CHECK(!pthread_mutex_unlock(&m));CHECK(!pthread_join(one,0));
 CHECK(!pthread_mutex_lock(&m));t.tv_sec=-1;t.tv_nsec=0;CHECK(pthread_cond_timedwait(&c,&m,&t)==ETIMEDOUT);CHECK(pthread_mutex_trylock(&m)==EBUSY);CHECK(!pthread_mutex_unlock(&m));
 CHECK(!pthread_cond_destroy(&c));puts("PY1 condvar PASS: 40 rounds x 4 waiters; realtime/monotonic; busy destroy; reacquire");return 0;
}
