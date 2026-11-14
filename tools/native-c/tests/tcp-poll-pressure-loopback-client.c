/* Loopback POLLOUT send-backpressure fixture.
 *
 * Guest-internal topology: this client owns the data/reset listeners and the
 * support service (tcp-poll-pressure-service) connects to them on command.
 * Both endpoints are guest sockets inside the same net_server smoltcp
 * interface, so withholding service reads shrinks the client's send window
 * and saturates the client's smoltcp transmit buffer deterministically.
 *
 * Mirrors the external pressure client's transitions, but the peer lives in
 * the guest; the service validates the deterministic stream and reports a
 * verdict over the same control connection.
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

#define PAYLOAD_LEN (256UL * 1024UL)
#define STREAM_LEN (8UL + PAYLOAD_LEN)
#define BACKPRESSURE_WAIT_MS 150
#define RECOVERY_WAIT_MS 5000
#define TOTAL_DEADLINE_MS 120000L
#define MAX_ITERATIONS 4096
#define REQUIRED_CYCLES 3
#define CHUNK 65536UL

static unsigned char g_chunk[CHUNK];
static const char *failure_step = "start";
static int failure_errno;
static unsigned long g_cycles;
static unsigned long g_unstable;
static unsigned long g_saturated_bytes;

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

static int poll_one(int fd, short events, int timeout_ms) {
    struct pollfd record = {fd, events, 0};
    int ready = poll(&record, 1, timeout_ms);
    if (ready < 0) return -1;
    if (ready == 0) return 0;
    return record.revents;
}

static void fill_chunk(unsigned char *dst, unsigned long start, unsigned long len) {
    for (unsigned long i = 0; i < len; ++i) {
        unsigned long offset = start + i;
        if (offset < 8) {
            dst[i] = (unsigned char)(PAYLOAD_LEN >> (8 * (7 - offset)));
        } else {
            unsigned long index = offset - 8;
            dst[i] = (unsigned char)((index * 31UL + 7UL) & 0xffUL);
        }
    }
}

static int push_stream(int fd, unsigned long *offset, unsigned long limit,
                       int *saturated) {
    *saturated = 0;
    while (*offset < limit) {
        unsigned long chunk = limit - *offset;
        if (chunk > CHUNK) chunk = CHUNK;
        fill_chunk(g_chunk, *offset, chunk);
        ssize_t n = send(fd, g_chunk, chunk, MSG_DONTWAIT);
        if (n > 0) {
            *offset += (unsigned long)n;
            continue;
        }
        if (n < 0 && errno == EAGAIN) {
            *saturated = 1;
            return 0;
        }
        if (n == 0) {
            errno = EIO;
            return -1;
        }
        return -1;
    }
    return 0;
}

static int contains(const char *haystack, const char *needle) {
    size_t nlen = strlen(needle);
    if (nlen == 0) return 1;
    for (; *haystack != 0; ++haystack) {
        if (strncmp(haystack, needle, nlen) == 0) return 1;
    }
    return 0;
}

static unsigned long read_line(int fd, unsigned char *buffer, unsigned long cap) {
    unsigned long length = 0;
    while (length + 1 < cap) {
        ssize_t n = recv(fd, buffer + length, 1, 0);
        if (n == 1) {
            if (buffer[length] == '\n') {
                buffer[length] = 0;
                return length;
            }
            length += 1;
            continue;
        }
        if (n == 0) break;
        if (n < 0 && errno == EAGAIN) {
            sleep_ms(5);
            continue;
        }
        break;
    }
    buffer[length] = 0;
    return length;
}

int main(void) {
    int control = -1, data = -1;
    int data_listener = -1;

    control = connect_ready(4040, now_ms() + 20000);
    if (control < 0) goto failed;
    data_listener = bind_listener(4061, now_ms() + 10000);
    if (data_listener < 0) goto failed;

    failure_step = "service-connect-data";
    if (send_byte(control, 'C') != 0) goto failed;
    if (poll_one(data_listener, POLLIN, 5000) != POLLIN) {
        errno = EINVAL;
        goto failed;
    }
    data = accept(data_listener, 0, 0);
    if (data < 0) goto failed;
    close(data_listener);
    data_listener = -1;

    /* A. Initial capacity. */
    failure_step = "initial-pollout";
    if (poll_one(data, POLLOUT, 0) != POLLOUT) {
        errno = EINVAL;
        goto failed;
    }
    failure_step = "initial-write";
    {
        fill_chunk(g_chunk, 0, 8);
        if (send(data, g_chunk, 8, MSG_DONTWAIT) != 8) goto failed;
    }

    /* B/C/D. Fill / withhold / recover / repeat. */
    failure_step = "fill";
    {
        unsigned long offset = 8;
        unsigned long iterations = 0;
        long deadline = now_ms() + TOTAL_DEADLINE_MS;
        while (offset < STREAM_LEN) {
            if (now_ms() > deadline) {
                errno = ETIMEDOUT;
                goto failed;
            }
            if (++iterations > MAX_ITERATIONS) {
                errno = EINVAL;
                goto failed;
            }
            int saturated = 0;
            if (push_stream(data, &offset, STREAM_LEN, &saturated) != 0) {
                failure_step = "fill-send";
                goto failed;
            }
            if (!saturated) break;
            g_saturated_bytes = offset;
            int snapshot = poll_one(data, POLLOUT, 0);
            if (snapshot < 0) {
                failure_step = "fill-poll0";
                goto failed;
            }
            if (snapshot != 0) {
                g_unstable++;
                continue;
            }
            long held_start = now_ms();
            int held = poll_one(data, POLLOUT, BACKPRESSURE_WAIT_MS);
            if (held < 0) {
                failure_step = "fill-poll-held";
                goto failed;
            }
            if (held != 0) {
                g_unstable++;
                continue;
            }
            long held_ms = now_ms() - held_start;
            g_cycles++;
            if (send_byte(control, 'D') != 0) {
                failure_step = "control-drain";
                goto failed;
            }
            long recovery_start = now_ms();
            if (poll_one(data, POLLOUT, RECOVERY_WAIT_MS) != POLLOUT) {
                failure_step = "recovery-pollout";
                errno = EINVAL;
                goto failed;
            }
            long recovery_ms = now_ms() - recovery_start;
            printf("poll-pressure-loopback: cycle=%lu filled=%lu held_ms=%ld "
                   "recovery_ms=%ld\n", g_cycles, offset, held_ms, recovery_ms);
            unsigned long burst = offset + CHUNK;
            if (burst > STREAM_LEN) burst = STREAM_LEN;
            int burst_saturated = 0;
            if (push_stream(data, &offset, burst, &burst_saturated) != 0) {
                failure_step = "burst-send";
                goto failed;
            }
            if (send_byte(control, 'H') != 0) {
                failure_step = "control-withhold";
                goto failed;
            }
        }
        if (offset != STREAM_LEN) {
            failure_step = "stream-incomplete";
            errno = EIO;
            goto failed;
        }
    }
    if (g_cycles < REQUIRED_CYCLES) {
        failure_step = "insufficient-cycles";
        errno = EINVAL;
        goto failed;
    }

    failure_step = "verify";
    if (send_byte(control, 'D') != 0) goto failed;
    if (send_byte(control, 'Q') != 0) goto failed;
    {
        unsigned char verdict[256];
        read_line(control, verdict, sizeof verdict);
        if (!contains((const char *)verdict, "PRESSURE-SERVICE PASS")) {
            printf("poll-pressure-loopback: verdict=%s\n", (const char *)verdict);
            failure_step = "peer-verdict";
            errno = EINVAL;
            goto failed;
        }
    }

    close(data);
    data = -1;
    if (send_byte(control, 'Z') != 0) goto failed;
    close(control);
    control = -1;

    failure_step = "diagnostics";
    {
        long live = -1, listeners = -1, streams = -1;
        long deadline = now_ms() + 8000;
        do {
            live = sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_SOCKET_LIVE);
            listeners = sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_LISTENER_LIVE);
            streams = sunlight_net_diagnostic(SUNLIGHT_NET_DIAGNOSTIC_STREAM_LIVE);
            if (live == 0 && listeners == 0 && streams == 0) break;
            sleep_ms(20);
        } while (now_ms() < deadline);
        if (live != 0 || listeners != 0 || streams != 0) goto failed;
        printf("poll-pressure-loopback: PASS cycles=%lu unstable=%lu "
               "saturated_bytes=%lu bytes=%lu socket_live=%ld listener_live=%ld "
               "stream_live=%ld\n", g_cycles, g_unstable, g_saturated_bytes,
               (unsigned long)STREAM_LEN, live, listeners, streams);
    }
    return 0;

failed:
    if (failure_errno == 0) failure_errno = errno;
    if (data >= 0) close(data);
    if (data_listener >= 0) close(data_listener);
    if (control >= 0) close(control);
    printf("poll-pressure-loopback: FAIL step=%s errno=%d cycles=%lu unstable=%lu\n",
           failure_step, failure_errno, g_cycles, g_unstable);
    return 1;
}
