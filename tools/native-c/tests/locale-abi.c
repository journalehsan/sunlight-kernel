#ifdef REVERSE_INCLUDES
#include <langinfo.h>
#include <stddef.h>
#endif
#include <locale.h>
#ifndef REVERSE_INCLUDES
#include <stddef.h>
#include <langinfo.h>
#endif
#ifdef __cplusplus
template<class A, class B> struct same { static const bool value = false; };
template<class A> struct same<A, A> { static const bool value = true; };
static_assert(same<decltype(&setlocale), char *(*)(int, const char *)>::value, "setlocale ABI");
#else
_Static_assert(_Generic(&setlocale, char *(*)(int, const char *): 1, default: 0), "setlocale ABI");
#endif
char *query_locale(void) { return setlocale(LC_ALL, NULL); }
