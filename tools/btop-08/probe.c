#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include <sys/statvfs.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    const char *path = argc == 2 ? argv[1] : "/mnt";
    const char *self = "/proc/self/mountinfo";
    char row[4096], chunk[37], point[1024];
    unsigned id, parent, major, minor;
    size_t used = 0;
    int found = 0, root = 0;
    int fd = open(self, O_RDONLY);
    if (fd < 0) {
        printf("MOUNTINFO errno=%d\n", errno);
        return 2;
    }
    ssize_t count;
    while ((count = read(fd, chunk, sizeof chunk)) > 0) {
        for (ssize_t index = 0; index < count; ++index) {
            if (used == sizeof row - 1)
                return 2;
            row[used++] = chunk[index];
            if (chunk[index] != '\n')
                continue;
            row[used] = 0;
            if (!strstr(row, " - ") ||
                sscanf(row, "%u %u %u:%u %*s %1023s", &id, &parent, &major, &minor, point) != 5)
                return 2;
            if (!strcmp(point, "/") || !strcmp(point, path))
                printf("MOUNT %s", row);
            root |= !strcmp(point, "/");
            found |= !strcmp(point, path);
            used = 0;
        }
    }
    close(fd);
    if (count < 0 || used || !root)
        return 2;
    printf("DISCOVERY root=%d target=%d\n", root, found);
    char numeric[100];
    snprintf(numeric, sizeof numeric, "/proc/%ld/mountinfo", (long)getpid());
    fd = open(numeric, O_RDONLY);
    printf("NUMERIC %d\n", fd < 0 ? errno : 0);
    if (fd >= 0)
        close(fd);
    struct statvfs fs;
    if (statvfs(path, &fs)) {
        printf("CAP path=%s errno=%d\n", path, errno);
        return 1;
    }
    printf("CAP path=%s bsize=%lu frsize=%lu blocks=%ju bfree=%ju bavail=%ju\n",
           path, fs.f_bsize, fs.f_frsize, (uintmax_t)fs.f_blocks,
           (uintmax_t)fs.f_bfree, (uintmax_t)fs.f_bavail);
    return found ? 0 : 2;
}
