/* Native public local-fd APIs over AF_UNIX created by existing guest dispatcher. */
#include <poll.h>
#include <sys/epoll.h>
#include <unistd.h>
#include <stdint.h>
static long call(long n,long a,long b,long c) {
    __asm__ volatile("syscall":"+a"(n):"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return n;
}
static int failures;
#define CHECK(c,name) do { if(c) write(1,"unix-public: PASS " name "\n",sizeof("unix-public: PASS " name "\n")-1); else { failures++;write(1,"unix-public: FAIL " name "\n",sizeof("unix-public: FAIL " name "\n")-1);} }while(0)
int main(void) {
 for(int i=0;i<8;i++) {
    struct {unsigned short family;char path[30];} addr={1,"/unix-public"};
    int l=call(1050,1,0x801,0),c=call(1050,1,0x801,0),ep=epoll_create1(0);
    CHECK(l>=0 && c>=0 && ep>=0,"setup");
    struct epoll_event e={.events=EPOLLIN,.data=0xfedcba9876543210ULL+(uint64_t)i},out;
    CHECK(epoll_ctl(ep,EPOLL_CTL_ADD,c,&e)==0 && epoll_wait(ep,&out,1,0)==0,"register-before-connect");
    struct pollfd p={.fd=c,.events=POLLIN|POLLOUT};
    CHECK(poll(&p,1,0)==0,"unconnected-poll");
    CHECK(call(1052,l,(long)&addr,sizeof(addr))==0 && call(1053,l,4,0)==0 && call(1051,c,(long)&addr,sizeof(addr))==0,"real-connect");
    CHECK(poll(&p,1,0)==1 && (p.revents&POLLOUT),"connected-poll");
    int a=call(1054,l,0,0);
    CHECK(a>=0 && write(a,"R",1)==1,"accepted-peer-produces-data");
    CHECK(epoll_wait(ep,&out,1,0)==1 && out.data==e.data && out.events==EPOLLIN,"native-registration-follows-transition");
    CHECK(call(1051,c,(long)&addr,sizeof(addr))==-106 && epoll_wait(ep,&out,1,0)==1 && out.data==e.data,"rejection-preserves-registration");
    e.data=0x8abcdef012345678ULL;
    CHECK(epoll_ctl(ep,EPOLL_CTL_MOD,c,&e)==0 && epoll_wait(ep,&out,1,0)==1 && out.data==e.data,"native-mod-full-width-data");
    char got=0;
    CHECK(read(c,&got,1)==1 && got=='R' && epoll_wait(ep,&out,1,0)==0,"native-read-correct-peer");
    CHECK(epoll_ctl(ep,EPOLL_CTL_DEL,c,0)==0 && epoll_ctl(ep,EPOLL_CTL_ADD,c,&e)==0,"native-del-add");
    CHECK(write(a,"S",1)==1 && close(a)==0,"buffer-and-close-peer");
    CHECK(epoll_wait(ep,&out,1,0)==1 && out.data==e.data && (out.events&(EPOLLIN|EPOLLHUP))==(EPOLLIN|EPOLLHUP),"native-buffered-hup");
    CHECK(read(c,&got,1)==1 && got=='S' && read(c,&got,1)==0,"native-buffer-before-eof");
    CHECK(close(c)==0,"native-close-registered");
    int reuse=call(1050,1,0x801,0);
    CHECK(reuse==c && epoll_wait(ep,&out,1,0)==0 && epoll_ctl(ep,EPOLL_CTL_ADD,reuse,&e)==0,"native-reuse-fresh-interest");
    CHECK(close(reuse)==0 && close(l)==0 && close(ep)==0,"native-cleanup");
 }
 CHECK(failures==0,"complete");return failures!=0;
}
