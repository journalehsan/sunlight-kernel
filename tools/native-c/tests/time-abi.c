/* Both languages, feature macros and include orders; repeat for guard checks. */
#ifdef REVERSE_INCLUDES
#include <unistd.h>
#include <sys/random.h>
#include <time.h>
#else
#include <time.h>
#include <sys/random.h>
#include <unistd.h>
#endif
#include <time.h>
#include <stddef.h>
#ifdef __cplusplus
#define ASSERT(c) static_assert(c, #c)
#define SAME(t, u) __is_same(t, u)
ASSERT(SAME(time_t, long));
ASSERT(SAME(clock_t, long));
ASSERT(SAME(clockid_t, int));
ASSERT(SAME(decltype(timespec::tv_sec), time_t));
ASSERT(SAME(decltype(timespec::tv_nsec), long));
ASSERT(SAME(decltype(&clock_gettime), int (*)(clockid_t, struct timespec *)));
ASSERT(SAME(decltype(&nanosleep), int (*)(const struct timespec *, struct timespec *)));
ASSERT(alignof(struct timespec) == 8);
#else
#define ASSERT(c) _Static_assert(c, #c)
ASSERT(_Generic((time_t)0, long: 1, default: 0));
ASSERT(_Generic((clock_t)0, long: 1, default: 0));
ASSERT(_Generic((clockid_t)0, int: 1, default: 0));
ASSERT(_Generic(((struct timespec){0}).tv_sec, time_t: 1, default: 0));
ASSERT(_Generic(((struct timespec){0}).tv_nsec, long: 1, default: 0));
ASSERT(_Generic(&clock_gettime, int (*)(clockid_t, struct timespec *): 1, default: 0));
ASSERT(_Generic(&nanosleep, int (*)(const struct timespec *, struct timespec *): 1, default: 0));
ASSERT(_Alignof(struct timespec) == 8);
#endif
ASSERT(sizeof(time_t) == 8 && sizeof(clock_t) == 8 && sizeof(clockid_t) == 4);
ASSERT((time_t)-1 < 0 && (clock_t)-1 < 0 && (clockid_t)-1 < 0);
ASSERT(sizeof(struct timespec) == 16);
ASSERT(offsetof(struct timespec, tv_sec) == 0);
ASSERT(offsetof(struct timespec, tv_nsec) == 8);
ASSERT(CLOCK_REALTIME == 0 && CLOCK_MONOTONIC == 1);
