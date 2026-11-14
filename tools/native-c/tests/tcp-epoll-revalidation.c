/* Negative protocol challenges complement real readiness/pressure fixtures.
 * Deliberately stale observations must never become application events. */
#include <sys/epoll.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <unistd.h>
#include <errno.h>
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <stdatomic.h>
struct observation { int32_t fd; uint32_t events; uint64_t token, identity; uint32_t ready, reserved; };
_Static_assert(sizeof(struct observation)==32 && _Alignof(struct observation)==8,"bridge ABI");
static long bridge(long a,long b,long c,long d,long e,long f) {
    register long r10 __asm__("r10")=d, r8 __asm__("r8")=e, r9 __asm__("r9")=f;
    long n=177; __asm__ volatile("syscall":"+a"(n):"D"(a),"S"(b),"d"(c),"r"(r10),"r"(r8),"r"(r9):"rcx","r11","memory"); return n;
}
static long now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return t.tv_sec*1000+t.tv_nsec/1000000; }
static void delay(void) { struct timespec t={0,10000000}; nanosleep(&t,0); }
extern void *sl_std_spawn(void *(*entry)(void *),void *,size_t);
extern void sl_std_join(void *);
struct worker { int ep,fd; atomic_int error; };
static void *mods(void *arg) {
    struct worker *w=arg;
    for(int i=0;i<20;i++) { struct epoll_event e={.events=0,.data=0xabcdef0000000000ULL+(unsigned)i}; delay(); if(epoll_ctl(w->ep,EPOLL_CTL_MOD,w->fd,&e)) atomic_store(&w->error,1); }
    return 0;
}
#define CHECK(x) do { if(!(x)) { printf("epoll-revalidation: FAIL line=%d errno=%d\n",__LINE__,errno); return 1; } } while(0)
int main(void) {
    int ep=epoll_create1(0), fd=socket(AF_INET,SOCK_STREAM,0);
    struct epoll_event e={.events=EPOLLIN,.data=0x123456789abcdef0ULL},out;
    struct observation snapshot[64],old;
    CHECK(ep>=0 && fd>=0);
    CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,fd,0)==-1 && errno==EFAULT);
    CHECK(epoll_wait(ep,0,1,0)==-1 && errno==EFAULT);
    CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,fd,&e)==0);
    struct sockaddr_in addr; memset(&addr,0,sizeof addr); addr.sin_family=AF_INET; addr.sin_addr.s_addr=htonl(INADDR_LOOPBACK); addr.sin_port=htons(4399);
    long end=now()+5000; int bound;
    do { bound=bind(fd,(struct sockaddr*)&addr,sizeof addr); if(bound) delay(); } while(bound && now()<end);
    CHECK(!bound && !listen(fd,1) && !epoll_wait(ep,&out,1,0));
    CHECK(bridge(ep,(long)snapshot,64,0,0,0)==1); old=snapshot[0]; old.ready=EPOLLIN;
    CHECK(old.identity!=UINT64_MAX);
    e.data=0x8877665544332211ULL; CHECK(!epoll_ctl(ep,EPOLL_CTL_MOD,fd,&e));
    CHECK(bridge(ep,(long)&old,1,(long)&out,1,1)==0); /* MOD invalidated token */
    CHECK(bridge(ep,(long)snapshot,64,0,0,0)==1); old=snapshot[0]; old.ready=EPOLLIN;
    struct observation wrong=old; wrong.identity^=1;
    CHECK(bridge(ep,(long)&wrong,1,(long)&out,1,1)==0);
    wrong=old; wrong.reserved=1; CHECK(bridge(ep,(long)&wrong,1,(long)&out,1,1)==-EINVAL);
    wrong=old; wrong.ready=0x80000000; CHECK(bridge(ep,(long)&wrong,1,(long)&out,1,1)==-EINVAL);
    CHECK(bridge(ep,(long)snapshot,65,(long)&out,1,1)==-EINVAL);
    CHECK(bridge(ep,0,64,0,0,0)==-EFAULT);
    CHECK(bridge(ep,(long)snapshot,64,0,0,256)==-EINVAL);
    CHECK(bridge(ep,EPOLL_CTL_MOD,fd,(long)&e,old.identity,2)==-EBADF); /* stale generation */
    CHECK(!epoll_ctl(ep,EPOLL_CTL_DEL,fd,0));
    CHECK(bridge(ep,(long)&old,1,(long)&out,1,1)==0);
    CHECK(!epoll_ctl(ep,EPOLL_CTL_ADD,fd,&e));
    CHECK(bridge(ep,(long)&old,1,(long)&out,1,1)==0); /* DEL/re-ADD */
    struct worker w={.ep=ep,.fd=fd,.error=0}; void *thread=sl_std_spawn(mods,&w,65536); CHECK(thread);
    long started=now(); CHECK(epoll_wait(ep,&out,1,95)==0 && now()-started>=90 && now()-started<1000);
    sl_std_join(thread); CHECK(!atomic_load(&w.error));
    CHECK(!close(fd) && !epoll_wait(ep,&out,1,0));
    fd=socket(AF_INET,SOCK_STREAM,0); CHECK(fd==old.fd);
    CHECK(!epoll_ctl(ep,EPOLL_CTL_ADD,fd,&e));
    CHECK(bridge(ep,(long)&old,1,(long)&out,1,1)==0); /* close/reuse */
    CHECK(!close(fd) && !close(ep));
    ep=epoll_create1(0); int sockets[65]; e.events=0;
    for(int i=0;i<65;i++) {
        sockets[i]=socket(AF_INET,SOCK_STREAM,0); CHECK(sockets[i]>=0);
        int rc=epoll_ctl(ep,EPOLL_CTL_ADD,sockets[i],&e);
        if(i<64) CHECK(rc==0); else CHECK(rc==-1 && errno==EMFILE);
    }
    CHECK(epoll_wait(ep,&out,2147483647,0)==0);
    for(int i=0;i<65;i++) CHECK(!close(sockets[i]));
    CHECK(!close(ep));
    puts("epoll-revalidation: PASS stale MOD DEL reADD identity generation close-reuse input-validation concurrent-deadline capacity64 maxevents");
    return 0;
}
