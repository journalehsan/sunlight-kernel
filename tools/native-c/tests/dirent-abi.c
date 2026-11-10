#ifdef REVERSE_INCLUDES
#include <stdio.h>
#include <unistd.h>
#include <dirent.h>
#else
#include <dirent.h>
#include <unistd.h>
#include <stdio.h>
#endif
#include <dirent.h>
#include <stddef.h>
#ifdef __cplusplus
#define ASSERT(c) static_assert(c, #c)
ASSERT(__is_same(decltype(&opendir), DIR *(*)(const char *)));
ASSERT(__is_same(decltype(&readdir), struct dirent *(*)(DIR *)));
ASSERT(__is_same(decltype(&closedir), int (*)(DIR *)));
ASSERT(__is_same(decltype(dirent::d_name), char[65]));
#else
#define ASSERT(c) _Static_assert(c, #c)
ASSERT(_Generic(&opendir, DIR *(*)(const char *): 1, default: 0));
ASSERT(_Generic(&readdir, struct dirent *(*)(DIR *): 1, default: 0));
ASSERT(_Generic(&closedir, int (*)(DIR *): 1, default: 0));
ASSERT(_Generic(((struct dirent *)0)->d_name, char *: 1, default: 0));
#endif
ASSERT(sizeof(struct dirent) == 65);
ASSERT(offsetof(struct dirent, d_name) == 0);
