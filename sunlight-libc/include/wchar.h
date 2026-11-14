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
size_t wcslen(const wchar_t *s);
wchar_t *wcscpy(wchar_t *destination, const wchar_t *source);
wchar_t *wcsncpy(wchar_t *destination, const wchar_t *source, size_t count);
int wcscmp(const wchar_t *a, const wchar_t *b);
int wcsncmp(const wchar_t *a, const wchar_t *b, size_t count);
wchar_t *wcschr(const wchar_t *s, wchar_t ch);
wchar_t *wcsrchr(const wchar_t *s, wchar_t ch);
wchar_t *wmemchr(const wchar_t *s, wchar_t ch, size_t count);
wchar_t *wcstok(wchar_t *s, const wchar_t *delimiters, wchar_t **save);
long wcstol(const wchar_t *s, wchar_t **end, int base);
int mbsinit(const mbstate_t *ps);
size_t mbrtowc(wchar_t *out, const char *s, size_t n, mbstate_t *ps);
size_t mbrlen(const char *s, size_t n, mbstate_t *ps);
#ifdef __cplusplus
}
#endif
#endif
