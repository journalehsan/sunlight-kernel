#include <sys/mman.h>
#include <errno.h>
#include <stdint.h>
#include <string.h>

/* Native only. Direct calls bypass libc validation to exercise kernel bounds
 * and independently ask the kernel to copy a clock result into the mapping. */
static unsigned long raw(unsigned long n, unsigned long a, unsigned long b,
                         unsigned long c, unsigned long d) {
    unsigned long r;
    register unsigned long r10 __asm__("r10") = d;
    register unsigned long r8 __asm__("r8") = (unsigned long)-1;
    register unsigned long r9 __asm__("r9") = 0;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c),
                     "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return r;
}
#define FLAGS (MAP_PRIVATE | MAP_ANONYMOUS)
static int subset(void) {
    errno = 123;
    volatile unsigned char *p = mmap(0, 1048576, PROT_READ | PROT_WRITE, FLAGS, -1, 0);
    if ((void *)p == MAP_FAILED || errno != 123) return 10;
    for (unsigned i = 0; i < 1048576; ++i) if (p[i]) return 11;
    p[0] = 17; p[1048575] = 29;
    if (mprotect((void *)p, 1048576, PROT_READ) || errno != 123) return 12;
    /* syscall 88 validates writable user PTEs before copying its timespec. */
    if (raw(88, 1, (unsigned long)p, 0, 0) != (unsigned long)-1 || p[0] != 17) return 13;
    if (mprotect((void *)p, 1048576, PROT_READ | PROT_WRITE) || errno != 123) return 14;
    if (raw(88, 1, (unsigned long)p, 0, 0)) return 15;
    p[1048575] = 31;
    if (munmap((void *)p, 1048576) || errno != 123) return 16;
    return 0;
}
static int fixed(void) {
    volatile unsigned char *p = mmap(0, 12288, PROT_READ | PROT_WRITE, FLAGS, -1, 0);
    if ((void *)p == MAP_FAILED) return 20;
    p[0] = 11; p[4096] = 22; p[8192] = 33;
    errno = 123;
    void *q = mmap((void *)(p + 4096), 4096, PROT_READ | PROT_WRITE, FLAGS | MAP_FIXED, -1, 0);
    if (q != (void *)(p + 4096) || errno != 123) return 21;
    if (p[0] != 11 || p[4096] != 0 || p[8192] != 33) return 22;
    p[4096] = 44;
    if (munmap((void *)p, 12288)) return 23;
    return 0;
}
static int bounds(void) {
    volatile unsigned char *p = mmap(0, 4096, PROT_READ | PROT_WRITE, FLAGS, -1, 0);
    if ((void *)p == MAP_FAILED) return 30;
    p[0] = 77;
    /* Include a valid first page but an invalid end; no withdrawal or
     * permission change may occur before the full range is rejected. */
    unsigned long sizes[] = {~0UL, 0x800000000000UL - (unsigned long)p + 4096};
    for (unsigned i = 0; i < 2; ++i) {
        if (raw(50, (unsigned long)p, sizes[i], PROT_READ, FLAGS | MAP_FIXED) != ~0UL) return 31;
        if (p[0] != 77) return 32;
        if (raw(52, (unsigned long)p, sizes[i], PROT_READ, 0) != ~0UL) return 33;
        p[0] = 78; p[0] = 77;
        if (raw(51, (unsigned long)p, sizes[i], 0, 0) != ~0UL || p[0] != 77) return 34;
    }
    const int bad_prot[] = {PROT_NONE, PROT_WRITE, PROT_EXEC, PROT_READ | PROT_WRITE | PROT_EXEC, 8, -1};
    for (unsigned i = 0; i < sizeof bad_prot / sizeof bad_prot[0]; ++i) {
        errno = 0;
        if (mmap(0, 4096, bad_prot[i], FLAGS, -1, 0) != MAP_FAILED || errno != EINVAL) return 35;
        errno = 0;
        if (mprotect((void *)p, 4096, bad_prot[i]) != -1 || errno != EINVAL) return 36;
        if (p[0] != 77) return 37;
    }
    if (mmap(0, 4096, PROT_READ, MAP_SHARED | MAP_ANONYMOUS, -1, 0) != MAP_FAILED) return 38;
    if (mmap(0, 4096, PROT_READ, FLAGS, 0, 0) != MAP_FAILED) return 39;
    if (mmap(0, 4096, PROT_READ, FLAGS, -1, 4096) != MAP_FAILED) return 40;
    if (mmap((void *)p, 4096, PROT_READ, FLAGS, -1, 0) != MAP_FAILED) return 41;
    if (mmap((void *)(p + 1), 4096, PROT_READ, FLAGS | MAP_FIXED, -1, 0) != MAP_FAILED) return 42;
    if (mprotect((void *)(p + 1), 4096, PROT_READ) != -1) return 43;
    if (munmap((void *)p, 4096)) return 44;
    return 0;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    if (!strcmp(argv[1], "subset")) return subset();
    if (!strcmp(argv[1], "fixed")) return fixed();
    if (!strcmp(argv[1], "bounds")) return bounds();
    return 3;
}
