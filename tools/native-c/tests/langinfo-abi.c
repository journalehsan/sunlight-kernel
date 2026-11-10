#ifdef REVERSE_INCLUDES
#include <string.h>
#include <errno.h>
#include <langinfo.h>
#else
#include <langinfo.h>
#include <errno.h>
#include <string.h>
#endif
#include <langinfo.h>
#ifdef __cplusplus
static_assert(__is_same(nl_item, int), "native nl_item");
static_assert(__is_same(decltype(&nl_langinfo), char *(*)(nl_item)), "C ABI");
#else
_Static_assert(_Generic((nl_item)0, int: 1, default: 0), "native nl_item");
_Static_assert(_Generic(&nl_langinfo, char *(*)(nl_item): 1, default: 0), "C ABI");
#endif
