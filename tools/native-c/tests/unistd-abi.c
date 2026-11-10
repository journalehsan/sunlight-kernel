/* Compile with C11 and C++17, each supported feature macro, both include orders. */
#ifdef REVERSE_INCLUDES
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#else
#include <unistd.h>
#include <stdlib.h>
#include <stdio.h>
#endif
#include <errno.h>
#ifdef _POSIX_VERSION
#error This foundation must not advertise a complete POSIX version
#endif
#ifdef __cplusplus
static_assert(sizeof(int) == 4 && sizeof(size_t) == 8 && sizeof(ssize_t) == 8);
static_assert((ssize_t)-1 < 0);
static_assert(__is_same(decltype(&write), ssize_t (*)(int, const void *, size_t)));
static_assert(__is_same(decltype(&atexit), int (*)(void (*)(void))));
static_assert(__is_same(decltype(&exit), void (*)(int)));
#else
_Static_assert(sizeof(int) == 4 && sizeof(size_t) == 8 && sizeof(ssize_t) == 8, "LP64");
_Static_assert((ssize_t)-1 < 0, "signed ssize_t");
_Static_assert(_Generic(&write, ssize_t (*)(int, const void *, size_t): 1, default: 0), "write ABI");
_Static_assert(_Generic(&atexit, int (*)(void (*)(void)): 1, default: 0), "atexit ABI");
_Static_assert(_Generic(&exit, void (*)(int): 1, default: 0), "exit ABI");
#endif
