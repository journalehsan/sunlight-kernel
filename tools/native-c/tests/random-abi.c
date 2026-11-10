/* Compile as C11/C++17 with each feature macro and both include orders. */
#ifdef REVERSE_INCLUDES
#include <unistd.h>
#include <sys/random.h>
#else
#include <sys/random.h>
#include <unistd.h>
#endif
#include <sys/random.h>
#include <errno.h>
#if defined(GRND_NONBLOCK) || defined(GRND_RANDOM) || defined(GRND_NONCRYPTO)
#error Unsupported flags must not be advertised by the native C header
#endif
#ifdef __cplusplus
static_assert(sizeof(size_t) == 8 && sizeof(ssize_t) == 8 && sizeof(unsigned int) == 4);
static_assert(__is_same(size_t, unsigned long));
static_assert(__is_same(ssize_t, long));
static_assert(__is_same(decltype(&getrandom), ssize_t (*)(void *, size_t, unsigned int)));
#else
_Static_assert(sizeof(size_t) == 8 && sizeof(ssize_t) == 8 && sizeof(unsigned int) == 4, "LP64");
_Static_assert(_Generic((size_t)0, unsigned long: 1, default: 0), "unsigned long size_t");
_Static_assert(_Generic((ssize_t)0, long: 1, default: 0), "signed long ssize_t");
_Static_assert(_Generic(&getrandom, ssize_t (*)(void *, size_t, unsigned int): 1, default: 0), "getrandom ABI");
#endif
