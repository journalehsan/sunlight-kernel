/* Deterministic native TCP readiness acceptance through the real net_server.
 *
 * The support service owns the control connection and connects to two
 * client-owned listeners only when commanded. That makes listener readiness
 * observable before a peer exists and lets the client synchronize FIN/data
 * ordering without fixed sleeps.
 */
#include <errno.h>
#include <netinet/in.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sunlight_net.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static const char *failure_step = "start";
static int failure_errno;

static long now_ms(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) return 0;
    return ts.tv_sec * 1000L + ts.tv_nsec / 1000000L;
}

static void sleep_ms(long ms) {
    struct timespec req;
    req.tv_sec = ms / 1000;
    req.tv_nsec = (ms % 1000) * 1000000L;
    (void)nanosleep(&req, 0);
}

static int fail(const char *step) {
    failure_step = step;
    failure_errno = errno;
    return -1;
}

static void fill_sockaddr(struct sockaddr_in *addr, unsigned short port) {
    memset(addr, 0, sizeof *addr);
    addr->sin_family = AF_INET;
    addr->sin_port = htons(port);
    addr->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
}

static int connect_ready(unsigned short port, long deadline_ms) {
    for (;;) {
        int fd = socket(AF_INET, SOCK_STREAM, 0);
        if (fd < 0) return fail("control-socket");
        struct sockaddr_in addr;
        fill_sockaddr(&addr, port);
        if (connect(fd, (struct sockaddr *)&addr, sizeof addr) == 0) return fd;
        int error = errno;
        close(fd);
        if (now_ms() >= deadline_ms) {
            errno = error;
            return fail("control-connect");
        }
        sleep_ms(20);
    }
}

static int bind_listener(unsigned short port, long deadline_ms) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return fail("listener-socket");
    struct sockaddr_in addr;
    fill_sockaddr(&addr, port);
    for (;;) {
        if (bind(fd, (struct sockaddr *)&addr, sizeof addr) == 0) break;
        int error = errno;
        if (error == EADDRINUSE || error == EINVAL || now_ms() >= deadline_ms) {
            errno = error;
            close(fd);
            return fail("listener-bind");
        }
        sleep_ms(20);
    }
    if (listen(fd, 1) != 0) {
        int error = errno;
        close(fd);
        errno = error;
        return fail("listener-listen");
    }
    return fd;
}

static int send_byte(int fd, unsigned char byte) {
    for (;;) {
        ssize_t n = send(fd, &byte, 1, 0);
        if (n == 1) return 0;
        if (n < 0 && errno == EAGAIN) continue;
        return -1;
    }
}

static int read_exact(int fd, unsigned char *buf, unsigned long len) {
    unsigned long offset = 0;
    while (offset < len) {
        ssize_t n = recv(fd, buf + offset, len - offset, 0);
        if (n == 0) return 0;
        if (n < 0) {
            if (errno == EAGAIN) continue;
            return -1;
        }
        offset += (unsigned long)n;
    }
    return 1;
}

/* Poll one descriptor and return its revents, 0 for timeout, or -1. */
static int poll_one(int fd, short events, int timeout_ms) {
    struct pollfd record = {fd, events, 0};
    int ready = poll(&record, 1, timeout_ms);
    if (ready < 0) return -1;
    if (ready == 0) return 0;
    return record.revents;
}

int main(void) {
    int control = -1;
    int data1 = -1;
    int data2 = -1;
    int listener1 = -1;
    int listener2 = -1;

    control = connect_ready(4040, now_ms() + 15000);
    if (control < 0) goto failed;

    failure_step = "listener";
    listener1 = bind_listener(4051, now_ms() + 5000);
    if (listener1 < 0) goto failed;
    if (poll_one(listener1, POLLIN, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }

    failure_step = "finite-timeout-no-event";
    {
        long started = now_ms();
        if (poll_one(listener1, POLLIN, 80) != 0) goto failed;
        if (now_ms() - started < 50) {
            errno = EINVAL;
            goto failed;
        }
    }

    failure_step = "invalid-and-negative";
    {
        struct pollfd records[3] = {
            {-1, POLLIN, 0},
            {999, POLLIN, 0},
            {listener1, POLLIN, 0},
        };
        int ready = poll(records, 3, 0);
        if (ready != 1) {
            errno = EINVAL;
            goto failed;
        }
        if (records[0].revents != 0 || records[1].revents != POLLNVAL ||
            records[2].revents != 0) {
            errno = EINVAL;
            goto failed;
        }
    }

    failure_step = "events-zero-error";
    {
        int unconnected = socket(AF_INET, SOCK_STREAM, 0);
        if (unconnected < 0) goto failed;
        struct pollfd record = {unconnected, 0, 0};
        if (poll(&record, 1, 0) != 1 || record.revents != POLLERR) {
            close(unconnected);
            errno = EINVAL;
            goto failed;
        }
        close(unconnected);
    }

    failure_step = "events-zero-hangup";
    {
        int pair[2];
        if (socketpair(AF_UNIX, SOCK_STREAM, 0, pair) != 0) goto failed;
        close(pair[0]);
        struct pollfd record = {pair[1], 0, 0};
        if (poll(&record, 1, 0) != 1 || (record.revents & POLLHUP) == 0) {
            close(pair[1]);
            errno = EINVAL;
            goto failed;
        }
        close(pair[1]);
    }

    failure_step = "listener-accept-readiness";
    if (send_byte(control, 'C') != 0) goto failed;
    if (poll_one(listener1, POLLIN, 2000) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    data1 = accept(listener1, 0, 0);
    if (data1 < 0) goto failed;

    failure_step = "idle-stream";
    if (poll_one(data1, POLLIN, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }
    if (poll_one(data1, POLLIN | POLLOUT, 0) != POLLOUT) {
        errno = EINVAL;
        goto failed;
    }

    failure_step = "writable-stream";
    if (poll_one(data1, POLLOUT, 500) != POLLOUT) {
        errno = EINVAL;
        goto failed;
    }
    if (send(data1, "P", 1, 0) != 1) goto failed;
    if (poll_one(data1, POLLIN, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }

    failure_step = "incoming-bytes";
    if (send_byte(control, 'S') != 0) goto failed;
    if (poll_one(data1, POLLIN, 2000) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    {
        unsigned char hello[5];
        if (read_exact(data1, hello, sizeof hello) != 1 ||
            memcmp(hello, "HELLO", sizeof hello) != 0) {
            errno = EINVAL;
            goto failed;
        }
    }

    failure_step = "partial-drain";
    if (send_byte(control, 'M') != 0) goto failed;
    if (poll_one(data1, POLLIN, 2000) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    {
        unsigned char first = 0;
        unsigned char rest[4];
        if (recv(data1, &first, 1, 0) != 1 || first != 'W') {
            errno = EINVAL;
            goto failed;
        }
        if (poll_one(data1, POLLIN, 0) != POLLIN) {
            errno = EINVAL;
            goto failed;
        }
        if (read_exact(data1, rest, sizeof rest) != 1 ||
            memcmp(rest, "ORLD", sizeof rest) != 0) {
            errno = EINVAL;
            goto failed;
        }
    }
    if (poll_one(data1, POLLIN, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }

    failure_step = "select-integration";
    if (send_byte(control, 'S') != 0) goto failed;
    {
        fd_set readable;
        FD_ZERO(&readable);
        FD_SET(data1, &readable);
        struct timeval timeout = {2, 0};
        if (select(data1 + 1, &readable, 0, 0, &timeout) != 1 ||
            !FD_ISSET(data1, &readable)) {
            errno = EINVAL;
            goto failed;
        }
    }
    {
        unsigned char hello[5];
        if (read_exact(data1, hello, sizeof hello) != 1 ||
            memcmp(hello, "HELLO", sizeof hello) != 0) {
            errno = EINVAL;
            goto failed;
        }
    }
    if (poll_one(data1, POLLIN, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }

    failure_step = "second-listener";
    listener2 = bind_listener(4052, now_ms() + 5000);
    if (listener2 < 0) goto failed;
    if (poll_one(listener2, POLLIN, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }
    if (send_byte(control, 'D') != 0) goto failed;
    if (poll_one(listener2, POLLIN, 2000) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    data2 = accept(listener2, 0, 0);
    if (data2 < 0) goto failed;
    close(listener2);
    listener2 = -1;
    if (poll_one(data2, POLLIN, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }

    failure_step = "multi-descriptor";
    if (send_byte(control, 'E') != 0) goto failed;
    {
        struct pollfd records[4] = {
            {data1, POLLIN, 0},
            {data2, POLLIN, 0},
            {control, POLLIN, 0},
            {-1, POLLIN, 0},
        };
        int ready = poll(records, 4, 2000);
        if (ready != 1 || records[0].revents != 0 ||
            records[1].revents != POLLIN || records[2].revents != 0 ||
            records[3].revents != 0) {
            printf("tcp-poll: multi ready=%d revents=%d,%d,%d,%d\n",
                   ready, records[0].revents, records[1].revents,
                   records[2].revents, records[3].revents);
            errno = EINVAL;
            goto failed;
        }
        unsigned char mark = 0;
        if (recv(data2, &mark, 1, 0) != 1 || mark != 'Y') {
            errno = EINVAL;
            goto failed;
        }
        if (poll(records, 4, 0) != 0) {
            errno = EINVAL;
            goto failed;
        }
    }

    failure_step = "repeated-wakeup";
    for (int iteration = 0; iteration < 16; ++iteration) {
        if (send_byte(control, 'N') != 0) goto failed;
        if (poll_one(data1, POLLIN, 2000) != POLLIN) {
            errno = EINVAL;
            goto failed;
        }
        unsigned char byte = 0;
        if (recv(data1, &byte, 1, 0) != 1 || byte != 'x') {
            errno = EINVAL;
            goto failed;
        }
        if (poll_one(data1, POLLIN, 0) != 0) {
            errno = EINVAL;
            goto failed;
        }
    }

    failure_step = "unread-data-then-fin";
    if (send_byte(control, 'F') != 0) goto failed;
    if (poll_one(data1, POLLIN, 2000) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    {
        unsigned char payload[8];
        if (read_exact(data1, payload, sizeof payload) != 1 ||
            memcmp(payload, "ABCDEFGH", sizeof payload) != 0) {
            errno = EINVAL;
            goto failed;
        }
    }
    if (poll_one(data1, POLLIN, 2000) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    {
        unsigned char byte = 0;
        if (recv(data1, &byte, 1, 0) != 0) {
            errno = EINVAL;
            goto failed;
        }
    }
    if (poll_one(data1, POLLIN, 0) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    /* CloseWait is an EOF, not a fully closed socket: events=0 stays 0. */
    if (poll_one(data1, 0, 0) != 0) {
        errno = EINVAL;
        goto failed;
    }

    failure_step = "close-reuse";
    {
        int old = data1;
        close(data1);
        data1 = -1;
        int reused = socket(AF_INET, SOCK_STREAM, 0);
        if (reused < 0) goto failed;
        if (reused != old) {
            printf("tcp-poll: reuse old=%d new=%d\n", old, reused);
            errno = EINVAL;
            close(reused);
            goto failed;
        }
        struct pollfd record = {reused, POLLIN | POLLOUT, 0};
        int ready = poll(&record, 1, 0);
        if (ready != 1 || record.revents != POLLERR) {
            printf("tcp-poll: reuse-ready=%d revents=%d\n", ready, record.revents);
            close(reused);
            errno = EINVAL;
            goto failed;
        }
        close(reused);
    }

    failure_step = "diagnostics";
    close(data2);
    data2 = -1;
    close(listener1);
    listener1 = -1;
    close(control);
    control = -1;
    long live = -1;
    long listeners = -1;
    long streams = -1;
    {
        long deadline = now_ms() + 5000;
        do {
            live = sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_SOCKET_LIVE);
            listeners = sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_LISTENER_LIVE);
            streams = sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_STREAM_LIVE);
            if (live == 0 && listeners == 0 && streams == 0) break;
            sleep_ms(20);
        } while (now_ms() < deadline);
        if (live != 0 || listeners != 0 || streams != 0) goto failed;
    }

    printf("tcp-poll: PASS socket_live=%ld listener_live=%ld stream_live=%ld\n",
           live, listeners, streams);
    return 0;

failed:
    if (failure_errno == 0) failure_errno = errno;
    if (data1 >= 0) close(data1);
    if (data2 >= 0) close(data2);
    if (listener1 >= 0) close(listener1);
    if (listener2 >= 0) close(listener2);
    if (control >= 0) close(control);
    printf("tcp-poll: FAIL step=%s errno=%d\n", failure_step, failure_errno);
    return 1;
}
