#ifndef SUNLIGHT_TERMIOS_H
#define SUNLIGHT_TERMIOS_H
/* Native source ABI, not Linux wire compatibility. No terminal mode is
 * configurable yet. Attribute/queue APIs reject terminal handles with ENOTSUP,
 * other open descriptors with ENOTTY, missing descriptors with EBADF.
 * The object and speed helpers are usable independently of a terminal.
 * Speed getters require valid objects; setters also diagnose NULL with EFAULT.
 * Descriptor APIs do not access the supplied object on their failure paths.
 * No hardware rate, hangup, canonical, echo, signal or raw-mode claim is made. */
typedef unsigned int tcflag_t;
typedef unsigned char cc_t;
typedef unsigned int speed_t;
#define NCCS 2
#define VMIN 0
#define VTIME 1
struct termios {
    tcflag_t c_iflag, c_oflag, c_cflag, c_lflag;
    cc_t c_cc[NCCS];
    unsigned char __reserved[2];
    speed_t c_ispeed, c_ospeed;
};
/* Deliberately exposed requests; currently rejected when applying settings. */
#define ICANON 1U
#define ECHO 2U
#define ISIG 4U
/* Native selectors, not literal rates. B0 input copies the output selector
 * in cfsetispeed. No selector changes hardware until a backend exists. */
#define B0 0U
#define B9600 1U
#define B19200 2U
#define B38400 3U
#define B115200 4U
#define TCSANOW 0
#define TCSADRAIN 1
#define TCSAFLUSH 2
#define TCIFLUSH 0
#define TCOFLUSH 1
#define TCIOFLUSH 2
#define TCOOFF 0
#define TCOON 1
#define TCIOFF 2
#define TCION 3
#ifdef __cplusplus
extern "C" {
#endif
int tcgetattr(int, struct termios *);
int tcsetattr(int, int, const struct termios *);
speed_t cfgetispeed(const struct termios *);
speed_t cfgetospeed(const struct termios *);
int cfsetispeed(struct termios *, speed_t);
int cfsetospeed(struct termios *, speed_t);
int tcdrain(int);
int tcflush(int, int);
int tcflow(int, int);
int tcsendbreak(int, int);
#ifdef __cplusplus
}
#endif
#endif
