#ifndef SUNLIGHT_LIMITS_H
#define SUNLIGHT_LIMITS_H
#include_next <limits.h>
/* Maximum over all installed native encodings, not the current locale. */
#undef MB_LEN_MAX
#define MB_LEN_MAX 4
#endif
