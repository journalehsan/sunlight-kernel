#define main proc01_main
#include "../btop-proc-01/leader.c"
#undef main

static int ordered_read(void)
{
    const long tag = (long)(UINT64_C(1) << 63);
    int ready[2], release[2];
    if (pipe(ready) || pipe(release)) return 20;
    const char *path = "/bin/btop-reference";
    const char *args[] = {path, "barrier-target", NULL};
    long pid = syscall(tag | 39, path, args, (long)ready[1],
                       (long)release[0], -1L, 0x53554e535444494fL);
    evidence("[PROC-ORDER] checkpoint=spawn parent=%ld target=%ld", (long)getpid(), pid);
    if (pid <= 0) return 21;
    close(ready[1]);
    close(release[0]);
    char byte;
    if (read(ready[0], &byte, 1) != 1 || byte != 'R') return 22;
    close(ready[0]);
    evidence("[PROC-ORDER] checkpoint=ready target=%ld", pid);
    char status[96];
    snprintf(status, sizeof status, "/proc/%ld/status", pid);
    errno = 0;
    int saved = open(status, O_RDONLY);
    evidence("[PROC-ORDER] checkpoint=open target=%ld fd=%d errno=%d", pid, saved, errno);
    if (saved < 0) return 23;
    evidence("[PROC-ORDER] checkpoint=release target=%ld", pid);
    if (write(release[1], "G", 1) != 1) return 24;
    close(release[1]);
    long waited;
    do {
        waited = tag | 32;
        __asm__ volatile("syscall" : "+a"(waited) : "D"(pid), "S"(0L)
                         : "rcx", "r11", "memory");
    } while (waited == -2);
    evidence("[PROC-ORDER] checkpoint=wait target=%ld result=%ld", pid, waited);
    if (waited != 0) return 25;
    evidence("[PROC-ORDER] checkpoint=read-enter target=%ld fd=%d", pid, saved);
    unsigned char data[32];
    errno = 0;
    ssize_t result = read(saved, data, sizeof data);
    int error = errno;
    char bytes[2 * sizeof data + 1] = {0};
    for (ssize_t index = 0; index < result; ++index)
        snprintf(bytes + 2 * index, 3, "%02x", data[index]);
    evidence("[PROC-ORDER] checkpoint=read-return target=%ld fd=%d result=%ld errno=%d bytes=%s",
             pid, saved, (long)result, error, bytes);
    close(saved);
    return result == -1 && error == ENOENT ? 0 : 26;
}

static int registry_observer(void)
{
    static const char *const leaves[] = {"comm", "cmdline", "status", "stat"};
    for (unsigned round = 0; round < 20; ++round) {
        evidence("[PROC-REG] before");
        DIR *dir = opendir("/proc");
        if (!dir) return 30;
        for (;;) {
            errno = 0;
            struct dirent *entry = readdir(dir);
            if (!entry) {
                if (errno) return 31;
                break;
            }
            if (!isdigit((unsigned char)entry->d_name[0])) continue;
            evidence("[PROC-SETS] round=%u reader=%ld pid=%s", round, (long)getpid(), entry->d_name);
            for (unsigned index = 0; index < sizeof leaves / sizeof *leaves; ++index) {
                char path[96], data[1024];
                snprintf(path, sizeof path, "/proc/%lu/%s", strtoul(entry->d_name, NULL, 10), leaves[index]);
                int fd = open(path, O_RDONLY);
                int open_error = fd < 0 ? errno : 0;
                ssize_t count = fd < 0 ? -1 : read(fd, data, sizeof data);
                int read_error = fd >= 0 && count < 0 ? errno : 0;
                evidence("[PROC-SETS] round=%u path=%s fd=%d open_errno=%d read=%ld read_errno=%d",
                         round, path, fd, open_error, (long)count, read_error);
                if (fd >= 0) close(fd);
            }
        }
        closedir(dir);
        evidence("[PROC-SETS] round=%u complete", round);
        evidence("[PROC-REG] after");
        sleep(4);
    }
    return 0;
}

static int observe_exit(const char *pid)
{
    char path[96], byte;
    snprintf(path, sizeof path, "/proc/%s/status", pid);
    DIR *dir = opendir("/proc");
    if (!dir) return 11;
    struct dirent *entry;
    int present = 0;
    while ((entry = readdir(dir)))
        if (!strcmp(entry->d_name, pid)) present = 1;
    closedir(dir);
    errno = 0;
    int saved = open(path, O_RDONLY);
    evidence("[PROC-LIFETIME] checkpoint=open pid=%s enumerated=%d fd=%d errno=%d",
             pid, present, saved, errno);
    if (!present || saved < 0) return 12;
    int initial = (int)read(saved, &byte, 1);
    evidence("[PROC-LIFETIME] checkpoint=initial-read pid=%s result=%d errno=%d",
             pid, initial, errno);
    if (initial != 1) return 16;
    for (unsigned attempt = 0; attempt < 35; ++attempt) {
        errno = 0;
        int probe = open(path, O_RDONLY);
        int lookup_error = errno;
        evidence("[PROC-LIFETIME] checkpoint=lookup pid=%s attempt=%u fd=%d errno=%d",
                 pid, attempt, probe, lookup_error);
        if (probe < 0) {
            if (lookup_error != ENOENT) return 13;
            evidence("[PROC-LIFETIME] checkpoint=stale-read-start pid=%s", pid);
            errno = 0;
            int result = (int)read(saved, &byte, 1);
            int error = errno;
            evidence("[PROC-LIFETIME] pid=%s enumerated=1 reopen=ENOENT saved_read=%d errno=%d",
                     pid, result, error);
            close(saved);
            return result == -1 && error == ENOENT ? 0 : 14;
        }
        close(probe);
        struct timespec delay = {1, 0};
        if (nanosleep(&delay, NULL)) {
            evidence("[PROC-LIFETIME] checkpoint=sleep pid=%s errno=%d", pid, errno);
            return 17;
        }
    }
    return 15;
}

int main(int argc, char **argv)
{
    if (argc == 2 && !strcmp(argv[1], "ordered-read")) return ordered_read();
    if (argc == 3 && !strcmp(argv[1], "registry")) return registry_observer();
    if (argc == 2 && !strcmp(argv[1], "barrier-target")) {
        char byte;
        return write(1, "R", 1) == 1 && read(0, &byte, 1) == 1 && byte == 'G' ? 0 : 27;
    }
    if (argc == 3 && !strcmp(argv[1], "lifetime")) return observe_exit(argv[2]);
    if (argc != 3 || strcmp(argv[1], "restricted"))
        return proc01_main(argc, argv);
    DIR *dir = opendir("/proc");
    if (!dir || getuid() != 1234) return 8;
    struct dirent *entry;
    long parent = getppid(), self = getpid();
    if (parent <= 0 || parent == self) return 9;
    unsigned seen = 0;
    evidence("[PROC-RESTRICTED] reader=%ld uid=%ld", (long)getpid(), (long)getuid());
    while ((entry = readdir(dir))) {
        if (!isdigit((unsigned char)entry->d_name[0])) continue;
        evidence("[PROC-RESTRICTED] entry=%s", entry->d_name);
        long pid = strtol(entry->d_name, NULL, 10);
        if (pid == self) seen |= 1;
        else if (pid == parent) seen |= 2;
        else return 9;
    }
    closedir(dir);
    int lookup_ok = 1;
    for (unsigned index = 0; index < 3; ++index) {
        char path[96];
        if (index == 0) snprintf(path, sizeof path, "/proc/%ld", self);
        else if (index == 1) snprintf(path, sizeof path, "/proc/%ld", parent);
        else snprintf(path, sizeof path, "/proc/%s", argv[2]);
        errno = 0;
        int fd = open(path, O_RDONLY | O_DIRECTORY);
        int error = errno;
        evidence("[PROC-DISCOVERY] path=%s fd=%d errno=%d", path, fd, error);
        if (fd >= 0) close(fd);
        if (index < 2 ? fd < 0 : fd >= 0 || error != ENOENT) lookup_ok = 0;
    }
    static const char *const leaves[] = {"comm", "cmdline", "status", "stat"};
    for (unsigned index = 0; index < sizeof leaves / sizeof *leaves; ++index) {
        char path[96];
        snprintf(path, sizeof path, "/proc/%s/%s", argv[2], leaves[index]);
        int fd = open(path, O_RDONLY);
        evidence("[PROC-RESTRICTED] path=%s fd=%d errno=%d", path, fd, errno);
        if (fd >= 0 || errno != ENOENT) return 10;
    }
    return seen == 3 && lookup_ok ? 0 : 9;
}
