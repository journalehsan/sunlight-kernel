#include <unistd.h>
#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <time.h>
#include <errno.h>
#include <string.h>
static pid_t owner,parent,owner_tid;
static unsigned ready,leaving;
static int owner_exit;
static long raw(long n,long a){long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a):"rcx","r11","memory");return r;}
static void tick(void){struct timespec t={0,1000000};nanosleep(&t,0);}
static void *worker(void *arg){
 (void)arg;pid_t tid=gettid();
 if(getpid()!=owner||getppid()!=parent||tid<=0||tid==owner_tid||tid!=raw(33,0))_Exit(81);
 __atomic_store_n(&ready,1,__ATOMIC_RELEASE);
 if(owner_exit){
  for(unsigned i=0;i<10000&&!__atomic_load_n(&leaving,__ATOMIC_ACQUIRE);i++)tick();
  if(!__atomic_load_n(&leaving,__ATOMIC_ACQUIRE))_Exit(82);
  for(int i=0;i<20;i++)tick();
  /* Kernel witness qualifies owner Finished/quiescent; elapsed time alone does not. */
  if(raw(156,owner)!=-1)_Exit(83);
  if(getpid()!=owner||getppid()!=parent||gettid()!=tid)_Exit(84);
  printf("PY4 PROCESS IDENTITY retained owner=%d parent=%d tid=%d PASS\n",owner,parent,tid);
 }else{
  for(int i=0;i<1000;i++)if(getpid()!=owner||getppid()!=parent||gettid()!=tid)_Exit(85);
 }
 return (void *)(uintptr_t)tid;
}
int main(int argc,char **argv){
 owner_exit=argc>1&&!strcmp(argv[1],"owner-exit");errno=71;
 owner=getpid();parent=getppid();owner_tid=gettid();
 if(owner<=0||parent<0||owner_tid!=owner||owner!=raw(33,0)||errno!=71)return 1;
 if(raw(178,3)!=-6||raw(178,-1)!=-6)return 2;
 pthread_t t;if(pthread_create(&t,0,worker,0))return 3;
 if(owner_exit){
  for(unsigned i=0;i<10000&&!__atomic_load_n(&ready,__ATOMIC_ACQUIRE);i++)tick();
  if(!__atomic_load_n(&ready,__ATOMIC_ACQUIRE))return 4;
  __atomic_store_n(&leaving,1,__ATOMIC_RELEASE);pthread_exit(0);
 }
 void *r=0;if(pthread_join(t,&r)||!r)return 5;
 puts("PY4 PROCESS IDENTITY live group/thread/parent PASS");return 0;
}
