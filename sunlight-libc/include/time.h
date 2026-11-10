#ifndef SUNLIGHT_TIME_H
#define SUNLIGHT_TIME_H
/* Narrow native x86-64 LP64 slice, available with base, POSIX or GNU feature
 * macros. No POSIX version/option or CPU-time-clock support is advertised. */
typedef long time_t;
typedef long clock_t;
typedef int clockid_t;
struct timespec {
    time_t tv_sec;
    long tv_nsec;
};
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#ifdef __cplusplus
extern "C" {
#endif
/* Realtime: UTC epoch seconds (tv_nsec == 0); monotonic: boot time at 10 ms
 * granularity. No wall-clock accuracy guarantee. Success preserves errno.
 * NULL output fails EFAULT, unsupported IDs fail EINVAL. Non-NULL storage
 * must be valid and writable; arbitrary mapping faults are not recovered. */
int clock_gettime(clockid_t clock_id, struct timespec *tp);
/* Relative monotonic sleep. Negative seconds or nanoseconds outside
 * [0, 1000000000) fail EINVAL. Duration and absolute deadline must fit below
 * UINT64_MAX nanoseconds (reserved for indefinite waits), else EINVAL.
 * NULL req fails EFAULT; non-NULL req must designate readable storage.
 * Zero duration succeeds without waiting. Scheduler wakes are retried until
 * the deadline. Signal interruption/EINTR is unavailable; rem is untouched,
 * including on success and failure. No remainder storage is accessed.
 * Success returns 0 and preserves errno; failure returns -1 and sets errno. */
int nanosleep(const struct timespec *req, struct timespec *rem);
#ifdef __cplusplus
}
#endif
#endif
