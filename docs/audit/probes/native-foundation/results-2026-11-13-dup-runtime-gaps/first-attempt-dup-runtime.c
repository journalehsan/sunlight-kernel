/* Separate focused fixtures, not standard-suite rows. Reuse the raw fixture's
 * assertions and worker protocol while substituting only the call under test.
 * Linux handler mode calls dispatcher 1031, the exact sys_linux_dup3 entry
 * reached by Linux-personality syscall 292. It is not a Linux TCP libc test.
 */
#ifdef WRAPPER_TEST
#define REPLACEMENT_ADAPTER coverage_replace
#else
#define REPLACEMENT_ADAPTER coverage_linux_replace
#endif
#define main replacement_base_main
#include "dup2-replacement.c"
#undef main

#ifdef WRAPPER_TEST
extern long coverage_replace(int, int);
extern unsigned long coverage_bridge_id(int);
extern unsigned long coverage_generation(int);
#else
long coverage_linux_replace(int old, int target) {
    long r = call3(1031, old, target, 0);
    /* Base fixture expects native BadFd and dup2 same-fd success. Execute
     * and check dup3's actual same-fd rejection here; normalize only the
     * base fixture's result. Dedicated checks below keep raw errno visible. */
    if (old == target) {
        check(r == -22, "linux-base-same-fd-EINVAL");
        return r == -22 ? target : -999;
    }
    return r == -9 ? -5 : r;
}
#endif

static void attached_rejections(void) {
    int l = -1, c = -1, s = -1;
    int ep = epoll_create1(0);
    if (!check(ep >= 0 && connect_pair(4321, &l, &c, &s),
               "attached-rejection-setup")) return;
    struct epoll_event e = { .events = EPOLLIN, .data = TAG + 0x100 };
    struct epoll_event out;
    check(epoll_ctl(ep, EPOLL_CTL_ADD, s, &e) == 0, "attached-registration");
#ifdef WRAPPER_TEST
    unsigned long id = coverage_bridge_id(s), generation = coverage_generation(s);
    check(id != 0 && generation != 0, "real-C-socket-visible-in-Rust-bridge");
#endif
    long released = sunlight_net_diagnostic(1);
    for (int i = 0; i < 8; ++i) {
        char sent = (char)('a' + i), got = 0;
        check(write(c, &sent, 1) == 1, "rejection-original-peer-send");
        check(epoll_wait(ep, &out, 1, 2000) == 1 && out.data == e.data,
              "rejection-readiness-before");
#ifdef WRAPPER_TEST
        check(coverage_replace(-1, s) == -5, "wrapper-invalid-source-EBADF");
        check(coverage_replace(s, 300) == -5, "wrapper-invalid-target-EBADF");
        check(coverage_replace(s, s) == s, "wrapper-attached-same-fd-noop");
        check(coverage_bridge_id(s) == id && coverage_generation(s) == generation,
              "restoration-original-id-and-generation");
#else
        check(call3(1031, -1, s, 0) == -9, "linux-invalid-source-EBADF");
        check(call3(1031, s, 300, 0) == -9, "linux-invalid-target-EBADF");
        check(call3(1031, s, s, 0) == -22, "linux-same-fd-EINVAL");
        check(call3(1031, c, s, 1) == -22, "linux-unsupported-flags-EINVAL");
        check(call3(1031, c, s, 0x80001) == -22, "linux-mixed-flags-EINVAL");
#endif
        check(epoll_wait(ep, &out, 1, 0) == 1 && out.data == e.data,
              "rejection-registration-survives");
        check(read(s, &got, 1) == 1 && got == sent,
              "rejection-routes-original-endpoint");
        check(write(s, &sent, 1) == 1 && read(c, &got, 1) == 1 && got == sent,
              "rejection-target-remains-bidirectional");
    }
    check(sunlight_net_diagnostic(1) == released, "rejections-release-no-attachments");
#ifdef WRAPPER_TEST
    int p[2];
    if (check(make_pipe(p, "Z"), "release-pipe-created")) {
        check(coverage_replace(p[0], s) == s, "attached-success-replacement");
        check(coverage_bridge_id(s) == 0, "success-detaches-target-bridge");
        check(sunlight_net_diagnostic(1) == released + 1,
              "success-releases-displaced-attachment-once");
        char got = 0;
        check(read(s, &got, 1) == 1 && got == 'Z', "success-local-routing");
        check(close(s) == 0, "success-target-close");
        check(sunlight_net_diagnostic(1) == released + 1,
              "target-close-does-not-release-attachment-twice");
        s = -1;
        close(p[0]); close(p[1]);
    }
#else
    int p[2], q[2];
    if (check(make_pipe(p, "P") && make_pipe(q, "Q"), "cloexec-pipes-created")) {
        check(call3(1031, p[0], q[0], 0x80000) == q[0], "linux-O_CLOEXEC-accepted");
        check(call3(49, q[0], 1, 0) == 1 && call3(49, p[0], 1, 0) == 0,
              "linux-cloexec-target-only");
        check(call3(1031, p[0], q[0], 0) == q[0] && call3(49, q[0], 1, 0) == 0,
              "linux-zero-flags-clears-cloexec");
        close(p[0]); close(p[1]); close(q[0]); close(q[1]);
    }
#endif
    if (s >= 0) close(s);
    close(c); close(l); close(ep);
}

int main(void) {
    int base = replacement_base_main();
    check(base == 0, "base-all-assertions-pass");
    attached_rejections();
    /* Reuse + cleanup: allocation and release totals must balance, including
     * buffers; counters are printed individually for the evidence parser. */
    long end = now_ms() + 5000;
    while (sunlight_net_diagnostic(2) && now_ms() < end) delay_ms(20);
    const int selectors[] = {2, 6, 9, 10, 11, 12, 13};
    for (unsigned i = 0; i < sizeof selectors / sizeof selectors[0]; ++i) {
        long value = sunlight_net_diagnostic(selectors[i]);
        printf("dup-runtime: counter %d=%ld\n", selectors[i], value);
        check(value == 0, "final-network-resource-zero");
    }
    long allocated = sunlight_net_diagnostic(0), released = sunlight_net_diagnostic(1);
    printf("dup-runtime: allocated=%ld released=%ld failures=%d\n", allocated, released, failures);
    check(allocated == released, "network-allocation-release-balanced");
    printf("dup-runtime: %s complete\n", failures ? "FAIL" : "PASS");
    return failures != 0;
}
