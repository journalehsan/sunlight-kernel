/* Freestanding real AF_UNIX connect path; no test-only kernel adapter. */
typedef unsigned long u64;
struct event { unsigned events; u64 data; } __attribute__((packed));
struct address { unsigned short family; char path[30]; };
struct pollfd { int fd; short events, revents; };
static long sc(long n,long a,long b,long c,long d) {
#ifdef NATIVE_ACTOR
    switch(n) {
    case 0:n=42;break; case 1:n=43;break; case 3:n=41;break;
    case 7:n=1006;break; case 41:n=1050;break; case 42:n=1051;break;
    case 43:n=1054;break; case 49:n=1052;break; case 50:n=1053;break;
    case 232:n=1022;break; case 233:n=1021;break; case 291:n=1020;break;
    }
#endif
    register long r10 __asm__("r10")=d;
    __asm__ volatile("syscall":"+a"(n):"D"(a),"S"(b),"d"(c),"r"(r10):"rcx","r11","memory");return n;
}
static int failures;
#define CHECK(c,name) do { if(c) sc(1,1,(long)"unix: PASS " name "\n",sizeof("unix: PASS " name "\n")-1,0); else {failures++;sc(1,1,(long)"unix: FAIL " name "\n",sizeof("unix: FAIL " name "\n")-1,0);} }while(0)
static long ctl(long ep,int op,long fd,struct event *e){return sc(233,ep,op,fd,(long)e);}
static long wait(long ep,struct event *e){return sc(232,ep,(long)e,1,0);}
int main(void) {
 for(int round=0;round<8;round++) {
    struct address addr={1,"/unix-replace"}, missing={1,"/unix-missing"};
    long l=sc(41,1,1|0x800,0,0), c=sc(41,1,1|0x800,0,0), ep=sc(291,0,0,0,0);
    CHECK(l>=0 && c>=0 && ep>=0,"setup");
    struct event e={1,0xfedcba9876543210UL},out;
    CHECK(ctl(ep,1,c,&e)==0,"register-before-connect");
    CHECK(wait(ep,&out)==0,"unconnected-not-ready");
    struct pollfd p={(int)c,5,0};
    CHECK(sc(7,(long)&p,1,0,0)==0 && p.revents==0,"poll-unconnected");
    CHECK(sc(42,c,(long)&missing,sizeof(missing),0)==-111,"refused-connect");
    CHECK(ctl(ep,3,c,&e)==0 && wait(ep,&out)==0,"rejection-preserves-registration");
    CHECK(sc(49,l,(long)&addr,sizeof(addr),0)==0 && sc(50,l,4,0,0)==0,"bind-listen");
    CHECK(sc(42,c,(long)&addr,sizeof(addr),0)==0,"connect-real-replacement");
    p.revents=0;
    CHECK(sc(7,(long)&p,1,0,0)==1 && (p.revents&4),"poll-connected-writable");
    long a=sc(43,l,0,0,0);
    CHECK(a>=0,"accept");
    CHECK(sc(1,a,(long)"A",1,0)==1,"peer-produces-readiness");
    CHECK(wait(ep,&out)==1 && out.data==e.data && (out.events&1),"registration-follows-logical-socket");
    CHECK(sc(42,c,(long)&addr,sizeof(addr),0)==-106,"repeat-connect-EISCONN");
    CHECK(wait(ep,&out)==1 && out.data==e.data,"repeat-rejection-preserves-ready-data");
    e.data=0x8123456789abcdefUL;
    CHECK(ctl(ep,3,c,&e)==0 && wait(ep,&out)==1 && out.data==e.data,"mod-preserves-full-width-data");
    char got=0;
    CHECK(sc(0,c,(long)&got,1,0)==1 && got=='A',"traffic-from-correct-peer");
    CHECK(wait(ep,&out)==0,"drained-no-readiness");
    CHECK(ctl(ep,2,c,0)==0 && ctl(ep,1,c,&e)==0,"del-and-fresh-add-after-transition");
    CHECK(sc(1,a,(long)"B",1,0)==1 && sc(3,a,0,0,0)==0,"buffer-then-peer-close");
    CHECK(wait(ep,&out)==1 && out.data==e.data && (out.events&17)==17,"buffered-data-and-hup");
    CHECK(sc(0,c,(long)&got,1,0)==1 && got=='B' && sc(0,c,(long)&got,1,0)==0,"buffer-before-eof");
    CHECK(sc(3,c,0,0,0)==0,"close-registered");
    long reused=sc(41,1,1|0x800,0,0);
    CHECK(reused==c && wait(ep,&out)==0,"reuse-no-stale-delivery");
    CHECK(ctl(ep,1,reused,&e)==0,"reuse-fresh-registration");
    CHECK(sc(3,reused,0,0,0)==0 && sc(3,l,0,0,0)==0 && sc(3,ep,0,0,0)==0,"cleanup");
 }
 CHECK(failures==0,"complete");return failures!=0;
}
#ifdef NATIVE_ACTOR
__asm__(".global _start\n_start:\nxor %ebp,%ebp\nand $-16,%rsp\ncall main\nmov %eax,%edi\nmov $20,%eax\nsyscall\nud2\n");
#else
__asm__(".global _start\n_start:\nxor %ebp,%ebp\nand $-16,%rsp\ncall main\nmov %eax,%edi\nmov $60,%eax\nsyscall\nud2\n");
#endif
