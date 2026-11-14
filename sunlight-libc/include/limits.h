#ifndef SUNLIGHT_LIMITS_H
#define SUNLIGHT_LIMITS_H
#include_next <limits.h>
/* Maximum over all installed native encodings, not the current locale. */
#undef MB_LEN_MAX
#define MB_LEN_MAX 4
/* Native ssize_t is signed long in every supported feature profile. */
#define SSIZE_MAX LONG_MAX
#define PATH_MAX 256
#define PTHREAD_KEYS_MAX 128
#define PTHREAD_DESTRUCTOR_ITERATIONS 4
#endif
