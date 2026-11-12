/* Deterministic native TCP client for the stalled-peer failure path.
 *
 * The host peer accepts, reads one framed payload and withholds every response.
 * The bridge's documented bounded I/O wait must surface as recv() == -1 with
 * ETIMEDOUT, not as an invented POSIX behaviour or an unbounded sleep.
 */
#include <errno.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sunlight_net.h>
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

static int parse_port(const char *text, unsigned short *out) {
    unsigned long value = 0;
    if (text == 0 || *text == 0) return -1;
    for (const char *p = text; *p != 0; ++p) {
        if (*p < '0' || *p > '9') return -1;
        value = value * 10 + (unsigned long)(*p - '0');
        if (value > 65535) return -1;
    }
    if (value == 0) return -1;
    *out = (unsigned short)value;
    return 0;
}

static int parse_ipv4(const char *text, unsigned long *out) {
    unsigned long value = 0;
    const char *p = text;
    if (text == 0 || *text == 0) return -1;
    for (int part = 0; part < 4; ++part) {
        if (*p < '0' || *p > '9') return -1;
        unsigned long octet = 0;
        do {
            octet = octet * 10 + (unsigned long)(*p - '0');
            if (octet > 255) return -1;
            ++p;
        } while (*p >= '0' && *p <= '9');
        value = (value << 8) | octet;
        if (part == 3) break;
        if (*p != '.') return -1;
        ++p;
    }
    if (*p != 0) return -1;
    *out = value;
    return 0;
}

static void fill_sockaddr(struct sockaddr_in *addr, unsigned long ip, unsigned short port) {
    memset(addr, 0, sizeof *addr);
    addr->sin_family = AF_INET;
    addr->sin_port = htons(port);
    addr->sin_addr.s_addr = htonl(ip);
}

static int connect_ready(unsigned long ip, unsigned short port, long deadline_ms) {
    for (;;) {
        int fd = socket(AF_INET, SOCK_STREAM, 0);
        if (fd < 0) return fail("socket");
        struct sockaddr_in addr;
        fill_sockaddr(&addr, ip, port);
        if (connect(fd, (struct sockaddr *)&addr, sizeof addr) == 0) return fd;
        int error = errno;
        close(fd);
        if (error != ECONNREFUSED && error != ECONNRESET && error != ETIMEDOUT &&
            error != ENOTCONN) {
            errno = error;
            return fail("connect");
        }
        if (now_ms() >= deadline_ms) {
            errno = error;
            return fail("connect-deadline");
        }
        sleep_ms(20);
    }
}

static int write_all(int fd, const unsigned char *buf, unsigned long len) {
    unsigned long off = 0;
    while (off < len) {
        ssize_t n = send(fd, buf + off, len - off, 0);
        if (n <= 0) {
            if (n < 0 && errno == EAGAIN) continue;
            return -1;
        }
        off += (unsigned long)n;
    }
    return 0;
}

int main(int argc, char **argv) {
    unsigned long dest_ip = 0x7f000001UL;
    unsigned short port = 4040;
    if (argc > 1 && parse_port(argv[1], &port) != 0) {
        printf("tcp-stall: FAIL step=port errno=%d\n", EINVAL);
        return 2;
    }
    if (argc > 2 && parse_ipv4(argv[2], &dest_ip) != 0) {
        printf("tcp-stall: FAIL step=address errno=%d\n", EINVAL);
        return 2;
    }
    const char *run_id = argc > 3 ? argv[3] : 0;
    if (run_id != 0) {
        printf("tcp-stall: run=%s dest=%lu.%lu.%lu.%lu:%u\n", run_id,
               (dest_ip >> 24) & 0xffUL, (dest_ip >> 16) & 0xffUL,
               (dest_ip >> 8) & 0xffUL, dest_ip & 0xffUL, port);
    }

    int fd = connect_ready(dest_ip, port, now_ms() + 15000);
    if (fd < 0) goto failed;

    failure_step = "send";
    {
        unsigned char frame[12] = {0, 0, 0, 8, 's', 't', 'a', 'l', 'l', 'e', 'd', '!'};
        if (write_all(fd, frame, sizeof frame) != 0) goto failed;
    }

    failure_step = "stalled-recv";
    long started = now_ms();
    long elapsed;
    {
        unsigned char reply[16];
        errno = 0;
        ssize_t n = recv(fd, reply, sizeof reply, 0);
        elapsed = now_ms() - started;
        if (n != -1 || errno != ETIMEDOUT) {
            if (n >= 0) errno = 0;
            goto failed;
        }
    }
    close(fd);
    fd = -1;

    failure_step = "diagnostics";
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

    printf("tcp-stall: PASS errno=%d elapsed_ms=%ld socket_live=%ld listener_live=%ld stream_live=%ld\n",
           ETIMEDOUT, elapsed, live, listeners, streams);
    return 0;

failed:
    if (failure_errno == 0) failure_errno = errno;
    if (fd >= 0) close(fd);
    printf("tcp-stall: FAIL step=%s errno=%d\n", failure_step, failure_errno);
    return 1;
}
