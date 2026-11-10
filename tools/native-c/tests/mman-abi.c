#ifdef REVERSE_INCLUDES
#include <stdio.h>
#include <unistd.h>
#include <sys/mman.h>
#else
#include <sys/mman.h>
#include <unistd.h>
#include <stdio.h>
#endif
#include <sys/mman.h>
#ifdef MAP_NATIVE_THREAD
#error internal native thread flag leaked
#endif
#ifdef __cplusplus
#define ASSERT(c) static_assert(c, #c)
ASSERT(__is_same(off_t, long));
ASSERT(__is_same(decltype(&mmap), void *(*)(void *, size_t, int, int, int, off_t)));
ASSERT(__is_same(decltype(&munmap), int (*)(void *, size_t)));
ASSERT(__is_same(decltype(&mprotect), int (*)(void *, size_t, int)));
#else
#define ASSERT(c) _Static_assert(c, #c)
ASSERT(_Generic((off_t)0, long: 1, default: 0));
ASSERT(_Generic(&mmap, void *(*)(void *, size_t, int, int, int, off_t): 1, default: 0));
ASSERT(_Generic(&munmap, int (*)(void *, size_t): 1, default: 0));
ASSERT(_Generic(&mprotect, int (*)(void *, size_t, int): 1, default: 0));
#endif
ASSERT(sizeof(off_t) == 8 && (off_t)-1 < 0 && sizeof(size_t) == 8);
ASSERT(PROT_NONE == 0 && PROT_READ == 1 && PROT_WRITE == 2 && PROT_EXEC == 4);
ASSERT(MAP_SHARED == 1 && MAP_PRIVATE == 2 && MAP_FIXED == 16 && MAP_ANONYMOUS == 32);
int map_failed_is_minus_one(void) { return MAP_FAILED == (void *)-1; }
