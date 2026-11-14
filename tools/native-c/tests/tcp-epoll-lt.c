/* Persistent native LT registrations, capacity, fairness and concurrent ctl.
 * All TCP traffic traverses the production net_server loopback path. */
#include <sys/epoll.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <sunlight_net.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <stddef.h>
#include <stdint.h>
#include <time.h>
#include <stdatomic.h>
#define N 40
_Static_assert(sizeof(struct epoll_event) == 16, "native size");
_Static_assert(_Alignof(struct epoll_event) == 8, "native alignment");
_Static_assert(offsetof(struct epoll_event, data) == 8, "native data offset");
static const uint64_t tag = 0xfedcba9800000000ULL;
static long now(void) { struct timespec t; if (clock_gettime(CLOCK_MONOTONIC, &t)) return -1; return t.tv_sec*1000L+t.tv_nsec/1000000; }
static void delay(long ms) { struct timespec t={ms/1000,(ms%1000)*1000000}; nanosleep(&t,0); }
extern void *sl_std_spawn(void *(*entry)(void *), void *arg, size_t stack);
extern void sl_std_join(void *handle);
struct mutation { int ep, fd, writer; atomic_int error; };
static void *mutate(void *arg) {
    struct mutation *m=arg;
    delay(160);
    struct epoll_event e={.events=0,.data=0x1122334455667788ULL};
    if (epoll_ctl(m->ep,EPOLL_CTL_MOD,m->fd,&e) || epoll_ctl(m->ep,EPOLL_CTL_DEL,m->fd,0) || write(m->writer,"W",1)!=1) atomic_store(&m->error,1);
    return 0;
}
#define CHECK(x) do { if (!(x)) { printf("tcp-epoll-lt: FAIL line=%d errno=%d\n",__LINE__,errno); return 1; } } while(0)
int main(void) {
    int listeners[N], clients[N], ep=epoll_create1(0), other=epoll_create1(0), pair[2];
    struct epoll_event e={.events=EPOLLIN,.data=tag}, out;
    CHECK(ep>=0 && other>=0 && socketpair(AF_UNIX,SOCK_STREAM,0,pair)==0);
    CHECK(epoll_wait(ep,&out,0,0)==-1 && errno==EINVAL);
    CHECK(epoll_wait(-1,&out,1,0)==-1 && errno==EBADF);
    CHECK(epoll_ctl(ep,99,pair[0],&e)==-1 && errno==EINVAL);
    CHECK(epoll_ctl(ep,EPOLL_CTL_MOD,pair[0],&e)==-1 && errno==ENOENT);
    CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,pair[0],&e)==0);
    CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,pair[0],&e)==-1 && errno==EEXIST);
    for(int i=0;i<N;i++) {
        clients[i]=-1;
        struct sockaddr_in addr; memset(&addr,0,sizeof addr); addr.sin_family=AF_INET; addr.sin_port=htons((unsigned short)(4200+i)); addr.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
        listeners[i]=socket(AF_INET,SOCK_STREAM,0); CHECK(listeners[i]>=0);
        e.events=EPOLLIN; e.data=tag+(unsigned)i+1;
        CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,listeners[i],&e)==0);
        if(i==0) CHECK(epoll_wait(ep,&out,1,0)==0); /* no placeholder ERR */
        long end=now()+5000; int bound;
        do { bound=bind(listeners[i],(struct sockaddr*)&addr,sizeof addr); if(bound) delay(20); } while(bound && now()<end);
        CHECK(bound==0 && listen(listeners[i],1)==0);
        e.events=EPOLLIN|EPOLLET; e.data=tag+(unsigned)i+1;
        CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,listeners[i],&e)==-1 && errno==EINVAL);
        e.events=EPOLLIN|EPOLLONESHOT;
        CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,listeners[i],&e)==-1 && errno==EINVAL);
        e.events=EPOLLIN;
        if(i==0 || i>=N-7) {
            clients[i]=socket(AF_INET,SOCK_STREAM,0); CHECK(clients[i]>=0);
            CHECK(connect(clients[i],(struct sockaddr*)&addr,sizeof addr)==0);
        }
    }
    CHECK(write(pair[1],"L",1)==1);
    uint64_t seen=0;
    for(int i=0;i<9;i++) {
        CHECK(epoll_wait(ep,&out,1,1000)==1 && out.events==EPOLLIN);
        CHECK(out.data>=tag && out.data<=tag+N);
        seen |= 1ULL<<(out.data-tag);
    }
    CHECK(seen==(3ULL | (((1ULL<<7)-1)<<(N-6))));
    printf("tcp-epoll-lt: capacity=40 net_batch=32 registered=41 ready=9 maxevents=1 fairness=PASS\n");
    for(int i=1;i<N;i++) { CHECK(close(listeners[i])==0); if(clients[i]>=0) CHECK(close(clients[i])==0); }
    CHECK(epoll_ctl(ep,EPOLL_CTL_DEL,pair[0],0)==0);
    CHECK(epoll_wait(ep,&out,1,0)==1 && out.data==tag+1);
    CHECK(epoll_ctl(other,EPOLL_CTL_ADD,listeners[0],&e)==0);
    CHECK(epoll_wait(other,&out,1,0)==1);
    CHECK(epoll_wait(other,&out,1,0)==1); /* listener LT */
    int stream=accept(listeners[0],0,0); CHECK(stream>=0);
    CHECK(epoll_wait(ep,&out,1,0)==0 && epoll_wait(other,&out,1,0)==0);
    CHECK(close(listeners[0])==0);
    CHECK(epoll_ctl(ep,EPOLL_CTL_DEL,stream,0)==-1 && errno==ENOENT);
    e.events=EPOLLIN; e.data=tag+99; CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,stream,&e)==0);
    CHECK(epoll_ctl(other,EPOLL_CTL_ADD,stream,&e)==0);
    CHECK(send(clients[0],"ABC",3,0)==3);
    CHECK(epoll_wait(ep,&out,1,1000)==1 && out.events==EPOLLIN && out.data==tag+99);
    CHECK(epoll_wait(other,&out,1,0)==1 && out.data==tag+99);
    CHECK(epoll_wait(ep,&out,1,0)==1);
    char bytes[4]; CHECK(recv(stream,bytes,1,0)==1 && bytes[0]=='A');
    CHECK(epoll_wait(ep,&out,1,0)==1);
    CHECK(recv(stream,bytes,2,0)==2 && !memcmp(bytes,"BC",2));
    CHECK(epoll_wait(ep,&out,1,0)==0);
    long started=now(); CHECK(epoll_wait(ep,&out,1,110)==0 && now()-started>=100 && now()-started<2000);
    e.events=EPOLLOUT; e.data=0x8877665544332211ULL;
    CHECK(epoll_ctl(ep,EPOLL_CTL_MOD,stream,&e)==0);
    CHECK(epoll_wait(ep,&out,1,0)==1 && out.events==EPOLLOUT && out.data==e.data);
    e.events=EPOLLIN; CHECK(epoll_ctl(ep,EPOLL_CTL_MOD,stream,&e)==0);
    CHECK(recv(pair[0],bytes,1,0)==1);
    e.data=tag+77; CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,pair[0],&e)==0);
    struct mutation m={.ep=ep,.fd=stream,.writer=pair[1],.error=0};
    void *thread=sl_std_spawn(mutate,&m,65536); CHECK(thread);
    started=now(); CHECK(epoll_wait(ep,&out,1,-1)==1 && out.data==tag+77 && now()-started>=120);
    sl_std_join(thread); CHECK(atomic_load(&m.error)==0);
    CHECK(epoll_ctl(ep,EPOLL_CTL_DEL,pair[0],0)==0);
    CHECK(epoll_wait(ep,&out,1,0)==0); /* concurrent DEL survived revalidation */
    e.events=EPOLLIN; e.data=tag+88; CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,stream,&e)==0);
    CHECK(close(clients[0])==0);
    CHECK(epoll_wait(ep,&out,1,2000)==1 && out.events==EPOLLIN && out.data==e.data);
    CHECK(recv(stream,bytes,1,0)==0 && epoll_wait(ep,&out,1,0)==1);
    e.events=0; CHECK(epoll_ctl(ep,EPOLL_CTL_MOD,stream,&e)==0);
    CHECK(epoll_wait(ep,&out,1,0)==0); /* EOF does not invent HUP */
    int old=stream; CHECK(close(stream)==0);
    int replacements[128], used=0;
    do { CHECK(used<128); stream=socket(AF_INET,SOCK_STREAM,0); CHECK(stream>=0); replacements[used++]=stream; } while(stream<old);
    CHECK(stream==old);
    CHECK(epoll_wait(ep,&out,1,0)==0 && epoll_wait(other,&out,1,0)==0);
    for(int i=0;i<used;i++) CHECK(close(replacements[i])==0);
    CHECK(close(pair[0])==0 && close(pair[1])==0 && close(ep)==0 && close(other)==0);
    long live=-1, end=now()+5000;
    do { live=sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_SOCKET_LIVE); if(live) delay(20); } while(live && now()<end);
    CHECK(live==0);
    printf("tcp-epoll-lt: PASS ABI ctl listener receive drain MOD DEL multi EOF reuse timeout concurrent socket_live=%ld\n",live);
    return 0;
}
