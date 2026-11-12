#ifndef SUNLIGHT_WCHAR_H
#define SUNLIGHT_WCHAR_H
#include <stddef.h>
#include <stdint.h>
typedef unsigned int wint_t;
#define WEOF ((wint_t)-1)
/* Native source ABI, opaque conversion state; initialize with {0}. */
typedef struct { unsigned int __opaque[2]; } mbstate_t;
#ifdef __cplusplus
extern "C" {
#endif
/* C locale: ASCII. C.UTF-8 and en_US.UTF-8: strict Unicode UTF-8.
 * ps == NULL uses a shared, serialized state, separately for each API.
 * Use explicit states for independent streams and concurrent conversions.
 * After EILSEQ reset the state before reuse. Locale changes require initial
 * states and external serialization against conversion calls. */
int mbsinit(const mbstate_t *ps);
size_t mbrtowc(wchar_t *out, const char *s, size_t n, mbstate_t *ps);
size_t mbrlen(const char *s, size_t n, mbstate_t *ps);
#ifdef __cplusplus
}
#endif
#endif
