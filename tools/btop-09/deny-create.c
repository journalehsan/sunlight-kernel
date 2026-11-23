#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/statvfs.h>
#include <sys/syscall.h>
#include <sys/vfs.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    const char *parent = argc > 1 ? argv[1] : "/mnt";
    char path[256];
    int length = snprintf(path, sizeof path, "%s/BTOP09.BIN", parent);
    if (length < 0 || (size_t)length >= sizeof path)
        return 2;
    struct statfs fs;
    struct statvfs vfs;
    printf("BTOP09 uid=%u euid=%u gid=%u egid=%u parent=%s\n",
           getuid(), geteuid(), getgid(), getegid(), parent);
    if (statfs(parent, &fs) == 0)
        printf("BTOP09 statfs flags=%#lx bsize=%ld frsize=%ld blocks=%llu free=%llu avail=%llu\n",
               (unsigned long)fs.f_flags, fs.f_bsize, fs.f_frsize,
               (unsigned long long)fs.f_blocks, (unsigned long long)fs.f_bfree,
               (unsigned long long)fs.f_bavail);
    else
        printf("BTOP09 statfs errno=%d\n", errno);
    if (statvfs(parent, &vfs) == 0)
        printf("BTOP09 statvfs flags=%#lx bsize=%lu frsize=%lu blocks=%llu free=%llu avail=%llu\n",
               vfs.f_flag, vfs.f_bsize, vfs.f_frsize, (unsigned long long)vfs.f_blocks,
               (unsigned long long)vfs.f_bfree, (unsigned long long)vfs.f_bavail);
    else
        printf("BTOP09 statvfs errno=%d\n", errno);
    const int flags = O_WRONLY | O_CREAT | O_EXCL;
    errno = 0;
    long fd = syscall(SYS_openat, AT_FDCWD, path, flags, 0600);
    printf("BTOP09 syscall=%ld dirfd=%d path=%s flags=%#x mode=0600 result=%ld errno=%d\n",
           (long)SYS_openat, AT_FDCWD, path, flags, fd, errno);
    if (fd >= 0) {
        if (close((int)fd) < 0 || unlink(path) < 0)
            return 2;
        return 1;
    }
    return 0;
}
