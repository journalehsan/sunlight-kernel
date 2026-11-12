#include <termios.h>
#include <errno.h>
#include <unistd.h>
#include <string.h>
#include <sys/socket.h>
static long dupfd(int fd) {
    long n=45;
    __asm__ volatile("syscall" : "+a"(n) : "D"((long)fd) : "rcx","r11","memory");
    return n;
}
static int terminal_identity(void) {
    struct termios t={0}, saved=t;
    int dup=(int)dupfd(3);
    if(dup<0) return 40;
    int fds[]={3,dup};
    for(unsigned i=0;i<2;++i) {
        errno=123;
        if(!isatty(fds[i]) || errno!=123) return 41;
        if(tcgetattr(fds[i],&t)!=-1 || errno!=ENOTSUP || memcmp(&t,&saved,sizeof t)) return 42;
        if(tcgetattr(fds[i],(void *)1)!=-1 || errno!=ENOTSUP) return 43;
        for(int a=TCSANOW;a<=TCSAFLUSH;++a)
            if(tcsetattr(fds[i],a,(void *)-2UL)!=-1 || errno!=ENOTSUP) return 44;
        t.c_lflag=ICANON|ECHO|ISIG;
        if(tcsetattr(fds[i],TCSANOW,&t)!=-1 || errno!=ENOTSUP) return 45;
        t=saved;
        if(tcdrain(fds[i])!=-1 || errno!=ENOTSUP) return 46;
        for(int a=TCIFLUSH;a<=TCIOFLUSH;++a)
            if(tcflush(fds[i],a)!=-1 || errno!=ENOTSUP) return 47;
        for(int a=TCOOFF;a<=TCION;++a)
            if(tcflow(fds[i],a)!=-1 || errno!=ENOTSUP) return 48;
        if(tcsendbreak(fds[i],0)!=-1 || errno!=ENOTSUP) return 49;
    }
    if(close(3) || !isatty(dup) || close(dup)) return 50;
    if(isatty(dup) || errno!=EBADF) return 51;
    return 0;
}
int main(int argc, char **argv) {
    (void)argv;
    if(argc>1) return terminal_identity();
    struct termios t={0}, before;
    for (speed_t s=B0; s<=B115200; ++s) {
        errno=123;
        if (cfsetospeed(&t,s) || cfgetospeed(&t)!=s || errno!=123) return 1;
        if (cfsetispeed(&t,B0) || cfgetispeed(&t)!=s) return 2;
        if (cfsetispeed(&t,B19200) || cfgetispeed(&t)!=B19200) return 3;
    }
    before=t;
    if (cfsetospeed(&t,999)!=-1 || errno!=EINVAL || memcmp(&t,&before,sizeof t)) return 4;
    if (cfsetispeed(&t,999)!=-1 || errno!=EINVAL || memcmp(&t,&before,sizeof t)) return 5;
    if (cfsetispeed(0,B0)!=-1 || errno!=EFAULT) return 6;
    if (cfsetospeed(0,B0)!=-1 || errno!=EFAULT) return 7;
    if (tcgetattr(-1,&t)!=-1 || errno!=EBADF) return 8;
    if (tcgetattr(255,(void *)1)!=-1 || errno!=EBADF) return 9;
    if (isatty(-1) || errno!=EBADF) return 10;
    if (tcsetattr(0,99,&t)!=-1 || errno!=EINVAL) return 11;
    if (tcflush(0,99)!=-1 || errno!=EINVAL) return 12;
    if (tcflow(0,99)!=-1 || errno!=EINVAL) return 13;
    int socketfd=socket(AF_INET,SOCK_STREAM,0);
    if(socketfd<0) return 14;
    int dup=(int)dupfd(socketfd);
    if(dup<0) return 15;
    int fds[]={0,1,2,socketfd,dup};
    for (int repeat=0; repeat<100; ++repeat) for(unsigned i=0;i<sizeof fds/sizeof fds[0];++i) {
        int fd=fds[i];
        if(isatty(fd) || errno!=ENOTTY) return 16;
        if(tcgetattr(fd,&t)!=-1 || errno!=ENOTTY || memcmp(&t,&before,sizeof t)) return 17;
        if(tcgetattr(fd,0)!=-1 || errno!=ENOTTY) return 18;
        if(tcgetattr(fd,(void *)-2UL)!=-1 || errno!=ENOTTY) return 19;
        for(int a=TCSANOW;a<=TCSAFLUSH;++a)
            if(tcsetattr(fd,a,(void *)1)!=-1 || errno!=ENOTTY) return 20;
        if(tcdrain(fd)!=-1 || errno!=ENOTTY) return 21;
        for(int a=TCIFLUSH;a<=TCIOFLUSH;++a)
            if(tcflush(fd,a)!=-1 || errno!=ENOTTY) return 22;
        for(int a=TCOOFF;a<=TCION;++a)
            if(tcflow(fd,a)!=-1 || errno!=ENOTTY) return 23;
        if(tcsendbreak(fd,0)!=-1 || errno!=ENOTTY) return 24;
    }
    if(close(socketfd) || close(dup)) return 25;
    if(tcgetattr(dup,&t)!=-1 || errno!=EBADF) return 26;
    return 0;
}
