/* Freestanding AF_UNIX state-validation and alias-coherence probe.
 *
 * Exercises the real production connect path (native dispatcher numbers or
 * Linux syscall numbers) with no test-only kernel adapter. Every operation is
 * nonblocking and synchronous; there are no sleeps. The fixture asserts the
 * intended Linux-compat contract: connect(2) on a socket in LISTEN state must
 * fail with EINVAL and preserve the listener, its pathname, its pending queue
 * and its registrations. It also observes alias (dup) transitions without
 * assuming Linux OFD epoll lifetime.
 */
typedef unsigned long u64;
struct event { unsigned events; u64 data; } __attribute__((packed));
struct address { unsigned short family; char path[30]; };
struct pollfd { int fd; short events, revents; };
static long sc(long n,long a,long b,long c,long d) {
#ifdef NATIVE_ACTOR
    switch(n) {
    case 0:n=42;break; case 1:n=43;break; case 3:n=41;break;
    case 7:n=1006;break; case 32:n=45;break;
    case 41:n=1050;break; case 42:n=1051;break; case 43:n=1054;break;
    case 49:n=1052;break; case 50:n=1053;break; case 72:n=49;break;
    case 232:n=1022;break; case 233:n=1021;break; case 291:n=1020;break;
    }
#endif
    register long r10 __asm__("r10")=d;
    __asm__ volatile("syscall":"+a"(n):"D"(a),"S"(b),"d"(c),"r"(r10):"rcx","r11","memory");return n;
}
static int failures;
#define CHECK(c,name) do { if(c) sc(1,1,(long)"state: PASS " name "\n",sizeof("state: PASS " name "\n")-1,0); else {failures++;sc(1,1,(long)"state: FAIL " name "\n",sizeof("state: FAIL " name "\n")-1,0);} }while(0)
static long ctl(long ep,int op,long fd,struct event *e){return sc(233,ep,op,fd,(long)e);}
static long wait1(long ep,struct event *e){return sc(232,ep,(long)e,1,0);}
static long mk(void){return sc(41,1,1|0x800,0,0);}
static long bindl(long fd,const char *path){struct address a={1,{0}};int i=0;while(path[i]&&i<29){a.path[i]=path[i];i++;}return sc(49,fd,(long)&a,sizeof(a),0);}
static long conn(long fd,const char *path){struct address a={1,{0}};int i=0;while(path[i]&&i<29){a.path[i]=path[i];i++;}return sc(42,fd,(long)&a,sizeof(a),0);}
static void close_if(long fd){if(fd>=0)sc(3,fd,0,0,0);}
static int close_ok(long fd){return fd<0?1:(sc(3,fd,0,0,0)==0);}

/* Section 1: listening source with an empty accept queue. */
static void section_empty(void) {
    long s=mk(), d=mk(), ep=sc(291,0,0,0,0);
    CHECK(s>=0&&d>=0&&ep>=0,"empty-setup");
    CHECK(bindl(s,"/usv-empty-src")==0&&sc(50,s,4,0,0)==0,"empty-source-listen");
    CHECK(bindl(d,"/usv-empty-dst")==0&&sc(50,d,4,0,0)==0,"empty-dest-listen");
    struct event e={1,0x1111222233334444UL}, out;
    CHECK(ctl(ep,1,s,&e)==0,"empty-register-source");
    CHECK(conn(s,"/usv-empty-dst")==-22,"empty-listening-source-EINVAL");
    long c=mk();
    CHECK(c>=0&&conn(c,"/usv-empty-src")==0,"empty-source-still-usable");
    CHECK(wait1(ep,&out)==1&&out.data==e.data&&(out.events&1),"empty-source-registration-preserved");
    long a=sc(43,s,0,0,0);
    CHECK(a>=0,"empty-source-accept");
    CHECK(sc(1,c,(long)"E",1,0)==1,"empty-peer-write");
    char b=0;
    CHECK(sc(0,a,(long)&b,1,0)==1&&b=='E',"empty-routing");
    long unexpected=sc(43,d,0,0,0);
    CHECK(unexpected==-11,"empty-dest-no-unexpected");
    close_if(unexpected);
    CHECK(close_ok(a)&close_ok(c)&close_ok(d)&close_ok(s)&close_ok(ep),"empty-cleanup");
}

/* Section 2: listening source with a pending connection in its queue. */
static void section_pending(void) {
    long s=mk(), d=mk(), ep=sc(291,0,0,0,0);
    CHECK(s>=0&&d>=0&&ep>=0,"pending-setup");
    CHECK(bindl(s,"/usv-pend-src")==0&&sc(50,s,4,0,0)==0,"pending-source-listen");
    CHECK(bindl(d,"/usv-pend-dst")==0&&sc(50,d,4,0,0)==0,"pending-dest-listen");
    long p=mk();
    CHECK(p>=0&&conn(p,"/usv-pend-src")==0,"pending-queue-one");
    struct event e={1,0x2222333344445555UL}, out;
    CHECK(ctl(ep,1,s,&e)==0&&wait1(ep,&out)==1&&out.data==e.data,"pending-ready-before");
    CHECK(conn(s,"/usv-pend-dst")==-22,"pending-listening-connect-EINVAL");
    CHECK(wait1(ep,&out)==1&&out.data==e.data,"pending-registration-preserved");
    long a=sc(43,s,0,0,0);
    CHECK(a>=0,"pending-accept-preserved");
    CHECK(sc(1,p,(long)"P",1,0)==1,"pending-peer-write");
    char b=0;
    CHECK(sc(0,a,(long)&b,1,0)==1&&b=='P',"pending-routing");
    long unexpected=sc(43,d,0,0,0);
    CHECK(unexpected==-11,"pending-dest-no-unexpected");
    close_if(unexpected);
    CHECK(close_ok(a)&close_ok(p)&close_ok(d)&close_ok(s)&close_ok(ep),"pending-cleanup");
}

/* Section 3: a full destination queue must not change the source verdict. */
static void section_full(void) {
    long s=mk(), d=mk();
    CHECK(s>=0&&d>=0,"full-setup");
    CHECK(bindl(s,"/usv-full-src")==0&&sc(50,s,4,0,0)==0,"full-source-listen");
    CHECK(bindl(d,"/usv-full-dst")==0&&sc(50,d,128,0,0)==0,"full-dest-listen");
    long pending[128];
    int filled=1;
    for(int i=0;i<128;i++){
        pending[i]=mk();
        if(pending[i]<0||conn(pending[i],"/usv-full-dst")!=0){filled=0;break;}
    }
    CHECK(filled,"full-dest-fill");
    struct event e={1,0x3333444455556666UL}, out;
    long ep=sc(291,0,0,0,0);
    CHECK(ep>=0&&ctl(ep,1,s,&e)==0,"full-register-source");
    CHECK(conn(s,"/usv-full-dst")==-22,"full-listening-source-EINVAL-precedes-EAGAIN");
    long c=mk();
    CHECK(c>=0&&conn(c,"/usv-full-dst")==-11,"full-dest-still-full");
    long c2=mk();
    CHECK(c2>=0&&conn(c2,"/usv-full-src")==0,"full-source-still-usable");
    CHECK(wait1(ep,&out)==1&&out.data==e.data,"full-source-registration-preserved");
    long a=sc(43,s,0,0,0);
    CHECK(a>=0,"full-source-accept");
    CHECK(sc(1,c2,(long)"F",1,0)==1,"full-peer-write");
    char b=0;
    CHECK(sc(0,a,(long)&b,1,0)==1&&b=='F',"full-routing");
    CHECK(close_ok(a)&close_ok(c2)&close_ok(c)&close_ok(s)&close_ok(d)&close_ok(ep),"full-cleanup");
    for(int i=0;i<128;i++) close_if(pending[i]);
}

/* Section 4: invalid and bound-but-not-listening destinations. */
static void section_invalid_dest(void) {
    long s=mk(), bnd=mk();
    CHECK(s>=0&&bnd>=0,"inv-setup");
    CHECK(bindl(s,"/usv-inv-src")==0&&sc(50,s,4,0,0)==0,"inv-source-listen");
    CHECK(bindl(bnd,"/usv-inv-bound")==0,"inv-bound-not-listening");
    CHECK(conn(s,"/usv-inv-missing")==-22,"inv-missing-dest-EINVAL");
    CHECK(conn(s,"/usv-inv-bound")==-22,"inv-bound-dest-EINVAL");
    long c=mk();
    CHECK(c>=0&&conn(c,"/usv-inv-src")==0,"inv-source-still-usable");
    long a=sc(43,s,0,0,0);
    CHECK(a>=0,"inv-source-accept");
    long n=mk();
    CHECK(n>=0&&conn(n,"/usv-inv-bound")==-111,"inv-nonlistener-refused");
    CHECK(close_ok(a)&close_ok(c)&close_ok(n)&close_ok(bnd)&close_ok(s),"inv-cleanup");
}

/* Section 5: self-targeting while listening. */
static void section_self(void) {
    long s=mk();
    CHECK(s>=0,"self-setup");
    CHECK(bindl(s,"/usv-self")==0&&sc(50,s,4,0,0)==0,"self-listen");
    CHECK(conn(s,"/usv-self")==-22,"self-listening-EINVAL");
    long c=mk();
    CHECK(c>=0&&conn(c,"/usv-self")==0,"self-source-still-usable");
    long a=sc(43,s,0,0,0);
    CHECK(a>=0,"self-accept");
    CHECK(close_ok(a)&close_ok(c)&close_ok(s),"self-cleanup");
}

/* Section 6: dup alias, connect through one descriptor. */
static void section_alias_connect(int round) {
    long a=mk();
    long b=sc(32,a,0,0,0);
    long d=mk();
    CHECK(a>=0&&b>=0&&d>=0,"alias-setup");
    CHECK(bindl(d,"/usv-alias-dst")==0&&sc(50,d,4,0,0)==0,"alias-dest-listen");
    long ea=sc(291,0,0,0,0), eb=sc(291,0,0,0,0);
    struct event eva={1,0xaaaa000000000001UL+(u64)round}, evb={1,0xbbbb000000000002UL+(u64)round}, out;
    CHECK(ea>=0&&eb>=0&&ctl(ea,1,a,&eva)==0&&ctl(eb,1,b,&evb)==0,"alias-register-both");
    CHECK(wait1(ea,&out)==0&&wait1(eb,&out)==0,"alias-unconnected-not-ready");
    CHECK(conn(a,"/usv-alias-dst")==0,"alias-connect-one");
    long peer=sc(43,d,0,0,0);
    CHECK(peer>=0,"alias-dest-accept");
    CHECK(sc(1,peer,(long)"X",1,0)==1,"alias-peer-write");
    CHECK(wait1(ea,&out)==1&&out.data==eva.data&&(out.events&1),"alias-connected-registration-data");
    char got=0;
    CHECK(sc(0,a,(long)&got,1,0)==1&&got=='X',"alias-connected-routing");
    CHECK(wait1(eb,&out)==0,"alias-other-unconnected-no-readiness");
    CHECK(conn(b,"/usv-alias-dst")==0,"alias-other-independent-connect");
    long peer2=sc(43,d,0,0,0);
    CHECK(peer2>=0,"alias-dest-accept-2");
    CHECK(sc(1,peer2,(long)"Y",1,0)==1,"alias-peer2-write");
    CHECK(wait1(eb,&out)==1&&out.data==evb.data&&(out.events&1),"alias-other-connected-registration-data");
    got=0;
    CHECK(sc(0,b,(long)&got,1,0)==1&&got=='Y',"alias-other-routing");
    CHECK(close_ok(a)&close_ok(b),"alias-close-a-then-b");
    CHECK(close_ok(peer)&close_ok(peer2)&close_ok(d)&close_ok(ea)&close_ok(eb),"alias-cleanup");
}

/* Section 7: dup alias of a listening source, prohibited transition. */
static void section_alias_listener(void) {
    long a=mk();
    long b=sc(32,a,0,0,0);
    long d=mk();
    CHECK(a>=0&&b>=0&&d>=0,"alias-ls-setup");
    CHECK(bindl(a,"/usv-alias-ls")==0&&sc(50,a,4,0,0)==0,"alias-ls-listen");
    CHECK(bindl(d,"/usv-alias-ld")==0&&sc(50,d,4,0,0)==0,"alias-ld-listen");
    long p=mk();
    CHECK(p>=0&&conn(p,"/usv-alias-ls")==0,"alias-ls-queue-one");
    long ea=sc(291,0,0,0,0), eb=sc(291,0,0,0,0);
    struct event eva={1,0xcccc000000000003UL}, evb={1,0xdddd000000000004UL}, out;
    CHECK(ea>=0&&eb>=0&&ctl(ea,1,a,&eva)==0&&ctl(eb,1,b,&evb)==0,"alias-ls-register-both");
    CHECK(wait1(ea,&out)==1&&out.data==eva.data,"alias-ls-a-ready");
    CHECK(wait1(eb,&out)==1&&out.data==evb.data,"alias-ls-b-ready");
    CHECK(conn(a,"/usv-alias-ld")==-22,"alias-ls-connect-a-EINVAL");
    CHECK(conn(b,"/usv-alias-ld")==-22,"alias-ls-connect-b-EINVAL");
    CHECK(wait1(ea,&out)==1&&out.data==eva.data,"alias-ls-a-registration-preserved");
    CHECK(wait1(eb,&out)==1&&out.data==evb.data,"alias-ls-b-registration-preserved");
    long unexpected=sc(43,d,0,0,0);
    CHECK(unexpected==-11,"alias-ls-dest-no-unexpected");
    close_if(unexpected);
    long acc=sc(43,a,0,0,0);
    CHECK(acc>=0,"alias-ls-accept-preserved");
    CHECK(sc(1,p,(long)"Z",1,0)==1,"alias-ls-peer-write");
    char got=0;
    CHECK(sc(0,acc,(long)&got,1,0)==1&&got=='Z',"alias-ls-routing");
    CHECK(close_ok(b)&close_ok(a),"alias-ls-close-b-then-a");
    CHECK(close_ok(acc)&close_ok(p)&close_ok(d)&close_ok(ea)&close_ok(eb),"alias-ls-cleanup");
}

/* Section 8: close one alias, reuse its descriptor number, keep the other. */
static void section_alias_reuse(void) {
    long a=mk();
    long b=sc(32,a,0,0,0);
    long d=mk();
    CHECK(a>=0&&b>=0&&d>=0,"reuse-setup");
    CHECK(bindl(d,"/usv-reuse-dst")==0&&sc(50,d,4,0,0)==0,"reuse-dest-listen");
    CHECK(sc(3,a,0,0,0)==0,"reuse-close-a");
    long a2=mk();
    CHECK(a2==a,"reuse-fd-number-reused");
    CHECK(conn(b,"/usv-reuse-dst")==0,"reuse-b-still-usable");
    CHECK(conn(a2,"/usv-reuse-dst")==0,"reuse-new-socket-connect");
    long peer=sc(43,d,0,0,0);
    CHECK(peer>=0,"reuse-accept");
    long peer2=sc(43,d,0,0,0);
    CHECK(peer2>=0,"reuse-accept-2");
    CHECK(sc(1,peer,(long)"R",1,0)==1,"reuse-peer-write");
    char got=0;
    CHECK(sc(0,b,(long)&got,1,0)==1&&got=='R',"reuse-b-routing");
    CHECK(close_ok(a2)&close_ok(b)&close_ok(peer)&close_ok(peer2)&close_ok(d),"reuse-cleanup");
}

/* Section 9: repeated rejected lifecycles must not consume resources. */
static void section_repeat(int rounds) {
    for(int r=0;r<rounds;r++){
        long s=mk(), d=mk();
        CHECK(s>=0&&d>=0,"repeat-setup");
        CHECK(bindl(s,"/usv-rep-src")==0&&sc(50,s,4,0,0)==0,"repeat-source-listen");
        CHECK(bindl(d,"/usv-rep-dst")==0&&sc(50,d,4,0,0)==0,"repeat-dest-listen");
        CHECK(conn(s,"/usv-rep-dst")==-22,"repeat-EINVAL");
        long c=mk();
        CHECK(c>=0&&conn(c,"/usv-rep-src")==0,"repeat-source-usable");
        long a=sc(43,s,0,0,0);
        CHECK(a>=0,"repeat-accept");
        CHECK(close_ok(a)&close_ok(c)&close_ok(d)&close_ok(s),"repeat-cleanup");
    }
}

int main(void) {
    section_empty();
    section_pending();
    section_full();
    section_invalid_dest();
    section_self();
    for(int r=0;r<4;r++) section_alias_connect(r);
    section_alias_listener();
    section_alias_reuse();
    section_repeat(4);
    CHECK(failures==0,"complete");
    return failures!=0;
}
#ifdef NATIVE_ACTOR
__asm__(".global _start\n_start:\nxor %ebp,%ebp\nand $-16,%rsp\ncall main\nmov %eax,%edi\nmov $20,%eax\nsyscall\nud2\n");
#else
__asm__(".global _start\n_start:\nxor %ebp,%ebp\nand $-16,%rsp\ncall main\nmov %eax,%edi\nmov $60,%eax\nsyscall\nud2\n");
#endif
