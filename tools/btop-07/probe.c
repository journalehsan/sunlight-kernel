#define _GNU_SOURCE
#include <errno.h>
#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include <sys/statvfs.h>

int main(int argc, char **argv)
{
    const char *path = argc == 2 ? argv[1] : "/mnt";
    char line[4096], mount[1024], stats[128];
    unsigned major, minor;
    int found = 0;
    FILE *fp = fopen("/proc/self/mountinfo", "r");
    if (!fp) {
        printf("MOUNTINFO errno=%d\n", errno);
    } else {
        while (fgets(line, sizeof(line), fp)) {
            if (sscanf(line, "%*u %*u %u:%u %*s %1023s", &major, &minor, mount) == 3
                && !strcmp(mount, path)) {
                printf("MOUNT %s", line);
                found = 1;
                break;
            }
        }
        fclose(fp);
        if (!found)
            puts("MOUNT no exact row");
    }
    if (found) {
        snprintf(stats, sizeof(stats), "/sys/dev/block/%u:%u/stat", major, minor);
        fp = fopen(stats, "r");
        if (!fp) {
            printf("IO %s errno=%d\n", stats, errno);
        } else {
            uint64_t rd, wr, busy;
            if (fscanf(fp, "%*u %*u %" SCNu64 " %*u %*u %*u %" SCNu64
                       " %*u %*u %" SCNu64, &rd, &wr, &busy) == 3)
                printf("IO %s read_sectors=%" PRIu64 " write_sectors=%" PRIu64
                       " busy_ms=%" PRIu64 "\n", stats, rd, wr, busy);
            else
                printf("IO %s unavailable: malformed\n", stats);
            fclose(fp);
        }
    } else {
        puts("IO unavailable: no device join");
    }
    struct statvfs fs;
    if (statvfs(path, &fs)) {
        printf("CAP path=%s errno=%d\n", path, errno);
        return 1;
    }
    printf("CAP path=%s bsize=%lu frsize=%lu blocks=%ju bfree=%ju bavail=%ju\n",
           path, fs.f_bsize, fs.f_frsize, (uintmax_t)fs.f_blocks,
           (uintmax_t)fs.f_bfree, (uintmax_t)fs.f_bavail);
    return 0;
}
