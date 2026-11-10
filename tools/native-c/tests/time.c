#define _GNU_SOURCE
#include <time.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* Independent native reads: syscall 81 UTC seconds, 86 monotonic milliseconds.
 * Never run this native ELF under a Linux host personality. */
static unsigned long raw_clock(unsigned long number) {
    unsigned long result;
    __asm__ volatile("syscall" : "=a"(result) : "a"(number) : "rcx", "r11", "memory");
    return result;
}
static int valid(struct timespec t) {
    return t.tv_sec >= 0 && t.tv_nsec >= 0 && t.tv_nsec < 1000000000L;
}
static unsigned long ns(struct timespec t) {
    return (unsigned long)t.tv_sec * 1000000000UL + (unsigned long)t.tv_nsec;
}
static int clocks(void) {
    struct timespec real, mono, next;
    unsigned long wall_before = raw_clock(81), boot_before = raw_clock(86);
    errno = 123;
    if (clock_gettime(CLOCK_REALTIME, &real) || !valid(real) || real.tv_nsec != 0 || errno != 123) return 10;
    if (clock_gettime(CLOCK_MONOTONIC, &mono) || !valid(mono) || mono.tv_nsec % 10000000L || errno != 123) return 11;
    unsigned long wall_after = raw_clock(81), boot_after = raw_clock(86);
    if ((unsigned long)real.tv_sec < wall_before || (unsigned long)real.tv_sec > wall_after) return 12;
    if (ns(mono) / 1000000UL < boot_before || ns(mono) / 1000000UL > boot_after) return 13;
    for (int i = 0; i < 256; ++i) {
        if (clock_gettime(CLOCK_MONOTONIC, &next) || !valid(next) || ns(next) < ns(mono)) return 14;
        mono = next;
    }
    printf("time: clocks realtime=%ld.%09ld monotonic=%ld.%09ld\n", real.tv_sec, real.tv_nsec, mono.tv_sec, mono.tv_nsec);
    return 0;
}
static int errors(void) {
    struct timespec out = {-7, -9}, rem = {-11, -13};
    const int ids[] = {-1, 2, 3, (-2147483647 - 1), 2147483647};
    for (unsigned i = 0; i < sizeof ids / sizeof ids[0]; ++i) {
        errno = 0;
        if (clock_gettime(ids[i], &out) != -1 || errno != EINVAL || out.tv_sec != -7 || out.tv_nsec != -9) return 20;
    }
    errno = 0;
    if (clock_gettime(CLOCK_REALTIME, 0) != -1 || errno != EFAULT) return 21;
    errno = 0;
    if (clock_gettime(CLOCK_MONOTONIC, 0) != -1 || errno != EFAULT) return 22;
    errno = 0;
    if (nanosleep(0, &rem) != -1 || errno != EFAULT) return 23;
    const struct timespec bad[] = {{-1, 0}, {0, -1}, {0, 1000000000L},
        {0, 9223372036854775807L}, {9223372036854775807L, 0},
        {18446744073L, 709551615L}, {18446744073L, 709551616L},
        /* Fits as a duration but adding the nonzero guest uptime overflows. */
        {18446744073L, 709551614L}};
    for (unsigned i = 0; i < sizeof bad / sizeof bad[0]; ++i) {
        errno = 0;
        if (nanosleep(&bad[i], &rem) != -1 || errno != EINVAL || rem.tv_sec != -11 || rem.tv_nsec != -13) return 24;
    }
    return 0;
}
static int zero(void) {
    struct timespec req = {0, 0}, rem = {-7, -9};
    errno = 123;
    if (nanosleep(&req, &rem) || errno != 123 || rem.tv_sec != -7 || rem.tv_nsec != -9) return 30;
    if (nanosleep(&req, 0) || errno != 123) return 31;
    if (nanosleep(&req, &req) || errno != 123 || req.tv_sec || req.tv_nsec) return 32;
    return 0;
}
static int sleeps(void) {
    const struct timespec requests[] = {{0, 1}, {0, 1000000}, {0, 25000000}, {1, 1}};
    for (unsigned i = 0; i < sizeof requests / sizeof requests[0]; ++i) {
        struct timespec before, after, req = requests[i], rem = {-7, -9};
        if (clock_gettime(CLOCK_MONOTONIC, &before)) return 40;
        errno = 123;
        /* Also exercise req/rem aliasing: completed sleep must not change it. */
        if (nanosleep(&req, i == 2 ? &req : &rem) || errno != 123) return 41;
        if (clock_gettime(CLOCK_MONOTONIC, &after)) return 42;
        unsigned long elapsed = ns(after) - ns(before), requested = ns(requests[i]);
        /* Bounds are in the native coarse clock domain; scheduling may be late.
         * They do not establish sub-tick physical or host wall-clock accuracy. */
        if (elapsed < requested || elapsed > requested + 5000000000UL) return 43;
        if (req.tv_sec != requests[i].tv_sec || req.tv_nsec != requests[i].tv_nsec || rem.tv_sec != -7 || rem.tv_nsec != -9) return 44;
        printf("time: sleep requested_ns=%lu elapsed_ns=%lu rem=unchanged\n", requested, elapsed);
    }
    return 0;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    if (!strcmp(argv[1], "clocks")) return clocks();
    if (!strcmp(argv[1], "errors")) return errors();
    if (!strcmp(argv[1], "zero")) return zero();
    if (!strcmp(argv[1], "sleep")) return sleeps();
    return 3;
}
