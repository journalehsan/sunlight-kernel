#ifndef SUNLIGHT_LIBC_STRING_H
#define SUNLIGHT_LIBC_STRING_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

char *strcpy(char *dest, const char *src);
size_t strcspn(const char *src, const char *reject);
char *strpbrk(const char *src, const char *accept);
char *strstr(const char *src, const char *needle);
char *strncpy(char *dest, const char *src, size_t n);

void *memcpy(void *dest, const void *src, size_t n);
void *memmove(void *dest, const void *src, size_t n);
void *memset(void *s, int c, size_t n);
int memcmp(const void *s1, const void *s2, size_t n);
void *memchr(const void *s, int c, size_t n);

/* Immutable C-locale strings; unknown numbers return "Unknown error".
 * strerror_r uses the POSIX int-returning ABI, including under _GNU_SOURCE.
 * A short buffer is NUL-terminated when its length is nonzero; returns ERANGE. */
char *strerror(int error);
int strerror_r(int error, char *buffer, size_t length);

size_t strlen(const char *s);
size_t strnlen(const char *s, size_t max_len);
int strcmp(const char *s1, const char *s2);
int strncmp(const char *s1, const char *s2, size_t n);
char *strchr(const char *s, int c);
char *strrchr(const char *s, int c);

#ifdef __cplusplus
}
#endif

#endif /* SUNLIGHT_LIBC_STRING_H */
