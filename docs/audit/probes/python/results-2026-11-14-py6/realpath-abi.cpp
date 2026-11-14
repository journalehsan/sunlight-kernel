#include <stdlib.h>
#include <limits.h>
#include <errno.h>
#include <stdint.h>
#include <stddef.h>
static_assert(PATH_MAX == 256, "native buffer contract");
static_assert(sizeof(size_t)==8 && sizeof(void*)==8, "native widths");
char *(*canonical_query)(const char *, char *) = &realpath;
