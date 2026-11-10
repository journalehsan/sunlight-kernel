/* Native-only fixtures: raw calls below are SunlightOS ABI numbers. */
#include <dirent.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static intptr_t raw(uint64_t op, uint64_t a, uint64_t b) {
    uint64_t result;
    __asm__ volatile("syscall" : "=a"(result) : "a"(op), "D"(a), "S"(b)
                     : "rcx", "r11", "memory");
    return (intptr_t)result;
}
static int root(void) {
    /* Reuse the bounded heap repeatedly and verify independent stream state. */
    for (int i = 0; i < 256; ++i) {
        errno = 123;
        DIR *a = opendir("."), *b = opendir("/");
        if (!a || !b || errno != 123) return 10;
        struct dirent *ea = readdir(a), *eb = readdir(b);
        if (!ea || !eb || ea == eb || strcmp(ea->d_name, eb->d_name)) return 11;
        int found = !strcmp(ea->d_name, "tmp"), count = 1;
        while ((ea = readdir(a))) {
            found += !strcmp(ea->d_name, "tmp");
            if (++count > 256) return 12;
        }
        if (found != 1 || errno != 123 || readdir(a) || errno != 123) return 13;
        /* Reading a to EOF must not consume b. */
        int other = 1;
        while (readdir(b)) if (++other > 256) return 14;
        if (other != count || closedir(a) || closedir(b) || errno != 123) return 15;
    }
    return 0;
}

static int errors(const char *work) {
    errno = 0;
    if (opendir(0) || errno != EFAULT) return 20;
    if (opendir("") || errno != ENOENT) return 21;
    const char *unsupported[] = {"relative", "..", "./tmp", "/tmp/.", "/tmp/../tmp", "//tmp", "/tmp/"};
    for (unsigned i = 0; i < sizeof unsupported / sizeof unsupported[0]; ++i)
        if (opendir(unsupported[i]) || errno != ENOSYS) return 22;
    char longpath[257]; memset(longpath, 'x', sizeof longpath); longpath[256] = 0;
    if (opendir(longpath) || errno != E2BIG) return 23;
    if (readdir(0) || errno != EBADF || closedir(0) != -1 || errno != EBADF) return 24;
    if (opendir(work) || errno != ENOENT) return 25;
    FILE *file = fopen(work, "w");
    if (!file || fclose(file)) return 26;
    int result = 0;
    if (opendir(work) || errno != ENOTDIR) result = 27;
    if (raw(65, (uintptr_t)work, 0) || result) return result ? result : 28;
    /* Pre-existing mode-000 directory in the isolated fixture. This exercises
     * the real kernel permission check without requiring a rmdir capability. */
    if (opendir("/tmp/native-c-denied") || errno != EACCES) result = 30;
    return result;
}

static int entries(const char *work) {
    char path[256];
    size_t len = strlen(work);
    if (len > 180) return 40;
    errno = 123;
    DIR *empty = opendir(work);
    if (!empty || readdir(empty) || errno != 123 || closedir(empty)) return 41;
    int created = 0, result = 0;
    for (int i = 0; i < 71; ++i) {
        memcpy(path, work, len); path[len] = '/';
        if (i < 70) {
            memcpy(path + len + 1, "entry-00", 9);
            path[len + 7] = (char)('0' + i / 10);
            path[len + 8] = (char)('0' + i % 10);
        } else {
            memset(path + len + 1, 'x', 64); path[len + 65] = 0;
        }
        FILE *file = fopen(path, "w");
        if (!file || fclose(file)) { result = 42; goto cleanup; }
        ++created;
    }
    if (raw(63, (uintptr_t)work, 0)) { result = 43; goto cleanup; }
    DIR *dir = opendir(".");
    if (raw(63, (uintptr_t)"/", 0) || !dir) { result = 44; goto cleanup; }
    unsigned char seen[71] = {0}; int count = 0;
    errno = 123;
    struct dirent *entry;
    while ((entry = readdir(dir))) {
        size_t n = strlen(entry->d_name); int index = -1;
        if (n == 64) {
            index = 70;
            for (size_t j = 0; j < n; ++j) if (entry->d_name[j] != 'x') index = -1;
        } else if (n == 8 && !memcmp(entry->d_name, "entry-", 6) &&
                   entry->d_name[6] >= '0' && entry->d_name[6] <= '6' &&
                   entry->d_name[7] >= '0' && entry->d_name[7] <= '9') {
            index = (entry->d_name[6] - '0') * 10 + entry->d_name[7] - '0';
        }
        if (index < 0 || seen[index]++ || ++count > 71) { result = 45; break; }
    }
    if (!result && (count != 71 || errno != 123 || readdir(dir) || errno != 123)) result = 46;
    if (closedir(dir) && !result) result = 47;
cleanup:
    for (int i = 0; i < created; ++i) {
        memcpy(path, work, len); path[len] = '/';
        if (i < 70) {
            memcpy(path + len + 1, "entry-00", 9);
            path[len + 7] = (char)('0' + i / 10);
            path[len + 8] = (char)('0' + i % 10);
        } else {
            memset(path + len + 1, 'x', 64); path[len + 65] = 0;
        }
        if (raw(65, (uintptr_t)path, 0) && !result) result = 48;
    }
    return result;
}
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "root")) return root();
    if (argc != 3) return 90;
    if (!strcmp(argv[1], "errors")) return errors(argv[2]);
    if (!strcmp(argv[1], "entries")) return entries(argv[2]);
    return 91;
}
