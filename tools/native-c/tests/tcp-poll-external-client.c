/* External TCP readiness client: the same framed exchange as the existing
 * host peer, but every write and read is preceded by poll() on the bridged
 * descriptor. Prints one machine-readable PASS/FAIL line.
 */
#include <errno.h>
#include <netinet/in.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sunlight_net.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static const unsigned long sizes[] = {1, 7, 64, 255, 1024};
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

static void fill_sockaddr(struct sockaddr_in *addr, unsigned long ip,
                          unsigned short port) {
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
        if (now_ms() >= deadline_ms) {
            errno = error;
            return fail("connect-deadline");
        }
        sleep_ms(20);
    }
}

static int write_all(int fd, const unsigned char *buf, unsigned long len) {
    unsigned long offset = 0;
    while (offset < len) {
        ssize_t n = send(fd, buf + offset, len - offset, 0);
        if (n <= 0) {
            if (n < 0 && errno == EAGAIN) continue;
            return -1;
        }
        offset += (unsigned long)n;
    }
    return 0;
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

static int wait_ready(int fd, short events) {
    struct pollfd record = {fd, events, 0};
    int ready = poll(&record, 1, 3000);
    if (ready < 0) return -1;
    if (ready == 0 || (record.revents & events) == 0) {
        errno = EINVAL;
        return -1;
    }
    return record.revents;
}

static void put_be32(unsigned char *p, unsigned long value) {
    p[0] = (unsigned char)(value >> 24);
    p[1] = (unsigned char)(value >> 16);
    p[2] = (unsigned char)(value >> 8);
    p[3] = (unsigned char)value;
}

int main(int argc, char **argv) {
    unsigned short port = 0;
    unsigned long ip = 0;
    if (argc < 3 || parse_port(argv[1], &port) != 0 ||
        parse_ipv4(argv[2], &ip) != 0) {
        printf("poll-external: FAIL step=arguments errno=%d\n", EINVAL);
        return 2;
    }
    const char *run_id = argc > 3 ? argv[3] : "external";
    printf("poll-external: run=%s dest=%lu.%lu.%lu.%lu:%u\n", run_id,
           (ip >> 24) & 0xffUL, (ip >> 16) & 0xffUL, (ip >> 8) & 0xffUL,
           ip & 0xffUL, port);

    int fd = connect_ready(ip, port, now_ms() + 15000);
    if (fd < 0) goto failed;

    unsigned long total = 0;
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
        payload[4 + sizes[index] / 2] = 0;
        payload[4 + sizes[index] / 3] = 0;
        put_be32(header, payload_len);

        if (wait_ready(fd, POLLOUT) < 0) {
            failure_step = "frame-pollout";
            goto failed;
        }
        if (write_all(fd, header, sizeof header) != 0 ||
            write_all(fd, payload, payload_len) != 0) {
            failure_step = "frame-write";
            goto failed;
        }
        if (wait_ready(fd, POLLIN) < 0) {
            failure_step = "frame-pollin";
            goto failed;
        }
        if (read_exact(fd, reply_header, sizeof reply_header) != 1 ||
            read_exact(fd, reply, payload_len) != 1 ||
            memcmp(reply_header, header, sizeof header) != 0 ||
            memcmp(reply, payload, payload_len) != 0) {
            failure_step = "frame-readback";
            errno = EINVAL;
            goto failed;
        }
        total += payload_len;
    }

    if (wait_ready(fd, POLLOUT) < 0) {
        failure_step = "terminator-pollout";
        goto failed;
    }
    unsigned char terminator[4] = {0, 0, 0, 0};
    if (write_all(fd, terminator, sizeof terminator) != 0) {
        failure_step = "terminator-write";
        goto failed;
    }
    if (wait_ready(fd, POLLIN) < 0) {
        failure_step = "eof-pollin";
        goto failed;
    }
    {
        unsigned char byte = 0;
        if (recv(fd, &byte, 1, 0) != 0) {
            failure_step = "eof-read";
            errno = EINVAL;
            goto failed;
        }
    }
    close(fd);
    fd = -1;

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
        if (live != 0 || listeners != 0 || streams != 0) {
            failure_step = "diagnostics";
            goto failed;
        }
    }
    printf("poll-external: PASS exchanges=5 bytes=%lu socket_live=%ld "
           "listener_live=%ld stream_live=%ld\n",
           total, live, listeners, streams);
    return 0;

failed:
    if (failure_errno == 0) failure_errno = errno;
    if (fd >= 0) close(fd);
    printf("poll-external: FAIL step=%s errno=%d\n", failure_step, failure_errno);
    return 1;
}
