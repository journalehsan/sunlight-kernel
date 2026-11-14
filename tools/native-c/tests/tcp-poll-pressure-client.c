/* Native POLLOUT send-backpressure / recovery / peer-termination fixture.
 *
 * Topology (external): client -> net_server -> virtio-net -> QEMU slirp ->
 * host peer (tools/native-c/tcp_pressure_peer.py). The client owns the data
 * streams; the peer owns the control commands. The client fills the write
 * path until the readiness predicate (smoltcp TX space) is false, then proves
 * poll(POLLOUT) withholds and recovers only after the peer is released.
 *
 * A separate data2 stream proves peer RST while waiting for write capacity.
 * Nonblocking fill uses MSG_DONTWAIT on the send path, an already-supported
 * mechanism; no general nonblocking socket support is assumed or added.
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

#define PAYLOAD_LEN (16UL * 1024UL * 1024UL)
#define STREAM_LEN (8UL + PAYLOAD_LEN)
#define RESET_CAP (48UL * 1024UL * 1024UL)
#define BURST (256UL * 1024UL)
#define BACKPRESSURE_WAIT_MS 800
#define RECOVERY_WAIT_MS 5000
#define TOTAL_DEADLINE_MS 150000L
#define MAX_ITERATIONS 4096
#define REQUIRED_CYCLES 3
#define CHUNK 65536UL

static unsigned char g_chunk[CHUNK];
static const char *failure_step = "start";
static int failure_errno;
static unsigned long g_cycles;
static unsigned long g_unstable;
static unsigned long g_stream_saturated_bytes;
static unsigned long g_reset_saturated_bytes;

/* Deterministic stream generated on the fly (no large static buffer):
 * bytes 0..7 are the 8-byte big-endian payload length; payload byte `i` is
 * (i * 31 + 7) & 0xff. The host recomputes both independently. */
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

static int send_byte(int fd, unsigned char byte) {
    for (;;) {
        ssize_t n = send(fd, &byte, 1, 0);
        if (n == 1) return 0;
        if (n < 0 && errno == EAGAIN) continue;
        return -1;
    }
}

/* Poll one descriptor: revents, 0 for timeout, -1 for an error. */
static int poll_one(int fd, short events, int timeout_ms) {
    struct pollfd record = {fd, events, 0};
    int ready = poll(&record, 1, timeout_ms);
    if (ready < 0) return -1;
    if (ready == 0) return 0;
    return record.revents;
}

/* Nonblocking write toward `limit`, generating deterministic bytes on the fly;
 * *saturated is set when the send path reports no capacity (smoltcp TX buffer
 * full) instead of a hard error. */
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

/* The native string subset has no strstr; return 1 when `needle` occurs in
 * `haystack`. Both are NUL-terminated. */
static int contains(const char *haystack, const char *needle) {
    size_t nlen = strlen(needle);
    if (nlen == 0) return 1;
    for (; *haystack != 0; ++haystack) {
        if (strncmp(haystack, needle, nlen) == 0) return 1;
    }
    return 0;
}

int main(int argc, char **argv) {
    unsigned long ip = 0;
    unsigned short ctrl_port = 0, data_port = 0, reset_port = 0;
    if (argc < 6 || parse_ipv4(argv[1], &ip) != 0 ||
        parse_port(argv[2], &ctrl_port) != 0 ||
        parse_port(argv[3], &data_port) != 0 ||
        parse_port(argv[4], &reset_port) != 0) {
        printf("poll-pressure: FAIL step=arguments errno=%d\n", EINVAL);
        return 2;
    }
    const char *run_id = argv[5];
    printf("poll-pressure: run=%s dest=%lu.%lu.%lu.%lu:%u stream=%lu\n",
           run_id, (ip >> 24) & 0xffUL, (ip >> 16) & 0xffUL,
           (ip >> 8) & 0xffUL, ip & 0xffUL, data_port,
           (unsigned long)STREAM_LEN);

    int control = -1, data = -1, reset = -1;
    control = connect_ready(ip, ctrl_port, now_ms() + 15000);
    if (control < 0) goto failed;
    data = connect_ready(ip, data_port, now_ms() + 15000);
    if (data < 0) goto failed;
    reset = connect_ready(ip, reset_port, now_ms() + 15000);
    if (reset < 0) goto failed;

    /* A. Initial capacity: POLLOUT is reported and a bounded write makes
     *    progress before the peer withholds anything. */
    failure_step = "initial-pollout";
    if (poll_one(data, POLLOUT, 0) != POLLOUT) {
        errno = EINVAL;
        goto failed;
    }
    failure_step = "initial-write";
    {
        fill_chunk(g_chunk, 0, 8);
        ssize_t n = send(data, g_chunk, 8, MSG_DONTWAIT);
        if (n != 8) goto failed;
    }
    if (poll_one(data, POLLOUT, 0) != POLLOUT) {
        /* The peer is withholding; 8 bytes cannot saturate the path. */
        errno = EINVAL;
        goto failed;
    }

    /* B/C/D. Fill until the readiness predicate is false, prove poll withholds,
     *        release the peer, prove recovery, and repeat. */
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
            g_stream_saturated_bytes = offset;
            int snapshot = poll_one(data, POLLOUT, 0);
            if (snapshot < 0) {
                failure_step = "fill-poll0";
                goto failed;
            }
            if (snapshot != 0) {
                /* Capacity reappeared between the send and the snapshot: a
                 * scheduling race, not a contract failure. Re-fill. */
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
                /* Capacity legitimately changed while the peer withheld reads
                 * (intervening socket/slirp buffers draining). Record and
                 * re-establish saturation. */
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
            int recovered = poll_one(data, POLLOUT, RECOVERY_WAIT_MS);
            if (recovered != POLLOUT) {
                failure_step = "recovery-pollout";
                errno = EINVAL;
                goto failed;
            }
            long recovery_ms = now_ms() - recovery_start;
            printf("poll-pressure: cycle=%lu filled=%lu held_ms=%ld "
                   "recovery_ms=%ld\n",
                   g_cycles, offset, held_ms, recovery_ms);
            unsigned long burst = offset + BURST;
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
        /* The milestone requires repetition across the fill/drain transition
         * so waiter re-registration is exercised more than once. Fewer than
         * REQUIRED_CYCLES is a partial result, not a pass. */
        failure_step = "insufficient-cycles";
        errno = EINVAL;
        goto failed;
    }

    /* Validate the complete stream on the peer, which recomputes the expected
     * length and FNV-1a 64 hash independently. */
    failure_step = "verify";
    if (send_byte(control, 'D') != 0) goto failed;
    unsigned char verdict[256];
    if (send_byte(control, 'Q') != 0) goto failed;
    {
        unsigned long length = 0;
        for (;;) {
            if (length + 1 >= sizeof verdict) {
                errno = EIO;
                goto failed;
            }
            ssize_t n = recv(control, verdict + length, 1, 0);
            if (n == 1) {
                if (verdict[length] == '\n') break;
                length += 1;
                continue;
            }
            if (n == 0) {
                errno = EIO;
                goto failed;
            }
            if (n < 0 && errno == EAGAIN) continue;
            goto failed;
        }
        verdict[length] = 0;
    }
    if (!contains((const char *)verdict, "HOST-PEER PASS")) {
        failure_step = "peer-verdict";
        errno = EINVAL;
        goto failed;
    }

    /* Orderly peer FIN: shutdown(SHUT_WR) on the host half-close. This is not
     * a reset and must not make the local send direction unusable. */
    failure_step = "half-close-fin";
    if (send_byte(control, 'F') != 0) goto failed;
    {
        int fin = poll_one(data, POLLIN, 3000);
        if (fin != POLLIN) {
            errno = EINVAL;
            goto failed;
        }
        unsigned char byte = 0;
        if (recv(data, &byte, 1, 0) != 0) {
            errno = EINVAL;
            goto failed;
        }
        /* Receive-side FIN only: the send direction remains usable. */
        ssize_t n = send(data, "tail", 4, MSG_DONTWAIT);
        if (n != 4) {
            failure_step = "half-close-write";
            goto failed;
        }
        if (poll_one(data, POLLOUT, 0) != POLLOUT) {
            errno = EINVAL;
            goto failed;
        }
    }

    /* E. Peer RST while the local writer waits for capacity. */
    failure_step = "reset-fill";
    {
        unsigned long offset = 0;
        /* Fill until the readiness predicate is false and a finite poll
         * actually holds. Intervening socket/slirp buffers can transiently
         * recover, so retry within the cap rather than treating one sample as
         * proof; reaching the cap without an observed hold is a failed
         * saturation attempt, not a pass. */
        int held = 0;
        while (offset < RESET_CAP) {
            int saturated = 0;
            if (push_stream(reset, &offset, RESET_CAP, &saturated) != 0) {
                failure_step = "reset-send";
                goto failed;
            }
            if (!saturated) break;
            if (poll_one(reset, POLLOUT, 0) != 0) continue;
            if (poll_one(reset, POLLOUT, 400) == 0) {
                held = 1;
                break;
            }
        }
        if (!held) {
            failure_step = "reset-not-saturated";
            errno = EINVAL;
            goto failed;
        }
        g_reset_saturated_bytes = offset;
        if (send_byte(control, 'R') != 0) goto failed;
        long started = now_ms();
        int revents = poll_one(reset, POLLOUT, 3000);
        long elapsed = now_ms() - started;
        if (revents <= 0) {
            failure_step = "reset-wait-timeout";
            errno = ETIMEDOUT;
            goto failed;
        }
        if ((revents & (POLLERR | POLLHUP)) == 0) {
            failure_step = "reset-revents";
            errno = EINVAL;
            goto failed;
        }
        errno = 0;
        ssize_t n = send(reset, "Z", 1, MSG_DONTWAIT);
        int write_errno = errno;
        if (n >= 0) {
            failure_step = "reset-write-succeeded";
            errno = EINVAL;
            goto failed;
        }
        printf("poll-pressure: reset revents=0x%x elapsed_ms=%ld "
               "write_errno=%d\n", revents, elapsed, write_errno);
    }

    close(reset);
    reset = -1;
    close(data);
    data = -1;
    if (send_byte(control, 'X') != 0) goto failed;
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
        printf("poll-pressure: PASS cycles=%lu unstable=%lu "
               "stream_saturated_bytes=%lu reset_saturated_bytes=%lu "
               "bytes=%lu socket_live=%ld listener_live=%ld stream_live=%ld\n",
               g_cycles, g_unstable, g_stream_saturated_bytes,
               g_reset_saturated_bytes, (unsigned long)STREAM_LEN + 4,
               live, listeners, streams);
    }
    return 0;

failed:
    if (failure_errno == 0) failure_errno = errno;
    if (data >= 0) close(data);
    if (reset >= 0) close(reset);
    if (control >= 0) close(control);
    printf("poll-pressure: FAIL step=%s errno=%d cycles=%lu unstable=%lu\n",
           failure_step, failure_errno, g_cycles, g_unstable);
    return 1;
}
