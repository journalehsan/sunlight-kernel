#include <fcntl.h>
#include <string.h>
#include <unistd.h>

static int check_file(void) {
    static const char path[] = "/tmp/" FIXTURE ".tmp";
    static const unsigned char expected[] = {0x00, 0x53, 0x4c, 0xff, 0x0a};
    unsigned char actual[sizeof expected + 1];
    int fd = open(path, O_CREAT | O_EXCL | O_WRONLY, 0600);
    if (fd < 0)
        return 0;
    int ok = write(fd, expected, sizeof expected) == sizeof expected;
    if (close(fd) != 0)
        ok = 0;
    if (ok) {
        fd = open(path, O_RDONLY);
        if (fd < 0)
            ok = 0;
        else {
            ok = read(fd, actual, sizeof expected) == sizeof expected &&
                 memcmp(actual, expected, sizeof expected) == 0 &&
                 read(fd, actual, 1) == 0;
            if (close(fd) != 0)
                ok = 0;
        }
    }
    return unlink(path) == 0 && ok;
}

#define MARK(check) do { \
    static const char line[] = FIXTURE " " check "\n"; \
    if (write(1, line, sizeof line - 1) != sizeof line - 1) return 90; \
} while (0)

/* Volatile ensures actual TLS loads on both sides of the rendezvous.
 * The atomic counter publishes both workers' writes before main releases
 * either worker. The host replay bounds failure to reach the rendezvous. */
static TLS volatile int local = 37;
static int ready, release_workers;
static int completed[2];

static void check_worker(int index) {
    const int initial_ok = local == 37;
    const int value = 100 + index;
    local = value;
    __atomic_add_fetch(&ready, 1, __ATOMIC_ACQ_REL);
    while (!__atomic_load_n(&release_workers, __ATOMIC_ACQUIRE))
        __asm__ volatile("pause");
    completed[index] = initial_ok && local == value ? value : -1;
}

static int rendezvous(void) {
    while (__atomic_load_n(&ready, __ATOMIC_ACQUIRE) != 2)
        __asm__ volatile("pause");
    int ok = local == 73;
    __atomic_store_n(&release_workers, 1, __ATOMIC_RELEASE);
    return ok;
}
