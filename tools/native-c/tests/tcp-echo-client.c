/* Deterministic native TCP client for the bounded echo service.
 *
 * Exercises connect readiness, nonblocking no-data, getsockname/getpeername,
 * framing with partial reads/writes, multiple payload sizes with embedded NUL
 * bytes, peer EOF, a real refused-connection path, and net_server ownership
 * diagnostics. Prints one machine-readable PASS/FAIL line.
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

static int read_exact(int fd, unsigned char *buf, unsigned long len) {
    unsigned long off = 0;
    while (off < len) {
        ssize_t n = recv(fd, buf + off, len - off, 0);
        if (n == 0) return 0;
        if (n < 0) {
            if (errno == EAGAIN) continue;
            return -1;
        }
        off += (unsigned long)n;
    }
    return 1;
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

static unsigned long be32(const unsigned char *p) {
    return ((unsigned long)p[0] << 24) | ((unsigned long)p[1] << 16) |
           ((unsigned long)p[2] << 8) | (unsigned long)p[3];
}

static void put_be32(unsigned char *p, unsigned long value) {
    p[0] = (unsigned char)(value >> 24);
    p[1] = (unsigned char)(value >> 16);
    p[2] = (unsigned char)(value >> 8);
    p[3] = (unsigned char)value;
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

/* Parse a dotted-quad IPv4 literal into host order. */
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

/* Retry until the service listener is ready or the deadline expires. This is a
 * bounded readiness handshake, not a fixed sleep. */
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

int main(int argc, char **argv) {
    unsigned long dest_ip = 0x7f000001UL;
    unsigned short port = 4040;
    if (argc > 1 && parse_port(argv[1], &port) != 0) {
        printf("tcp-echo: FAIL step=port errno=%d\n", EINVAL);
        return 2;
    }
    if (argc > 2 && parse_ipv4(argv[2], &dest_ip) != 0) {
        printf("tcp-echo: FAIL step=address errno=%d\n", EINVAL);
        return 2;
    }
    const char *run_id = argc > 3 ? argv[3] : 0;
    if (run_id != 0) {
        printf("tcp-echo: run=%s dest=%lu.%lu.%lu.%lu:%u\n", run_id,
               (dest_ip >> 24) & 0xffUL, (dest_ip >> 16) & 0xffUL,
               (dest_ip >> 8) & 0xffUL, dest_ip & 0xffUL, port);
    }

    int fd = connect_ready(dest_ip, port, now_ms() + 15000);
    if (fd < 0) goto failed;

    failure_step = "nonblocking";
    {
        unsigned char byte;
        errno = 0;
        if (recv(fd, &byte, 1, MSG_DONTWAIT) != -1 || errno != EAGAIN) goto failed;
    }

    failure_step = "shutdown";
    {
        /* Bounded subset: shutdown() is explicitly unsupported, never a silent
         * success, and must leave the connected stream usable. */
        errno = 0;
        if (shutdown(fd, SHUT_WR) != -1 || errno != EOPNOTSUPP) goto failed;
    }

    failure_step = "endpoints";
    {
        struct sockaddr_in local;
        struct sockaddr_in peer;
        socklen_t local_len = sizeof local;
        socklen_t peer_len = sizeof peer;
        if (getsockname(fd, (struct sockaddr *)&local, &local_len) != 0) goto failed;
        if (getpeername(fd, (struct sockaddr *)&peer, &peer_len) != 0) goto failed;
        if (local_len != (socklen_t)sizeof local || peer_len != (socklen_t)sizeof peer) goto failed;
        if (local.sin_family != AF_INET || peer.sin_family != AF_INET) goto failed;
        if (peer.sin_port != htons(port)) goto failed;
        if (ntohl(peer.sin_addr.s_addr) != dest_ip) goto failed;
        if (ntohs(local.sin_port) == 0) goto failed;
    }

    static const unsigned long sizes[] = {1, 7, 64, 255, 1024};
    unsigned long total = 0;
    failure_step = "exchange";
    for (unsigned long index = 0; index < sizeof sizes / sizeof sizes[0]; ++index) {
        unsigned long payload_len = 4 + sizes[index];
        unsigned char payload[1028];
        unsigned char header[4];
        unsigned char reply_header[4];
        unsigned char reply[1028];
        put_be32(payload, index + 1);
        for (unsigned long i = 0; i < sizes[index]; ++i) {
            payload[4 + i] = (unsigned char)((index * 37UL + i * 11UL) & 0xffUL);
        }
        /* Guarantee embedded NUL bytes at non-terminal positions. */
        payload[4 + sizes[index] / 2] = 0;
        payload[4 + sizes[index] / 3] = 0;
        put_be32(header, payload_len);
        if (index == 0) {
            /* Exercise the connected sendto/recvfrom wrappers once: a null
             * destination behaves as send(), and recvfrom fills the peer. */
            struct sockaddr_in peer;
            socklen_t peer_len = sizeof peer;
            unsigned long off = 0;
            while (off < 4) {
                ssize_t n = sendto(fd, header + off, 4 - off, 0, 0, 0);
                if (n <= 0) goto failed;
                off += (unsigned long)n;
            }
            if (write_all(fd, payload, payload_len) != 0) goto failed;
            if (read_exact(fd, reply_header, 4) != 1) goto failed;
            if (be32(reply_header) != payload_len) goto failed;
            ssize_t first = recvfrom(fd, reply, payload_len, 0,
                                     (struct sockaddr *)&peer, &peer_len);
            if (first <= 0) goto failed;
            if (peer_len != (socklen_t)sizeof peer || peer.sin_family != AF_INET ||
                peer.sin_port != htons(port) ||
                ntohl(peer.sin_addr.s_addr) != dest_ip) {
                errno = EINVAL;
                goto failed;
            }
            unsigned long got = (unsigned long)first;
            while (got < payload_len) {
                ssize_t n = recv(fd, reply + got, payload_len - got, 0);
                if (n <= 0) goto failed;
                got += (unsigned long)n;
            }
        } else {
            if (write_all(fd, header, 4) != 0) goto failed;
            if (write_all(fd, payload, payload_len) != 0) goto failed;
            if (read_exact(fd, reply_header, 4) != 1) goto failed;
            if (be32(reply_header) != payload_len) goto failed;
            if (read_exact(fd, reply, payload_len) != 1) goto failed;
        }
        if (memcmp(payload, reply, payload_len) != 0) goto failed;
        total += payload_len;
    }

    failure_step = "terminate";
    {
        unsigned char end[4] = {0, 0, 0, 0};
        unsigned char byte;
        if (write_all(fd, end, 4) != 0) goto failed;
        if (read_exact(fd, &byte, 1) != 0) goto failed; /* peer EOF */
    }
    close(fd);
    fd = -1;

    int refused_errno = 0;
    failure_step = "refused";
    {
        int probe = socket(AF_INET, SOCK_STREAM, 0);
        if (probe < 0) goto failed;
        struct sockaddr_in addr;
        fill_sockaddr(&addr, dest_ip, port);
        if (connect(probe, (struct sockaddr *)&addr, sizeof addr) == 0) {
            close(probe);
            errno = 0;
            goto failed;
        }
        int error = errno;
        close(probe);
        if (error != ECONNREFUSED && error != ECONNRESET && error != ETIMEDOUT &&
            error != ENOTCONN) {
            errno = error;
            goto failed;
        }
        refused_errno = error;
    }

    int nonblocking_errno = 0;
    failure_step = "nonblocking-connect";
    {
        /* Asynchronous connect is outside the bounded subset and must be
         * rejected explicitly rather than silently blocking. */
        int probe = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
        if (probe < 0) goto failed;
        struct sockaddr_in addr;
        fill_sockaddr(&addr, dest_ip, port);
        errno = 0;
        if (connect(probe, (struct sockaddr *)&addr, sizeof addr) != -1 ||
            errno != EOPNOTSUPP) {
            close(probe);
            goto failed;
        }
        nonblocking_errno = errno;
        close(probe);
    }

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

    printf("tcp-echo: refused errno=%d\n", refused_errno);
    printf("tcp-echo: nonblocking-connect errno=%d\n", nonblocking_errno);
    printf("tcp-echo: PASS exchanges=5 bytes=%lu socket_live=%ld listener_live=%ld stream_live=%ld\n",
           total, live, listeners, streams);
    return 0;

failed:
    if (failure_errno == 0) failure_errno = errno;
    if (fd >= 0) close(fd);
    printf("tcp-echo: FAIL step=%s errno=%d\n", failure_step, failure_errno);
    return 1;
}
