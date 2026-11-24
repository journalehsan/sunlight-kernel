#include <stdint.h>

enum { WIDTH = 256, HEIGHT = 160, STRIDE = WIDTH * 4 + 16,
       OFFSET = 4096 + 13, POOL = OFFSET + STRIDE * HEIGHT };

static const struct {
    uint16_t family;
    char path[38];
} address = {1, "/sunlight/native-unix/wayland-fixture"};

static long syscall6(long number, long arg0, long arg1, long arg2,
                     long arg3, long arg4, long arg5)
{
    register long r10 __asm__("r10") = arg3;
    register long r8 __asm__("r8") = arg4;
    register long r9 __asm__("r9") = arg5;
    __asm__ volatile("syscall" : "+a"(number)
                     : "D"(arg0), "S"(arg1), "d"(arg2), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return number;
}
#define call(number, arg0, arg1, arg2) syscall6(number, arg0, arg1, arg2, 0, 0, 0)
#define native(number) ((long)((1UL << 63) | (number)))

static void require(int ok, const char *message)
{
    if (ok) return;
    long len = 0;
    while (message[len]) ++len;
    call(native(99), (long)message, len, 0);
    call(60, 1, 0, 0);
    __builtin_trap();
}

static void hold(unsigned ms)
{
    long start = call(native(86), 0, 0, 0);
    while (call(native(86), 0, 0, 0) - start < (long)ms)
        call(24, 0, 0, 0);
}

static long stream;
static long io_deadline;

static long capture_native(long number, long arg0, long arg1)
{
    return call(native(number), arg0, arg1, 0);
}

static void capture_log(const char *message)
{
    long length = 0;
    while (message[length]) ++length;
    capture_native(99, (long)message, length);
}

static void capture_pause(void)
{
    call(24, 0, 0, 0);
}

#include "helios-capture-ack.h"

static void transfer(const void *data, unsigned len, int writing)
{
    const unsigned char *cursor = data;
    while (len) {
        require(capture_native(86, 0, 0) < io_deadline,
                "[WL-CLIENT] FAIL Wayland transfer deadline\n");
        long count = call(writing, stream, (long)cursor, len);
        if (count == -11) {
            call(24, 0, 0, 0);
            continue;
        }
        require(count > 0 && count <= len, "[WL-CLIENT] FAIL stream I/O");
        cursor += count;
        len -= (unsigned)count;
    }
}

#define header(len, op) (((len) << 16) | (op))
#define request(id, op, ...) do { \
    const uint32_t packet[] = {id, 0, __VA_ARGS__}; \
    uint32_t wire[] = {id, header(sizeof packet, op), __VA_ARGS__}; \
    transfer(wire, sizeof wire, 1); \
} while (0)

static unsigned event(uint32_t *words)
{
    transfer(words, 8, 0);
    unsigned len = words[1] >> 16;
    require(len >= 8 && len <= 64 && len % 4 == 0, "[WL-CLIENT] FAIL event length");
    transfer(words + 2, len - 8, 0);
    return len;
}

static void expect(uint32_t id, uint32_t op, unsigned len, uint32_t argument)
{
    uint32_t words[16];
    require(event(words) == len && words[0] == id && (words[1] & 65535) == op &&
            (len < 12 || words[2] == argument), "[WL-CLIENT] FAIL unexpected event");
}

static void bind_globals(void)
{
    static const char *const interfaces[] = {"wl_compositor", "wl_shm", "xdg_wm_base"};
    unsigned seen = 0;
    request(1, 1, 2);
    for (unsigned index = 0; index < 3; ++index) {
        uint32_t words[16];
        unsigned len = event(words);
        unsigned name = words[2];
        require(len >= 24 && words[0] == 2 && (words[1] & 65535) == 0 &&
                name >= 1 && name <= 3 && !(seen & (1U << name)),
                "[WL-CLIENT] FAIL registry global");
        seen |= 1U << name;
        const char *interface = interfaces[name - 1];
        const char *wire = (const char *)(words + 4);
        unsigned count = 0;
        while (interface[count]) ++count;
        unsigned padded = (count + 4) & ~3U;
        require(words[3] == count + 1 && len == 20 + padded &&
                words[4 + padded / 4] == 1, "[WL-CLIENT] FAIL interface version");
        for (unsigned byte = 0; byte <= count; ++byte)
            require(wire[byte] == interface[byte], "[WL-CLIENT] FAIL interface name");
        words[1] = header(len + 4, 0);
        words[5 + padded / 4] = name + 2;
        transfer(words, len + 4, 1);
    }
    expect(4, 0, 12, 0);
    expect(4, 0, 12, 1);
}

static void create_pool(int fd)
{
    uint32_t packet[] = {4, header(16, 0), 9, POOL};
    unsigned char *bytes = (unsigned char *)packet;
    struct { const void *base; unsigned long len; } iov = {bytes + 5, 1};
    struct { unsigned long len; int level, type, fd, padding; } control = {20, 1, 1, fd, 0};
    struct {
        void *name;
        unsigned name_len;
        const void *iov;
        unsigned long iov_len;
        const void *control;
        unsigned long control_len;
        int flags;
    } msg = {0, 0, &iov, 1, &control, sizeof control, 0};
    transfer(bytes, 5, 1);
    hold(30);
    require(call(46, stream, (long)&msg, 0x4000) == 1, "[WL-CLIENT] FAIL sendmsg");
    hold(30);
    transfer(bytes + 6, sizeof packet - 6, 1);
}

static void paint(volatile unsigned char *map, unsigned frame)
{
    for (unsigned row = 0; row < HEIGHT; ++row) {
        for (unsigned col = 0; col < WIDTH; ++col) {
            unsigned stripe = (frame == 1 ? col : row) / 16;
            uint32_t color = frame == 1
                ? (stripe % 2 ? 0x125dd4 : 0x28dcff)
                : (stripe % 2 ? 0xd42409 : 0xffcb32);
            if (col >= 104 && col < 152 && row >= 60 && row < 100)
                color = 0xffffff;
            for (unsigned byte = 0; byte < 4; ++byte)
                map[OFFSET + row * STRIDE + col * 4 + byte] = (unsigned char)(color >> (byte * 8));
        }
    }
}

__attribute__((force_align_arg_pointer, noreturn))
void _start(void)
{
    stream = call(41, 1, 1, 0);
    require(stream >= 0, "[WL-CLIENT] FAIL socket");
    long status;
    long started = call(native(86), 0, 0, 0);
    do {
        status = call(42, stream, (long)&address, sizeof address);
        if (status == 0) break;
        call(24, 0, 0, 0);
    } while (call(native(86), 0, 0, 0) - started < 15000);
    require(status == 0, "[WL-CLIENT] FAIL connect");
    io_deadline = capture_native(86, 0, 0) + 20000;
    bind_globals();
    request(3, 0, 6);
    request(5, 2, 7, 6);
    request(7, 1, 8);
    request(6, 6);
    expect(8, 0, 20, WIDTH);
    expect(7, 0, 12, 1);
    request(7, 4, 1);

    long fd = call(319, (long)"wl-fixture", 1, 0);
    require(fd >= 0 && call(77, fd, POOL, 0) == 0, "[WL-CLIENT] FAIL memfd size");
    long addr = syscall6(9, 0, POOL, 3, 1, fd, 0);
    require(addr > 0, "[WL-CLIENT] FAIL shared map");
    volatile unsigned char *map = (volatile unsigned char *)addr;
    create_pool((int)fd);
    require(call(3, fd, 0, 0) == 0, "[WL-CLIENT] FAIL close memfd");

    request(9, 0, 10, POOL - 4, WIDTH, HEIGHT, STRIDE, 1);
    request(9, 0, 11, OFFSET, WIDTH, HEIGHT, STRIDE, 1);
    request(9, 0, 12, OFFSET, WIDTH, HEIGHT, STRIDE, 1);
    request(6, 1, 10, 0, 0);
    request(6, 6);
    expect(10, 0, 8, 0);

    for (unsigned frame = 1; frame <= 2; ++frame) {
        io_deadline = capture_native(86, 0, 0) + 20000;
        paint(map, frame);
        request(6, 1, 10 + frame, 0, 0);
        request(6, 2, 0, 0, WIDTH, HEIGHT);
        request(6, 3, 12 + frame);
        request(6, 6);
        expect(10 + frame, 0, 8, 0);
        uint32_t callback[16];
        require(event(callback) == 12 && callback[0] == 12 + frame &&
                (callback[1] & 65535) == 0, "[WL-CLIENT] FAIL frame callback");
        expect(1, 1, 12, 12 + frame);
        capture_ack(frame);
    }
    require(call(11, addr, POOL, 0) == 0 && call(3, stream, 0, 0) == 0,
            "[WL-CLIENT] FAIL disconnect");
    capture_ack(3);
    call(60, 0, 0, 0);
    __builtin_trap();
}
