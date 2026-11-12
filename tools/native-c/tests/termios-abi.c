#ifdef REVERSE_INCLUDES
#include <unistd.h>
#include <errno.h>
#include <termios.h>
#else
#include <termios.h>
#include <errno.h>
#include <unistd.h>
#endif
#include <stddef.h>
#ifdef __cplusplus
#define CHECK(c) static_assert(c, #c)
CHECK(__is_same(decltype(&tcsetattr), int (*)(int,int,const struct termios *)));
CHECK(__is_same(decltype(&cfgetispeed), speed_t (*)(const struct termios *)));
#else
#define CHECK(c) _Static_assert(c, #c)
CHECK(_Generic(&tcgetattr, int (*)(int,struct termios *):1, default:0));
CHECK(_Generic(&cfsetospeed, int (*)(struct termios *,speed_t):1, default:0));
#endif
CHECK(sizeof(tcflag_t)==4);
CHECK(sizeof(cc_t)==1);
CHECK(sizeof(speed_t)==4);
CHECK(sizeof(struct termios)==28);
CHECK(__alignof__(struct termios)==4);
CHECK(offsetof(struct termios,c_iflag)==0);
CHECK(offsetof(struct termios,c_oflag)==4);
CHECK(offsetof(struct termios,c_cflag)==8);
CHECK(offsetof(struct termios,c_lflag)==12);
CHECK(offsetof(struct termios,c_cc)==16);
CHECK(offsetof(struct termios,c_ispeed)==20);
CHECK(offsetof(struct termios,c_ospeed)==24);
CHECK(NCCS==2 && VMIN==0 && VTIME==1);
