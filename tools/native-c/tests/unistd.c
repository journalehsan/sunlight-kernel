/* Focused Phase 5 guest contracts. Host execution is not native evidence. */
#include <unistd.h>
#include <stdlib.h>
#include <stdio.h>
#include <errno.h>
#include <string.h>
#include <stdint.h>
#include <sunlight_runtime.h>

_Static_assert(sizeof(int) == 4 && sizeof(size_t) == 8, "native LP64");
_Static_assert(sizeof(ssize_t) == 8 && (ssize_t)-1 < 0, "signed native count");
static ssize_t (*const signature)(int, const void *, size_t) = write;

static void a(void) { if (write(1, "A", 1) != 1) sl_std_exit(80); }
static void b(void) { if (write(1, "B", 1) != 1) sl_std_exit(81); }
static void c(void) { if (write(1, "C", 1) != 1) sl_std_exit(82); }
static void register_c(void) { b(); if (atexit(c)) sl_std_exit(83); }
static void x(void) { if (write(1, "x", 1) != 1) sl_std_exit(84); }

int main(int argc, char **argv) {
    if (argc != 2) return 60;
    if (!strcmp(argv[1], "basic")) {
        errno = 77;
        if (signature(STDOUT_FILENO, "write OK\n", 9) != 9 || errno != 77) return 1;
        if (write(STDERR_FILENO, "stderr OK\n", 10) != 10 || errno != 77) return 2;
        return 0;
    }
    if (!strcmp(argv[1], "zero")) {
        errno = 77;
        if (write(1, NULL, 0) != 0 || errno != 77) return 3;
        /* Native zero-count writes do not check nonnegative fd existence. */
        if (write(4095, NULL, 0) != 0 || errno != 77) return 4;
        if (write(-1, NULL, 0) != -1 || errno != EBADF) return 5;
        return 0;
    }
    if (!strcmp(argv[1], "short")) {
        FILE *f = tmpfile();
        if (!f) return 6;
        static char bytes[8192];
        memset(bytes, 's', sizeof bytes);
        errno = 77;
        /* Actual native VFS writes copy at most 4096 bytes per syscall. */
        if (write(fileno(f), bytes, sizeof bytes) != 4096 || errno != 77) return 7;
        if (ftell(f) != 4096) return 8;
        rewind(f);
        memset(bytes, 0, sizeof bytes);
        if (fread(bytes, 1, sizeof bytes, f) != 4096 || !feof(f) || ferror(f)) return 9;
        for (int i = 0; i < 4096; ++i) if (bytes[i] != 's') return 10;
        if (bytes[4096] != 0 || fgetc(f) != EOF) return 11;
        return fclose(f) ? 12 : 0;
    }
    if (!strcmp(argv[1], "errors")) {
        if (write(-1, "x", 1) != -1 || errno != EBADF) return 13;
        if (write(4095, "x", 1) != -1 || errno != EBADF) return 14;
        if (write(0, "x", 1) != -1 || errno != EACCES) return 15;
        if (write(1, NULL, 1) != -1 || errno != EFAULT) return 16;
        if (write(1, "x", SIZE_MAX) != -1 || errno != EINVAL) return 17;
        /* Kernel copy validates this non-NULL unmapped address; the native
         * ABI currently encodes user-copy failure generically as EIO. */
        if (write(1, (const void *)(uintptr_t)1, 1) != -1 || errno != EIO) return 18;
        return 0;
    }
    if (!strcmp(argv[1], "order")) {
        if (atexit(a) || atexit(b) || atexit(c)) return 19;
        return 0; /* Expected stdout CBA on return from main. */
    }
    if (!strcmp(argv[1], "register")) {
        if (atexit(a) || atexit(register_c)) return 20;
        return 0; /* Callback registration must run unlocked: BCA. */
    }
    if (!strcmp(argv[1], "capacity")) {
        if (atexit(NULL) == 0 || errno != EINVAL) return 21;
        errno = 77;
        for (int i = 0; i < 32; ++i) if (atexit(x) || errno != 77) return 22;
        if (atexit(a) == 0 || errno != ENOMEM) return 23;
        return 0; /* Exactly 32 x's; failed registrations must not run. */
    }
    if (!strcmp(argv[1], "exit")) {
        /* Leave a live anonymous FILE for the actual group-exit reaper. */
        FILE *f = tmpfile();
        if (!f || fputs("cleanup", f) < 0 || atexit(a)) return 24;
        exit(27); /* Expected stdout A, process status 27. */
    }
    if (!strcmp(argv[1], "raw-exit")) {
        if (atexit(a)) return 25;
        sl_std_exit(29); /* Existing raw runtime ABI bypasses callbacks. */
    }
    return 61;
}
