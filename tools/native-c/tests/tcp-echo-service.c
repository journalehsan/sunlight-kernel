/* Bounded native TCP echo service over the real `net` transport.
 *
 * Wire format: 4-byte big-endian length followed by that many payload bytes.
 * A zero-length frame terminates the session. The service closes its listener
 * immediately after accept so the client can probe the closed-port failure
 * path, then echoes every frame until EOF or the terminator.
 */
#include <errno.h>
#include <netinet/in.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

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

int main(int argc, char **argv) {
    unsigned short port = 4040;
    if (argc > 1 && parse_port(argv[1], &port) != 0) return 1;

    int listener = socket(AF_INET, SOCK_STREAM, 0);
    if (listener < 0) return 2;

    struct sockaddr_in local;
    memset(&local, 0, sizeof local);
    local.sin_family = AF_INET;
    local.sin_port = htons(port);
    local.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    /* Bounded readiness handshake: the `net` service publishes its handlers and
     * interface asynchronously, so a transient EIO/ENOTCONN is retried until the
     * deadline. Permanent address errors fail immediately. */
    long deadline = now_ms() + 15000;
    for (;;) {
        if (bind(listener, (struct sockaddr *)&local, sizeof local) == 0) break;
        int error = errno;
        if (error == EADDRINUSE || error == EAFNOSUPPORT || error == EINVAL) {
            errno = error;
            return 3;
        }
        if (now_ms() >= deadline) {
            errno = error;
            return 3;
        }
        sleep_ms(20);
    }
    for (;;) {
        if (listen(listener, 1) == 0) break;
        int error = errno;
        if (error == EINVAL || now_ms() >= deadline) {
            errno = error;
            return 4;
        }
        sleep_ms(20);
    }

    int client = accept(listener, 0, 0);
    if (client < 0) return 5;
    /* Free the port before echoing so a later connect is refused. */
    close(listener);

    unsigned char header[4];
    unsigned char payload[1028];
    for (;;) {
        int status = read_exact(client, header, 4);
        if (status <= 0) break;
        unsigned long len = be32(header);
        if (len == 0) break;
        if (len > sizeof payload) {
            close(client);
            return 6;
        }
        if (read_exact(client, payload, len) <= 0) break;
        if (write_all(client, header, 4) != 0) break;
        if (write_all(client, payload, len) != 0) break;
    }
    close(client);
    return 0;
}
