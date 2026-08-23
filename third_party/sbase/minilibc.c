/* Tiny Linux syscall libc used only to host unmodified sbase echo(1).
 * This is a portability layer, not an application-specific Helios hack.
 */

typedef long ssize_t;
typedef unsigned long size_t;

struct HeliosFile {
    int fd;
    int err;
    int closed;
};

static struct HeliosFile stdout_file = {1, 0, 0};
static struct HeliosFile stderr_file = {2, 0, 0};
struct HeliosFile *stdout = &stdout_file;
struct HeliosFile *stderr = &stderr_file;
char *argv0;

static long sys_write(int fd, const void *buf, size_t n)
{
    long ret;
    __asm__ volatile("syscall"
                     : "=a"(ret)
                     : "a"(1L), "D"(fd), "S"(buf), "d"(n)
                     : "rcx", "r11", "memory");
    return ret;
}

static void sys_exit(int code)
{
    __asm__ volatile("syscall"
                     :
                     : "a"(60L), "D"((long)code)
                     : "rcx", "r11", "memory");
    __builtin_unreachable();
}

void exit(int status)
{
    sys_exit(status);
}

size_t strlen(const char *s)
{
    size_t n = 0;
    while (s[n]) {
        n++;
    }
    return n;
}

int strcmp(const char *a, const char *b)
{
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return (unsigned char)*a - (unsigned char)*b;
}

int fputc(int c, struct HeliosFile *fp)
{
    unsigned char ch = (unsigned char)c;
    if (!fp || fp->closed) {
        return -1;
    }
    if (sys_write(fp->fd, &ch, 1) != 1) {
        fp->err = 1;
        return -1;
    }
    return c;
}

int putchar(int c)
{
    return fputc(c, stdout);
}

int fputs(const char *s, struct HeliosFile *fp)
{
    size_t n;
    if (!fp || fp->closed || !s) {
        return -1;
    }
    n = strlen(s);
    if (n == 0) {
        return 0;
    }
    if (sys_write(fp->fd, s, n) != (ssize_t)n) {
        fp->err = 1;
        return -1;
    }
    return 0;
}

int fflush(struct HeliosFile *fp)
{
    (void)fp;
    return 0;
}

int fclose(struct HeliosFile *fp)
{
    if (!fp) {
        return -1;
    }
    fp->closed = 1;
    return 0;
}

int ferror(struct HeliosFile *fp)
{
    return fp ? fp->err : 1;
}

int vfprintf(struct HeliosFile *fp, const char *fmt, __builtin_va_list ap)
{
    (void)ap;
    return fputs(fmt, fp);
}

void weprintf(const char *fmt, ...)
{
    __builtin_va_list ap;
    __builtin_va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    __builtin_va_end(ap);
    fputc('\n', stderr);
}

int main(int argc, char **argv);

void _start(void)
{
    long argc;
    char **argv;
    int status;

    __asm__ volatile("mov (%%rsp), %0" : "=r"(argc));
    __asm__ volatile("lea 8(%%rsp), %0" : "=r"(argv));
    status = main((int)argc, argv);
    sys_exit(status);
}
