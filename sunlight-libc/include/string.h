#ifndef SUNLIGHT_LIBC_STRING_H
#define SUNLIGHT_LIBC_STRING_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

void *memcpy(void *dest, const void *src, size_t n);
void *memmove(void *dest, const void *src, size_t n);
void *memset(void *s, int c, size_t n);
int memcmp(const void *s1, const void *s2, size_t n);
void *memchr(const void *s, int c, size_t n);

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
