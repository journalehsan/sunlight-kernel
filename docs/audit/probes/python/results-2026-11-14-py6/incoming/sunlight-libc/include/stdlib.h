#ifndef SUNLIGHT_STDLIB_H
#define SUNLIGHT_STDLIB_H
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
size_t __sunlight_mb_cur_max(void);
#define MB_CUR_MAX (__sunlight_mb_cur_max())
/* Creates a mode-0600 read/write file. Replace six trailing X characters in
 * a writable NUL-terminated template. Caller must close and unlink the file. */
int mkstemp(char *path_template);
char *getenv(const char *name);
/* Private per-call conversion state; NULL destination counts all input.
 * ASCII in C, strict UTF-8 in installed UTF-8 locales. No partial encoding. */
size_t mbstowcs(wchar_t *destination, const char *source, size_t count);
size_t wcstombs(char *destination, const wchar_t *source, size_t count);
unsigned long strtoul(const char *s, char **end, int base);
long strtol(const char *s, char **end, int base);
void qsort(void *base, size_t count, size_t size,
           int (*compare)(const void *, const void *));
void *malloc(size_t size);
void *calloc(size_t count, size_t size);
void *realloc(void *ptr, size_t size);
void free(void *ptr);
/* 32 simultaneous registrations; reverse order, including registrations made
 * by a callback. Failure returns nonzero and sets errno (ENOMEM when full).
 * C main return and exit run callbacks. The raw sl_std_exit ABI bypasses them.
 * Streams are currently unbuffered; tmpfile names are already unlinked. */
int atexit(void (*function)(void));
#ifdef __cplusplus
[[noreturn]] void exit(int status);
[[noreturn]] void _Exit(int status);
#else
_Noreturn void exit(int status);
_Noreturn void _Exit(int status);
#endif
#ifdef __cplusplus
}
#endif
#endif
