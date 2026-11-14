/* Raw Linux-personality actor. No host libc, native libc, or alternate dup3.
 * OSABI=3 is applied by run_dup_runtime.py before guest execution. */
typedef unsigned long u64;
struct event { unsigned events; u64 data; } __attribute__((packed));
static long sc(long n, long a, long b, long c, long d) {
    register long r10 __asm__("r10") = d;
    __asm__ volatile("syscall" : "+a"(n) : "D"(a), "S"(b), "d"(c), "r"(r10)
                     : "rcx", "r11", "memory");
    return n;
}
static int failures;
#define CHECK(c, name) do { \
    if (c) sc(1, 1, (long)"linux-entry: PASS " name "\n", sizeof("linux-entry: PASS " name "\n")-1, 0); \
    else { failures++; sc(1, 1, (long)"linux-entry: FAIL " name "\n", sizeof("linux-entry: FAIL " name "\n")-1, 0); } \
} while (0)
int main(void) {
    int a[2], b[2]; char x = 'x', y = 'y', got = 0;
    CHECK(sc(22,(long)a,0,0,0)==0 && sc(22,(long)b,0,0,0)==0, "pipe-setup");
    long ep = sc(291,0,0,0,0);
    CHECK(ep >= 0, "epoll-setup");
    struct event e = {1,0xfedcba9876543210UL}, out;
    CHECK(sc(1,a[1],(long)&x,1,0)==1 && sc(1,b[1],(long)&y,1,0)==1, "ready-before-replacement");
    CHECK(sc(233,ep,1,a[0],(long)&e)==0, "register-target");
    CHECK(sc(232,ep,(long)&out,1,0)==1 && out.data==e.data, "full-width-data-before");
    CHECK(sc(292,-1,a[0],0,0)==-9, "invalid-source-EBADF");
    CHECK(sc(292,b[0],300,0,0)==-9, "invalid-target-EBADF");
    CHECK(sc(292,a[0],a[0],0,0)==-22, "same-fd-EINVAL");
    CHECK(sc(292,b[0],a[0],1,0)==-22, "unsupported-flags-EINVAL");
    CHECK(sc(232,ep,(long)&out,1,0)==1 && out.data==e.data, "rejections-preserve-registration");
    CHECK(sc(0,a[0],(long)&got,1,0)==1 && got=='x', "rejections-preserve-original-object");
    CHECK(sc(292,b[0],a[0],0x80000,0)==a[0], "dup3-292-O_CLOEXEC");
    CHECK(sc(72,a[0],1,0,0)==1 && sc(72,b[0],1,0,0)==0, "cloexec-target-only");
    CHECK(sc(232,ep,(long)&out,1,0)==0, "replacement-invalidates-registration");
    CHECK(sc(0,a[0],(long)&got,1,0)==1 && got=='y', "replacement-routes-source");
    CHECK(sc(292,b[0],a[0],0,0)==a[0] && sc(72,a[0],1,0,0)==0, "zero-flags-clears-cloexec");
    for (int i=0;i<8;i++) {
        CHECK(sc(233,ep,1,a[0],(long)&e)==0, "repeat-fresh-registration");
        CHECK(sc(292,b[0],a[0],0,0)==a[0], "repeat-replacement");
        CHECK(sc(232,ep,(long)&out,1,0)==0, "repeat-invalidation");
    }
    int old = a[0];
    CHECK(sc(3,a[0],0,0,0)==0 && sc(3,a[1],0,0,0)==0, "close-target");
    CHECK(sc(22,(long)a,0,0,0)==0 && a[0]==old, "descriptor-reuse");
    CHECK(sc(232,ep,(long)&out,1,0)==0, "reuse-has-no-old-registration");
    CHECK(sc(3,a[0],0,0,0)==0 && sc(3,a[1],0,0,0)==0 &&
          sc(3,b[0],0,0,0)==0 && sc(3,b[1],0,0,0)==0 && sc(3,ep,0,0,0)==0, "cleanup");
    CHECK(failures==0, "complete");
    return failures != 0;
}
__asm__(".global _start\n_start:\nxor %ebp,%ebp\nand $-16,%rsp\ncall main\nmov %eax,%edi\nmov $60,%eax\nsyscall\nud2\n");
