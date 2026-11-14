/* Scripted loopback peer for the POLLOUT backpressure fixture.
 *
 * Unlike tools/native-c/tests/tcp-poll-service.c (kept unchanged for the
 * earlier readiness milestone), this peer can withhold application reads so
 * the client's smoltcp transmit buffer saturates, then drain on command and
 * independently validate the deterministic stream.
 *
 * This service deliberately exercises only transitions reachable with the
 * supported native socket surface. Peer reset is external-only: native libc
 * has no setsockopt/SO_LINGER and shutdown() is unsupported, so an in-guest
 * RST cannot be manufactured without adding general socket options.
 *
 * Control commands (one byte each), client -> service:
 *   'C' connect to the client's data listener (127.0.0.1:4061)
 *   'D' drain the data stream
 *   'H' withhold further data reads
 *   'Q' validate the drained stream, reply with one verdict line
 *   'Z' exit
 *
 * Stream: 8-byte big-endian payload length, payload[i] = (i * 31 + 7) & 0xff.
 * Validation is byte-exact length plus an independently recomputed FNV-1a 64.
 */
#include <errno.h>
#include <netinet/in.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#define MAX_STREAM (32UL * 1024UL * 1024UL)

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

static void fill_sockaddr(struct sockaddr_in *addr, unsigned short port) {
    memset(addr, 0, sizeof *addr);
    addr->sin_family = AF_INET;
    addr->sin_port = htons(port);
    addr->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
}

static int connect_port(unsigned short port, long deadline_ms) {
    for (;;) {
        int fd = socket(AF_INET, SOCK_STREAM, 0);
        if (fd < 0) return -1;
        struct sockaddr_in addr;
        fill_sockaddr(&addr, port);
        if (connect(fd, (struct sockaddr *)&addr, sizeof addr) == 0) return fd;
        int error = errno;
        close(fd);
        if (now_ms() >= deadline_ms) {
            errno = error;
            return -1;
        }
        sleep_ms(20);
    }
}

static int recv_byte(int fd, unsigned char *byte) {
    for (;;) {
        ssize_t n = recv(fd, byte, 1, 0);
        if (n == 1) return 1;
        if (n == 0) return 0;
        if (errno == EAGAIN) {
            sleep_ms(5);
            continue;
        }
        return -1;
    }
}

static unsigned long long fnv1a(unsigned long long hash,
                                const unsigned char *data, unsigned long len) {
    const unsigned long long prime = 0x100000001b3ULL;
    for (unsigned long i = 0; i < len; ++i) {
        hash ^= data[i];
        hash *= prime;
    }
    return hash;
}

/* The native stdio subset has no snprintf; build the verdict line manually. */
static unsigned long append_str(char *out, unsigned long at, const char *text) {
    while (*text != 0) out[at++] = *text++;
    out[at] = 0;
    return at;
}

static unsigned long append_ulong(char *out, unsigned long at, unsigned long value) {
    char digits[24];
    int count = 0;
    if (value == 0) {
        out[at++] = '0';
        out[at] = 0;
        return at;
    }
    while (value > 0 && count < 24) {
        digits[count++] = (char)('0' + (value % 10));
        value /= 10;
    }
    while (count > 0) out[at++] = digits[--count];
    out[at] = 0;
    return at;
}

static unsigned long append_hex64(char *out, unsigned long at,
                                  unsigned long long value) {
    static const char hex[] = "0123456789abcdef";
    for (int shift = 60; shift >= 0; shift -= 4) {
        out[at++] = hex[(value >> shift) & 0xf];
    }
    out[at] = 0;
    return at;
}

int main(void) {
    static unsigned char buffer[MAX_STREAM];
    int control_listener = socket(AF_INET, SOCK_STREAM, 0);
    if (control_listener < 0) return 1;
    struct sockaddr_in local;
    fill_sockaddr(&local, 4040);
    long deadline = now_ms() + 20000;
    for (;;) {
        if (bind(control_listener, (struct sockaddr *)&local, sizeof local) == 0) break;
        if (now_ms() >= deadline) return 2;
        sleep_ms(20);
    }
    if (listen(control_listener, 1) != 0) return 3;
    int control = accept(control_listener, 0, 0);
    if (control < 0) return 4;
    close(control_listener);

    int data = -1;
    unsigned long length = 0;
    int overflow = 0;
    int draining = 0;

    for (;;) {
        struct pollfd records[2];
        records[0].fd = control;
        records[0].events = POLLIN;
        records[0].revents = 0;
        int count = 1;
        if (draining && data >= 0) {
            records[1].fd = data;
            records[1].events = POLLIN;
            records[1].revents = 0;
            count = 2;
        }
        if (poll(records, count, 20) < 0) {
            if (errno == EAGAIN) continue;
            break;
        }
        unsigned char command = 0;
        int received = 0;
        if (records[0].revents & POLLIN) {
            received = recv_byte(control, &command);
        }
        if (received < 0) break;
        if (count == 2 && (records[1].revents & POLLIN)) {
            unsigned char chunk[65536];
            ssize_t n = recv(data, chunk, sizeof chunk, 0);
            if (n > 0) {
                if (length + (unsigned long)n > MAX_STREAM) {
                    overflow = 1;
                } else {
                    memcpy(buffer + length, chunk, (unsigned long)n);
                    length += (unsigned long)n;
                }
            } else if (n == 0) {
                draining = 0;
            }
        }
        if (received == 0) continue;
        if (command == 'C') {
            if (data >= 0) return 5;
            data = connect_port(4061, now_ms() + 20000);
            if (data < 0) return 6;
        } else if (command == 'D') {
            draining = 1;
        } else if (command == 'H') {
            draining = 0;
        } else if (command == 'Q') {
            if (data < 0) return 9;
            draining = 1;
            long drain_deadline = now_ms() + 30000;
            while (now_ms() < drain_deadline) {
                if (length >= 8) {
                    unsigned long have = 0;
                    for (int i = 0; i < 8; ++i) have = (have << 8) | buffer[i];
                    if (length >= 8 + have) break;
                }
                unsigned char chunk[65536];
                /* Nonblocking so the verdict is emitted promptly instead of
                 * blocking the control reply behind a 10 s read deadline. */
                ssize_t n = recv(data, chunk, sizeof chunk, MSG_DONTWAIT);
                if (n > 0) {
                    if (length + (unsigned long)n > MAX_STREAM) {
                        overflow = 1;
                        break;
                    }
                    memcpy(buffer + length, chunk, (unsigned long)n);
                    length += (unsigned long)n;
                } else if (n == 0) {
                    break;
                } else if (errno != EAGAIN) {
                    break;
                } else {
                    sleep_ms(5);
                }
            }
            char line[160];
            unsigned long at = 0;
            line[0] = 0;
            if (overflow) {
                at = append_str(line, at, "PRESSURE-SERVICE FAIL error=stream-overflow\n");
            } else if (length < 8) {
                at = append_str(line, at, "PRESSURE-SERVICE FAIL error=no-header\n");
            } else {
                unsigned long payload = 0;
                for (int i = 0; i < 8; ++i) {
                    payload = (payload << 8) | buffer[i];
                }
                unsigned long long expected = 0xcbf29ce484222325ULL;
                unsigned char header[8];
                for (int i = 0; i < 8; ++i) header[i] = buffer[i];
                expected = fnv1a(expected, header, 8);
                for (unsigned long i = 0; i < payload; ++i) {
                    unsigned char byte = (unsigned char)((i * 31UL + 7UL) & 0xffUL);
                    expected = fnv1a(expected, &byte, 1);
                }
                unsigned long long actual = fnv1a(0xcbf29ce484222325ULL, buffer, length);
                if (length != 8 + payload) {
                    at = append_str(line, at, "PRESSURE-SERVICE FAIL error=short-");
                    at = append_ulong(line, at, length);
                    at = append_str(line, at, "/");
                    at = append_ulong(line, at, 8 + payload);
                    at = append_str(line, at, "\n");
                } else if (actual != expected) {
                    at = append_str(line, at,
                                    "PRESSURE-SERVICE FAIL error=hash expected=");
                    at = append_hex64(line, at, expected);
                    at = append_str(line, at, " actual=");
                    at = append_hex64(line, at, actual);
                    at = append_str(line, at, "\n");
                } else {
                    at = append_str(line, at,
                                    "PRESSURE-SERVICE PASS bytes=");
                    at = append_ulong(line, at, length);
                    at = append_str(line, at, " payload=");
                    at = append_ulong(line, at, payload);
                    at = append_str(line, at, " fnv=");
                    at = append_hex64(line, at, actual);
                    at = append_str(line, at, "\n");
                }
            }
            unsigned long off = 0;
            unsigned long len = strlen(line);
            while (off < len) {
                ssize_t n = send(control, line + off, len - off, 0);
                if (n > 0) { off += (unsigned long)n; continue; }
                if (n < 0 && errno == EAGAIN) continue;
                break;
            }
        } else if (command == 'Z') {
            break;
        } else {
            return 11;
        }
    }
    if (data >= 0) close(data);
    close(control);
    return 0;
}
