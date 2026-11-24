#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static void evidence(const char *format, ...)
{
    char message[256];
    va_list arguments;
    va_start(arguments, format);
    vsnprintf(message, sizeof message, format, arguments);
    va_end(arguments);
    syscall((long)((UINT64_C(1) << 63) | 99), message, strlen(message));
}

static void sample(const char *pid, const char *leaf)
{
    char path[96], data[1024];
    snprintf(path, sizeof path, "/proc/%s/%s", pid, leaf);
    int fd = open(path, O_RDONLY);
    ssize_t count = fd < 0 ? -1 : read(fd, data, sizeof data);
    if (count < 0) {
        evidence("[PROC-FIX] reader=%ld path=%s errno=%d",
                (long)getpid(), path, errno);
    } else {
        for (ssize_t index = 0; index < count; ++index)
            if (data[index] == '\0' || data[index] == '\n') data[index] = '|';
        evidence("[PROC-FIX] reader=%ld path=%s bytes=%.*s",
                (long)getpid(), path, (int)count, data);
    }
    if (fd >= 0) close(fd);
}

static atomic_int stop_worker;

static void *worker(void *argument)
{
    (void)argument;
    while (!atomic_load(&stop_worker)) sleep(1);
    return NULL;
}

int main(int argc, char **argv)
{
    if (argc != 3) return 2;
    unsigned duration = strtoul(argv[1], NULL, 10);
    if (duration < 2) return 3;
    pthread_t thread;
    int threaded = strstr(argv[0], "original") != NULL;
    if (threaded && pthread_create(&thread, NULL, worker, NULL)) return 4;
    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    now = start;
    do {
        if (threaded) {
            DIR *dir = opendir("/proc");
            if (!dir) return 5;
            evidence("[PROC-FIX] sample=%ld reader=%ld uid=%ld",
                    now.tv_sec - start.tv_sec, (long)getpid(), (long)getuid());
            errno = 0;
            struct dirent *entry;
            while ((entry = readdir(dir))) {
                if (!isdigit((unsigned char)entry->d_name[0])) continue;
                sample(entry->d_name, "comm");
                sample(entry->d_name, "cmdline");
                sample(entry->d_name, "status");
                sample(entry->d_name, "stat");
                errno = 0;
            }
            if (errno) return 6;
            closedir(dir);
            sample(argv[2], "stat");
        }
        sleep(threaded ? 4 : duration);
        clock_gettime(CLOCK_MONOTONIC, &now);
    } while (now.tv_sec - start.tv_sec < duration);
    atomic_store(&stop_worker, 1);
    if (threaded && pthread_join(thread, NULL)) return 7;
    return 0;
}
