#ifndef SUNLIGHT_BITS_TIME_H
#define SUNLIGHT_BITS_TIME_H
/* Shared native LP64 clock ABI. long is exactly the time_t declared by
 * sys/types.h. Keep this header independent of compiler resource headers so
 * pthread.h remains usable by the existing bare C++ ABI probes. */
typedef int clockid_t;
struct timespec {
    long tv_sec;
    long tv_nsec;
};
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#endif
