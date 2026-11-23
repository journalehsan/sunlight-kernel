#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define CHECK(expr) do { if (!(expr)) { \
    fprintf(stderr, "procfs FAIL line=%d %s errno=%d\n", __LINE__, #expr, errno); \
    exit(1); \
} } while (0)

static size_t sample(const char *path, char *buf, size_t cap) {
    struct timespec begin, now;
    CHECK(clock_gettime(CLOCK_MONOTONIC, &begin) == 0);
    int fd;
    int unavailable = 0;
    while ((fd = open(path, O_RDONLY | O_CLOEXEC)) < 0 && errno == EAGAIN) {
        unavailable = 1;
        CHECK(clock_gettime(CLOCK_MONOTONIC, &now) == 0);
        if (now.tv_sec - begin.tv_sec >= 4) break;
        struct timespec pause = {0, 10000000};
        nanosleep(&pause, NULL);
    }
    if (fd < 0) fprintf(stderr, "procfs unable to sample %s errno=%d\n", path, errno);
    CHECK(fd >= 0);
    if (unavailable) printf("PROCFS-RECOVERED %s after EAGAIN\n", path);
    size_t off = 0;
    ssize_t n;
    while ((n = read(fd, buf + off, cap - 1 - off)) > 0) off += (size_t)n;
    CHECK(n == 0 && off > 0 && off < cap - 1);
    CHECK(close(fd) == 0);
    buf[off] = 0;
    return off;
}

static uint64_t cpu(uint64_t *idle, unsigned *cores) {
    char buf[8194];
    uint64_t a[10], sum[10] = {0};
    sample("/proc/stat", buf, sizeof buf);
    char *save = NULL, *line = strtok_r(buf, "\n", &save);
    *cores = 0;
    uint64_t all[10];
    CHECK(line && sscanf(line, "cpu %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64
        " %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64,
        &all[0], &all[1], &all[2], &all[3], &all[4], &all[5], &all[6], &all[7], &all[8], &all[9]) == 10);
    while ((line = strtok_r(NULL, "\n", &save)) && strncmp(line, "cpu", 3) == 0) {
        unsigned cpu_id;
        CHECK(sscanf(line, "cpu%u %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64
            " %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64 " %" SCNu64,
            &cpu_id, &a[0], &a[1], &a[2], &a[3], &a[4], &a[5], &a[6], &a[7], &a[8], &a[9]) == 11);
        CHECK(cpu_id == (*cores)++);
        for (unsigned j = 0; j < 10; ++j) sum[j] += a[j];
    }
    CHECK(*cores != 0 && memcmp(sum, all, sizeof sum) == 0);
    *idle = all[3];
    return all[0] + all[1] + all[2] + all[3];
}

static uint64_t task(uint64_t *start, uint64_t *rss) {
    char buf[2048];
    sample("/proc/self/stat", buf, sizeof buf);
    CHECK(strtol(buf, NULL, 10) == getpid());
    char *end = strrchr(buf, ')'), *save = NULL;
    CHECK(end && end[1] == ' ');
    uint64_t ticks = 0;
    unsigned field = 3;
    for (char *part = strtok_r(end + 2, " \n", &save); part; part = strtok_r(NULL, " \n", &save), ++field) {
        if (field == 14 || field == 15) ticks += strtoull(part, NULL, 10);
        if (field == 22) *start = strtoull(part, NULL, 10);
        if (field == 24) *rss = strtoull(part, NULL, 10);
    }
    CHECK(field == 53 && *rss != 0);
    return ticks;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "net") == 0) {
        char buf[2048];
        sample("/proc/net/dev", buf, sizeof buf);
        char *row = strstr(buf, "eth0:");
        CHECK(row);
        printf("NET-SAMPLE %s", row);
        return 0;
    }
    struct stat st;
    CHECK(stat("/proc", &st) == 0 && S_ISDIR(st.st_mode));
    CHECK(access("/proc", R_OK | X_OK) == 0);
    CHECK(lstat("/proc/self", &st) == 0 && S_ISLNK(st.st_mode));
    char pid[32], link[32], path[128], buf[8194];
    snprintf(pid, sizeof pid, "%ld", (long)getpid());
    ssize_t linklen = readlink("/proc/self", link, sizeof link);
    CHECK(linklen == (ssize_t)strlen(pid) && memcmp(link, pid, (size_t)linklen) == 0);
    CHECK(stat("/proc/self/", &st) == 0 && S_ISDIR(st.st_mode));
    CHECK(open("/proc/stat", O_WRONLY) == -1 && errno == EROFS);

    int root = open("/proc", O_DIRECTORY | O_RDONLY);
    CHECK(root >= 0 && fstat(root, &st) == 0 && S_ISDIR(st.st_mode));
    int fd = openat(root, "stat", O_RDONLY | O_CLOEXEC);
    CHECK(fd >= 0 && fstat(fd, &st) == 0 && S_ISREG(st.st_mode));
    char a[32], b[32], c[32];
    CHECK(read(fd, a, 3) == 3 && memcmp(a, "cpu", 3) == 0);
    CHECK(read(fd, b, 7) == 7 && lseek(fd, 0, SEEK_CUR) == 10);
    CHECK(pread(fd, c, 10, 0) == 10 && !memcmp(a, c, 3) && !memcmp(b, c + 3, 7));
    CHECK(lseek(fd, 0, SEEK_CUR) == 10);
    int dupe = dup(fd);
    CHECK(dupe >= 0 && close(fd) == 0 && lseek(dupe, 0, SEEK_CUR) == 10);
    CHECK(lseek(dupe, 65536, SEEK_SET) == 65536 && read(dupe, a, 1) == 0);
    CHECK(lseek(dupe, 0, SEEK_SET) == 0 && read(dupe, a, 3) == 3 && close(dupe) == 0);
    CHECK(close(root) == 0);

    DIR *dir = opendir("/proc");
    CHECK(dir);
    struct dirent *entry;
    int saw_self = 0, saw_pid = 0, saw_hidden = 0, count = 0;
    while ((entry = readdir(dir))) {
        if (!strcmp(entry->d_name, "self")) saw_self = 1;
        if (!strcmp(entry->d_name, pid)) saw_pid = 1;
        if (argc == 2 && !strcmp(entry->d_name, argv[1])) saw_hidden = 1;
        ++count;
    }
    CHECK(closedir(dir) == 0 && saw_self && saw_pid && !saw_hidden && count <= 135);
    if (argc == 2) {
        snprintf(path, sizeof path, "/proc/%s/stat", argv[1]);
        CHECK(open(path, O_RDONLY) < 0 && errno == ENOENT);
    }
    CHECK(open("/proc/1/status", O_RDONLY) < 0 && errno == ENOENT);
    CHECK(stat("/proc/1", &st) < 0 && errno == ENOENT);
    sample("/proc/self/cmdline", buf, sizeof buf);
    CHECK(!strcmp(buf, argv[0]));
    sample("/proc/self/status", buf, sizeof buf);
    snprintf(path, sizeof path, "Uid:\t%ld\t", (long)getuid());
    CHECK(strstr(buf, path));
    uint64_t total, free_kb, avail;
    sample("/proc/meminfo", buf, sizeof buf);
    CHECK(sscanf(buf, "MemTotal: %" SCNu64 " kB\nMemFree: %" SCNu64 " kB\nMemAvailable: %" SCNu64 " kB",
        &total, &free_kb, &avail) == 3 && total > free_kb && avail == free_kb && !(total % 4));
    CHECK(sysconf(_SC_CLK_TCK) == 100 && sysconf(_SC_PAGE_SIZE) == 4096);
    uint64_t idle1, idle2, start1 = 0, start2 = 0, rss1 = 0, rss2 = 0;
    unsigned cores1, cores2;
    uint64_t global1 = cpu(&idle1, &cores1);
    uint64_t proc1 = task(&start1, &rss1);
    struct timespec t0, t1;
    CHECK(clock_gettime(CLOCK_MONOTONIC, &t0) == 0);
    do { CHECK(clock_gettime(CLOCK_MONOTONIC, &t1) == 0); }
    while ((t1.tv_sec - t0.tv_sec) * 1000000000L + t1.tv_nsec - t0.tv_nsec < 300000000L);
    uint64_t proc2 = task(&start2, &rss2);
    uint64_t global2 = cpu(&idle2, &cores2);
    sample("/proc/uptime", buf, sizeof buf);
    double up, idle;
    CHECK(sscanf(buf, "%lf %lf", &up, &idle) == 2 && up >= 0 && idle >= 0);
    CHECK(clock_gettime(CLOCK_MONOTONIC, &t1) == 0 && up >= (double)t1.tv_sec - 1.0 && up <= (double)t1.tv_sec + 1.0);
    CHECK(start1 == start2 && start1 <= (uint64_t)(up * 100) && proc2 > proc1);
    CHECK(global2 > global1 && idle2 >= idle1 && cores1 == cores2);
    CHECK(cores2 == (unsigned)sysconf(_SC_NPROCESSORS_ONLN));
    double elapsed_capacity = up * 100.0 * cores2;
    CHECK(global2 > elapsed_capacity * 0.5 && global2 < elapsed_capacity * 1.5);
    printf("PROCFS-PASS pid=%s hidden=%s cores=%u cpu=%" PRIu64 "->%" PRIu64
           " task=%" PRIu64 "->%" PRIu64 " rss=%" PRIu64 " mem=%" PRIu64 "/%" PRIu64
           " uptime=%.2f\n", pid, argc == 2 ? argv[1] : "-", cores2, global1, global2,
           proc1, proc2, rss2, free_kb, total, up);
    return 0;
}
