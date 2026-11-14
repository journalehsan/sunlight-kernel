#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
static unsigned ready, endpoint_ready, owner_exiting, stage;
static unsigned long owner, creator_tid, token, first, second, endpoint, public_endpoint;
static int mode;
static pthread_key_t key;
static unsigned long raw(unsigned long nr,unsigned long a,unsigned long b) {
    unsigned long r; __asm__ volatile("syscall":"=a"(r):"a"(nr),"D"(a),"S"(b):"rcx","r11","memory"); return r;
}
static void check(int ok,const char *what) { if(!ok) {printf("R2B5 FAIL %s\n",what);raw(157,91,0);__builtin_trap();} }
static unsigned load(unsigned *p) {return __atomic_load_n(p,__ATOMIC_ACQUIRE);}
static void tick(void) {struct timespec t={0,1000000};nanosleep(&t,0);}
static void wait_for(unsigned *p,unsigned n) {for(unsigned i=0;i<10000&&load(p)<n;i++)tick();check(load(p)>=n,"bounded rendezvous");}
static void set_stage(unsigned n) {__atomic_store_n(&stage,n,__ATOMIC_RELEASE);}
static void destructor(void *p) {(void)p;__atomic_store_n(&owner_exiting,1,__ATOMIC_RELEASE);}
/* Register transport matches kernel/src/ipc/message.rs; empty payloads. */
static unsigned long ipc(unsigned long nr,unsigned long ep,unsigned long *label) {
 unsigned long result,counts=0,object=ep;
 __asm__ volatile("syscall":"=a"(result),"+D"(*label),"+S"(object),"+d"(counts):"a"(nr):"rcx","r11","r8","r9","r10","r12","r13","r14","memory");
 return result;
}
static void exchange(unsigned id,unsigned round) {
 unsigned long label=id==0?0:0xb500+round,result=3;
 for(unsigned i=0;i<10000&&result==3;i++)result=ipc(id==0?4:1,id==0?endpoint:public_endpoint,&label);
 check(result==0,"IPC roundtrip transport");
 check(label==(id==0?0xb500UL+round:0xac00UL+round),"IPC roundtrip label");
 if(id==0){label=0xac00+round;check(ipc(2,0,&label)==0,"IPC reply");}
 printf("R2B5 IPC round=%u actor=%u PASS\n",round,id);
}
static void *worker(void *arg) {
 unsigned id=(unsigned)(uintptr_t)arg;
 unsigned long tid=raw(33,0,0);
 if(id==0) {creator_tid=tid;endpoint=raw(10,0,0);check(endpoint!=0,"endpoint create");public_endpoint=raw(11,endpoint,0);check(public_endpoint!=0,"derive send only");__atomic_store_n(&endpoint_ready,1,__ATOMIC_RELEASE);}
 wait_for(&endpoint_ready,1);exchange(id,0);
 __atomic_fetch_add(&ready,1,__ATOMIC_RELEASE);
 wait_for(&owner_exiting,1);
 for(unsigned i=0;i<20;i++)tick();
 check(raw(156,owner,0)==~0UL,"owner observation query");
 printf("R2B5 progress actor=%u tid=%lu owner=%lu vm=%lu\n",id,tid,owner,raw(141,0,0));
 exchange(id,1);
 if(mode==3) {
  if(id==0) {
   set_stage(1);wait_for(&stage,2);puts("R2B5 endpoint owner exiting");return 0;
  }
  wait_for(&stage,1);
  check(raw(13,endpoint,0)!=0,"peer cannot destroy owner endpoint");
  check(raw(13,public_endpoint,0)!=0,"send only cannot destroy endpoint");
  unsigned long label=0;
  check(ipc(18,endpoint,&label)==1,"peer cannot receive owner endpoint");
  check(raw(11,public_endpoint,0)==0,"send only cannot rederive");
  puts("R2B5 peer endpoint authority denied");set_stage(2);
  for(unsigned i=0;i<10000&&raw(156,creator_tid,0)!=1;i++)tick();
  check(raw(156,creator_tid,0)==1,"endpoint owner physically reaped");
  check(raw(11,endpoint,0)==0,"closed endpoint cannot derive");
  unsigned long fresh=raw(10,0,0);check(fresh!=0&&fresh!=endpoint,"fresh endpoint token");
  check(raw(13,endpoint,0)!=0,"stale endpoint cannot destroy successor");
  unsigned long derived=raw(11,fresh,0);check(derived!=0,"successor survives stale destroy");
  check(raw(13,fresh,0)==0,"authorized successor destroy");
  check(raw(11,fresh,0)==0,"destroyed successor cannot resurrect");
  puts("R2B5 PASS IPC continuity authority closure stale-token successor preserved");return 0;
 }
 if(id==0) {
  unsigned long addr,cap;
  __asm__ volatile("syscall":"=a"(addr),"=d"(cap):"a"(92UL),"D"(4096UL):"rcx","r11","r13","memory");
  check(addr!=~0UL&&addr!=0&&cap!=0,"borrower allocation");
  first=addr;token=cap;*(volatile unsigned long *)first=0x1234abcUL;
  printf("R2B5 allocated token=%lu view=%lx creator=%lu\n",token,first,tid);set_stage(1);
  wait_for(&stage,2);
  check(*(volatile unsigned long *)first==0xfeed456UL,"shared write visibility");
  if(mode==2) {puts("R2B5 forced group with two views");raw(72,owner,9);check(0,"forced kill returned");}
  if(mode==0)check(raw(94,token,0)==0,"creator unmap");
  printf("R2B5 creator retiring mode=%d\n",mode);set_stage(3);return 0;
 }
 wait_for(&stage,1);
 check(raw(13,endpoint,0)!=0,"borrower cannot destroy peer endpoint");
 check(raw(13,public_endpoint,0)!=0,"send only cannot destroy endpoint");
 puts("R2B5 peer endpoint management denied");
 second=raw(93,token,0);check(second!=~0UL&&second!=0,"peer legacy token map");
 check(second!=first,"distinct views in shared VM");
 check(*(volatile unsigned long *)second==0x1234abcUL,"shared read visibility");
 *(volatile unsigned long *)second=0xfeed456UL;
 printf("R2B5 peer mapped token=%lu view=%lx\n",token,second);set_stage(2);
 wait_for(&stage,3);
 for(unsigned i=0;i<10000;i++) {
  if(raw(156,creator_tid,0)==1)break;
  tick();
 }
 check(raw(156,creator_tid,0)==1,"creator physically reaped");
 check(raw(11,endpoint,0)==0,"retired endpoint cannot derive");
 check(*(volatile unsigned long *)second==0xfeed456UL,"peer view after creator retirement");
 check(raw(94,token,0)==0,"last peer unmap");
 check(raw(94,token,0)==~0UL,"duplicate free rejected");
 check(raw(93,token,0)==~0UL,"retired backing token rejected");
 puts("R2B5 PASS legacy-shm peer retained last-view retired stale-token rejected");return 0;
}
int main(int argc,char **argv) {
 check(argc==2,"mode argument");mode=argv[1][0]-'0';owner=raw(33,0,0);
 check(!pthread_key_create(&key,destructor)&&!pthread_setspecific(key,(void *)1),"owner cleanup announcement");
 for(unsigned i=0;i<2;i++){pthread_t t;check(!pthread_create(&t,0,worker,(void *)(uintptr_t)i),"create borrower");}
 wait_for(&ready,2);puts("R2B5 original owner pthread_exit");pthread_exit(0);
}
