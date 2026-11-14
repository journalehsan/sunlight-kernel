#include <unistd.h>
#include <langinfo.h>
#include <unistd.h>
#include <langinfo.h>
#ifdef __cplusplus
static_assert(__is_same(decltype(&getpid),pid_t (*)(void)),"getpid ABI");
static_assert(__is_same(decltype(&getppid),pid_t (*)(void)),"getppid ABI");
static_assert(__is_same(decltype(&gettid),pid_t (*)(void)),"gettid ABI");
static_assert(__is_same(decltype(&nl_langinfo),char *(*)(nl_item)),"langinfo ABI");
#else
_Static_assert(_Generic(&getpid,pid_t (*)(void):1,default:0),"getpid ABI");
_Static_assert(_Generic(&getppid,pid_t (*)(void):1,default:0),"getppid ABI");
_Static_assert(_Generic(&gettid,pid_t (*)(void):1,default:0),"gettid ABI");
_Static_assert(_Generic(&nl_langinfo,char *(*)(nl_item):1,default:0),"langinfo ABI");
#endif
