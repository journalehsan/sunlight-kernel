/* Scripted peer for the native TCP poll acceptance fixture.
 *
 * The service listens on 127.0.0.1:4040. The client owns the data listeners
 * on 4051/4052 and explicitly commands each connection, so listener readiness
 * is observed before any peer connects. Commands arrive one byte at a time on
 * the control connection.
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

static void fill_sockaddr(struct sockaddr_in *addr, unsigned short port) {
    memset(addr, 0, sizeof *addr);
    addr->sin_family = AF_INET;
    addr->sin_port = htons(port);
    addr->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
}

static int send_all(int fd, const unsigned char *buf, unsigned long len) {
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

static int recv_byte(int fd, unsigned char *byte) {
    for (;;) {
        ssize_t n = recv(fd, byte, 1, 0);
        if (n == 1) return 1;
        if (n == 0) return 0;
        if (errno != EAGAIN) return -1;
    }
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

int main(void) {
    int control_listener = socket(AF_INET, SOCK_STREAM, 0);
    if (control_listener < 0) return 1;
    struct sockaddr_in local;
    fill_sockaddr(&local, 4040);
    long deadline = now_ms() + 15000;
    for (;;) {
        if (bind(control_listener, (struct sockaddr *)&local, sizeof local) == 0) break;
        if (now_ms() >= deadline) return 2;
        sleep_ms(20);
    }
    if (listen(control_listener, 1) != 0) return 3;
    int control = accept(control_listener, 0, 0);
    if (control < 0) return 4;
    close(control_listener);

    int data1 = -1;
    int data2 = -1;
    for (;;) {
        unsigned char command = 0;
        int received = recv_byte(control, &command);
        if (received <= 0) break;
        switch (command) {
        case 'C':
            if (data1 >= 0) return 5;
            data1 = connect_port(4051, now_ms() + 15000);
            if (data1 < 0) return 6;
            break;
        case 'D':
            if (data2 >= 0) return 7;
            data2 = connect_port(4052, now_ms() + 15000);
            if (data2 < 0) return 8;
            break;
        case 'S':
            if (data1 < 0 || send_all(data1, (const unsigned char *)"HELLO", 5) != 0)
                return 9;
            break;
        case 'M':
            if (data1 < 0 || send_all(data1, (const unsigned char *)"WORLD", 5) != 0)
                return 10;
            break;
        case 'N':
            if (data1 < 0 || send_all(data1, (const unsigned char *)"x", 1) != 0)
                return 11;
            break;
        case 'F':
            if (data1 < 0 ||
                send_all(data1, (const unsigned char *)"ABCDEFGH", 8) != 0)
                return 12;
            close(data1);
            data1 = -1;
            break;
        case 'E':
            if (data2 < 0 || send_all(data2, (const unsigned char *)"Y", 1) != 0)
                return 13;
            break;
        default:
            return 14;
        }
    }
    if (data1 >= 0) close(data1);
    if (data2 >= 0) close(data2);
    close(control);
    return 0;
}
