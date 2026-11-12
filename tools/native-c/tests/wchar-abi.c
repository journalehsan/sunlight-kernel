#ifdef REVERSE_INCLUDES
#include <stdint.h>
#include <stddef.h>
#include <limits.h>
#endif
#include <wchar.h>
#include <wchar.h>
#include <stdint.h>
#include <stddef.h>
#include <limits.h>
#ifdef __cplusplus
#define ASSERT static_assert
#define ALIGN alignof
#else
#define ASSERT _Static_assert
#define ALIGN _Alignof
#endif
ASSERT(sizeof(wchar_t)==4,"wchar size");
ASSERT((wchar_t)-1<0,"wchar signed");
ASSERT(sizeof(wchar_t)==sizeof(__WCHAR_TYPE__),"compiler wchar");
ASSERT(WCHAR_MIN==(-2147483647-1) && WCHAR_MAX==2147483647,"wide limits");
ASSERT(sizeof(wint_t)==4 && (wint_t)-1>0,"wint");
ASSERT(WEOF>0x10ffff,"sentinel");
ASSERT(sizeof(mbstate_t)==8 && ALIGN(mbstate_t)==4,"state ABI");
ASSERT(MB_LEN_MAX==4,"encoding maximum");
int main(void) { mbstate_t st={0}; return !mbsinit(&st); }
