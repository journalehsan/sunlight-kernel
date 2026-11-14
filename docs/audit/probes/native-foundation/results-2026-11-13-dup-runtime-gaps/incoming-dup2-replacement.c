/* Descriptor replacement identity fixture.
 *
 * The native C surface exports no dup/dup2 symbol and <unistd.h> declares
 * neither, so this probe reaches the guest ABI directly with syscall 46.
 * That is the same technique the shipped open-unlink.c fixture uses for
 * syscall 45. Every assertion is deterministic: readiness is created before
 * the replacement, so no sleep or timing window is needed.
 *
 * Contract under test (see the dated NATIVE_DUP2_REPLACEMENT document under
 * docs/networking):
 *   - replacing a registered fd number ends that fd-keyed registration;
 *   - a replaced endpoint's readiness never becomes an event for the new
 *     identity, for local and TCP descriptors;
 *   - the stale transport attachment of a replaced TCP fd is never used for
 *     I/O (fail closed with ENOTCONN, not the replaced endpoint's data);
 *   - local descriptors keep ordinary local semantics after the replacement;
 *   - dup2(fd, fd), invalid source and invalid target change nothing;
 *   - repeated replacement and final close return net_server counters to
 *     zero.
 */
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

_Static_assert(sizeof(struct epoll_event) == 16, "native epoll event size");
_Static_assert(offsetof(struct epoll_event, data) == 8, "native data offset");

#define TAG 0x5a5a5a5a00000001ULL
#define ERR_EBADF_RAW (-5)

static int failures;

static int check(int condition, const char *step) {
    if (condition) {
        printf("dup2-replacement: PASS %s\n", step);
    } else {
        printf("dup2-replacement: FAIL %s (errno=%d)\n", step, errno);
        failures++;
    }
    return condition;
}

static long call3(long number, long a, long b, long c) {
    __asm__ volatile("syscall"
                     : "+a"(number)
                     : "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory");
    return number;
}

static long raw_dup2(int old_fd, int new_fd) {
    return call3(46, old_fd, new_fd, 0);
}

static long now_ms(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) return -1;
    return t.tv_sec * 1000L + t.tv_nsec / 1000000;
}

static void delay_ms(long ms) {
    struct timespec t = { ms / 1000, (ms % 1000) * 1000000 };
    nanosleep(&t, 0);
}

static int make_pipe(int pair[2], const char *byte) {
    if (call3(47, (long)pair, 0, 0)) return 0;
    return write(pair[1], byte, 1) == 1;
}

extern void *sl_std_spawn(void *(*entry)(void *), void *arg, size_t stack);
extern void sl_std_join(void *handle);

struct replace_job {
    int target;
    atomic_int go[8];
    atomic_int done[8];
};

/* One replacement per iteration, published only after the kernel syscall has
 * returned. The main thread never sleeps: it arms an iteration, spins on
 * epoll_wait(0), and only then inspects the post-replacement state. */
static void *replace_worker(void *arg) {
    struct replace_job *job = arg;
    for (int i = 0; i < 8; ++i) {
        while (!atomic_load_explicit(&job->go[i], memory_order_acquire)) {
            __asm__ volatile("pause" ::: "memory");
        }
        int source[2] = { -1, -1 };
        char byte = (char)('A' + i);
        if (!make_pipe(source, &byte) ||
            raw_dup2(source[0], job->target) != job->target) {
            atomic_store_explicit(&job->done[i], -1, memory_order_release);
            continue;
        }
        /* The target keeps its own reference; the worker's numbers go away. */
        if (close(source[0]) || close(source[1])) {
            atomic_store_explicit(&job->done[i], -1, memory_order_release);
            continue;
        }
        atomic_store_explicit(&job->done[i], 1, memory_order_release);
    }
    return 0;
}

/* Loopback listener + client + accepted stream, all owned by this process. */
static int connect_pair(int port, int *listener, int *client, int *stream) {
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof addr);
    addr.sin_family = AF_INET;
    addr.sin_port = htons((unsigned short)port);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    *listener = socket(AF_INET, SOCK_STREAM, 0);
    if (*listener < 0) return 0;
    long end = now_ms() + 5000;
    int bound;
    do {
        bound = bind(*listener, (struct sockaddr *)&addr, sizeof addr);
        if (bound) delay_ms(20);
    } while (bound && now_ms() < end);
    if (bound || listen(*listener, 1)) return 0;
    *client = socket(AF_INET, SOCK_STREAM, 0);
    if (*client < 0) return 0;
    if (connect(*client, (struct sockaddr *)&addr, sizeof addr)) return 0;
    *stream = accept(*listener, 0, 0);
    return *stream >= 0;
}

int main(void) {
    struct epoll_event event = { .events = EPOLLIN, .data = TAG };
    struct epoll_event out;
    int ep = epoll_create1(0);
    check(ep >= 0, "epoll-create");
    if (ep < 0) return 1;

    /* 1. Local source replacing a registered local target. The new object is
     *    already readable before the replacement, so a surviving fd-keyed
     *    registration reports the new identity's readiness under the old
     *    token and data. */
    int old_pipe[2] = { -1, -1 }, new_pipe[2] = { -1, -1 };
    if (check(make_pipe(old_pipe, "A") && make_pipe(new_pipe, "B"),
              "local-pipes-created")) {
        if (check(epoll_ctl(ep, EPOLL_CTL_ADD, old_pipe[0], &event) == 0,
                  "local-target-add")) {
            check(epoll_wait(ep, &out, 1, 0) == 1 && out.data == TAG,
                  "local-readiness-before-replacement");
            check(raw_dup2(new_pipe[0], old_pipe[0]) == old_pipe[0],
                  "local-replacement-syscall");
            check(epoll_wait(ep, &out, 1, 0) == 0,
                  "local-target-registration-invalidated");
            char byte = 0;
            check(read(old_pipe[0], &byte, 1) == 1 && byte == 'B',
                  "local-target-keeps-local-semantics");
        }
        check(close(old_pipe[0]) == 0 && close(old_pipe[1]) == 0 &&
              close(new_pipe[0]) == 0 && close(new_pipe[1]) == 0,
              "local-pipes-closed");
    }

    /* 2. dup2(fd, fd) is a no-op; invalid source/target never destroy the
     *    target's registration or content. */
    int keep_pipe[2] = { -1, -1 };
    if (check(make_pipe(keep_pipe, "C"), "keep-pipe-created")) {
        event.data = TAG + 1;
        if (check(epoll_ctl(ep, EPOLL_CTL_ADD, keep_pipe[0], &event) == 0,
                  "target-add-before-negative-cases")) {
            check(raw_dup2(keep_pipe[0], keep_pipe[0]) == keep_pipe[0],
                  "same-fd-noop");
            check(raw_dup2(-1, keep_pipe[0]) == ERR_EBADF_RAW,
                  "invalid-source-rejected");
            check(raw_dup2(keep_pipe[0], 300) == ERR_EBADF_RAW,
                  "invalid-target-rejected");
            check(epoll_wait(ep, &out, 1, 0) == 1 && out.data == TAG + 1,
                  "invalid-source-preserves-registration");
            char byte = 0;
            check(read(keep_pipe[0], &byte, 1) == 1 && byte == 'C',
                  "invalid-source-preserves-target-content");
        }
        check(close(keep_pipe[0]) == 0 && close(keep_pipe[1]) == 0,
              "keep-pipe-closed");
    }

    /* 3. TCP source replacing a registered TCP target. Both endpoints are
     *    real loopback streams with pending receive data before the
     *    replacement. */
    int listener1 = -1, client1 = -1, stream_a = -1;
    int listener2 = -1, client2 = -1, stream_b = -1;
    if (check(connect_pair(4311, &listener1, &client1, &stream_a) &&
              connect_pair(4312, &listener2, &client2, &stream_b),
              "tcp-pairs-connected")) {
        event.data = TAG + 2;
        if (check(epoll_ctl(ep, EPOLL_CTL_ADD, stream_a, &event) == 0,
                  "tcp-target-add")) {
            check(write(client1, "A", 1) == 1, "tcp-target-peer-send");
            check(epoll_wait(ep, &out, 1, 2000) == 1 && out.data == TAG + 2,
                  "tcp-readiness-before-replacement");
            check(write(client2, "B", 1) == 1, "tcp-source-peer-send");
            check(raw_dup2(stream_b, stream_a) == stream_a,
                  "tcp-to-tcp-replacement-syscall");
            check(epoll_wait(ep, &out, 1, 0) == 0,
                  "tcp-target-registration-invalidated");
            char byte = 0;
            errno = 0;
            check(read(stream_a, &byte, 1) == -1 && errno == ENOTCONN,
                  "replaced-tcp-target-fails-closed");
            check(read(stream_b, &byte, 1) == 1 && byte == 'B',
                  "tcp-source-identity-preserved");
            check(write(stream_b, "z", 1) == 1,
                  "tcp-source-still-writable");
        }
    }

    /* 4. TCP source replacing a registered local target. */
    int local_target[2] = { -1, -1 };
    if (stream_b >= 0 && check(make_pipe(local_target, "D"),
                               "local-target-for-tcp-source")) {
        event.data = TAG + 3;
        if (check(epoll_ctl(ep, EPOLL_CTL_ADD, local_target[0], &event) == 0,
                  "tcp-source-local-target-add")) {
            check(epoll_wait(ep, &out, 1, 0) == 1 && out.data == TAG + 3,
                  "local-target-readiness-before-tcp-replacement");
            check(raw_dup2(stream_b, local_target[0]) == local_target[0],
                  "tcp-over-local-replacement-syscall");
            check(epoll_wait(ep, &out, 1, 0) == 0,
                  "local-target-registration-invalidated");
            char byte = 0;
            errno = 0;
            check(read(local_target[0], &byte, 1) == -1 && errno == ENOTCONN,
                  "tcp-over-local-fails-closed");
        }
    }

    /* 5. Local source replacing a registered TCP target. The replaced fd
     *    becomes an ordinary local descriptor and must not keep the old
     *    transport attachment for I/O. */
    int listener3 = -1, client3 = -1, stream_c = -1;
    int local_source[2] = { -1, -1 };
    if (check(connect_pair(4313, &listener3, &client3, &stream_c),
              "third-tcp-pair-connected") &&
        check(make_pipe(local_source, "E"), "local-source-created")) {
        event.data = TAG + 4;
        if (check(epoll_ctl(ep, EPOLL_CTL_ADD, stream_c, &event) == 0,
                  "tcp-target-add-for-local-source")) {
            check(write(client3, "F", 1) == 1, "third-peer-send");
            check(epoll_wait(ep, &out, 1, 2000) == 1 && out.data == TAG + 4,
                  "third-readiness-before-replacement");
            check(raw_dup2(local_source[0], stream_c) == stream_c,
                  "local-over-tcp-replacement-syscall");
            check(epoll_wait(ep, &out, 1, 0) == 0,
                  "tcp-target-invalidated-by-local-source");
            char byte = 0;
            check(read(stream_c, &byte, 1) == 1 && byte == 'E',
                  "local-source-keeps-local-semantics");
        }
    }

    /* 6. Repeated replacement: every new fd-keyed registration is a fresh
     *    token and the next replacement invalidates it again. */
    int repeat_target[2] = { -1, -1 };
    if (check(make_pipe(repeat_target, "R"), "repeat-pipe-created")) {
        int ok = 1;
        for (int i = 0; i < 8 && ok; ++i) {
            int source[2] = { -1, -1 };
            char byte = (char)('a' + i);
            if (!make_pipe(source, &byte)) { ok = 0; break; }
            event.data = TAG + 0x10 + (unsigned)i;
            if (epoll_ctl(ep, EPOLL_CTL_ADD, repeat_target[0], &event) != 0) {
                ok = 0;
                break;
            }
            if (epoll_wait(ep, &out, 1, 0) != 1) { ok = 0; break; }
            if (raw_dup2(source[0], repeat_target[0]) != repeat_target[0]) {
                ok = 0;
                break;
            }
            if (epoll_wait(ep, &out, 1, 0) != 0) { ok = 0; break; }
            if (close(source[0]) || close(source[1])) { ok = 0; break; }
        }
        check(ok, "repeated-replacement-invalidates-each-registration");
        check(close(repeat_target[0]) == 0 && close(repeat_target[1]) == 0,
              "repeat-pipe-closed");
    }

    /* 7. Concurrent replacement and readiness collection. The worker publishes
     *    only after the replacement syscall returns, so the post-handshake
     *    epoll_wait(0) must observe the invalidated registration. */
    int concurrent_target[2] = { -1, -1 };
    if (check(make_pipe(concurrent_target, "S"), "concurrent-target-created")) {
        struct replace_job job;
        job.target = concurrent_target[0];
        for (int i = 0; i < 8; ++i) {
            atomic_init(&job.go[i], 0);
            atomic_init(&job.done[i], 0);
        }
        void *worker = sl_std_spawn(replace_worker, &job, 65536);
        int ok = worker != 0;
        for (int i = 0; i < 8 && ok; ++i) {
            event.data = TAG + 0x30 + (unsigned)i;
            if (epoll_ctl(ep, EPOLL_CTL_ADD, concurrent_target[0], &event) != 0) {
                ok = 0;
                break;
            }
            atomic_store_explicit(&job.go[i], 1, memory_order_release);
            int observed = -2;
            for (;;) {
                observed = atomic_load_explicit(&job.done[i], memory_order_acquire);
                if (observed) break;
                epoll_wait(ep, &out, 1, 0);
            }
            if (observed != 1 || epoll_wait(ep, &out, 1, 0) != 0) ok = 0;
        }
        sl_std_join(worker);
        check(ok, "concurrent-replacement-invalidates-registration");
        check(close(concurrent_target[0]) == 0 &&
              close(concurrent_target[1]) == 0,
              "concurrent-target-closed");
    }

    if (stream_a >= 0) close(stream_a);
    if (stream_b >= 0) close(stream_b);
    if (stream_c >= 0) close(stream_c);
    if (client1 >= 0) close(client1);
    if (client2 >= 0) close(client2);
    if (client3 >= 0) close(client3);
    if (listener1 >= 0) close(listener1);
    if (listener2 >= 0) close(listener2);
    if (listener3 >= 0) close(listener3);
    if (local_target[0] >= 0) close(local_target[0]);
    if (local_target[1] >= 0) close(local_target[1]);
    if (local_source[0] >= 0) close(local_source[0]);
    if (local_source[1] >= 0) close(local_source[1]);
    check(close(ep) == 0, "epoll-closed");

    long end = now_ms() + 5000;
    long live = -1;
    do {
        live = sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_SOCKET_LIVE);
        if (live) delay_ms(20);
    } while (live && now_ms() < end);
    check(live == 0, "net-socket-live-zero");
    check(sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_LISTENER_LIVE) == 0,
          "net-listener-live-zero");
    check(sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_STREAM_LIVE) == 0,
          "net-stream-live-zero");

    if (failures) {
        printf("dup2-replacement: FAIL failures=%d\n", failures);
        return 1;
    }
    printf("dup2-replacement: PASS local tcp replacement invalidation "
           "fail-closed same-fd invalid-source cleanup\n");
    return 0;
}
