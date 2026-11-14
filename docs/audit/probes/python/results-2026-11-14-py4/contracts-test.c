/* Observe blocked native contracts without pretending that they work. */
#include <stdio.h>
#include <stdint.h>
#include <pthread.h>
#include <signal.h>
#include <errno.h>
#include <sys/random.h>
static long raw(long n){long r;__asm__ volatile("syscall":"=a"(r):"a"(n):"rcx","r11","memory");return r;}
static long owner,worker_pid,worker_generation;
static void *worker(void *arg){(void)arg;worker_pid=raw(33);worker_generation=raw(141);return 0;}
int main(void){
 /* Syscalls 33 and 141 observe native task and address-space generation. */
 owner=raw(33);long generation=raw(141);pthread_t t;if(pthread_create(&t,0,worker,0)||pthread_join(t,0))return 1;
 if(owner<=0||worker_pid<=0||owner==worker_pid||generation!=worker_generation)return 2;
 errno=0;if(kill((int)owner,SIGUSR1)!=-1||errno!=ENOSYS)return 3;
 sigset_t set=0;errno=0;if(sigprocmask(SIG_BLOCK,&set,0)!=-1||errno!=ENOSYS)return 4;
 unsigned char b[9];for(unsigned i=0;i<sizeof(b);i++)b[i]=0xa5;
 errno=0;if(getrandom(b+1,7,1)!=-1||errno!=EINVAL)return 5;
 for(unsigned i=0;i<sizeof(b);i++)if(b[i]!=0xa5)return 6;
 printf("PY4 CONTRACT LIMITS PASS native_owner=%ld worker=%ld generation=%ld; historical task IDs distinct; masking, nonblocking entropy BLOCKED\n",owner,worker_pid,worker_generation);return 0;
}
