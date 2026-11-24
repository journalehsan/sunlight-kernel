#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

static long native(long number, long first, long second, long third)
{
    __asm__ volatile("syscall" : "+a"(number)
                     : "D"(first), "S"(second), "d"(third)
                     : "rcx", "r11", "memory");
    return number;
}

static void log_result(const char *path, long pid, long result)
{
    char message[128];
    snprintf(message, sizeof message, "[PROC-FIX] path=%s pid=%ld result=%ld",
             path, pid, result);
    native(99, (long)message, strlen(message), 0);
}

static long spawn(const char *path, const char *lifetime, const char *shell)
{
    const char *argv[] = {path, lifetime, shell, NULL};
    long pid = native(39, (long)path, (long)argv, -1);
    log_result(path, pid, pid > 0 ? 0 : pid);
    return pid;
}

static long wait_child(long pid)
{
    long result;
    do {
        result = native(32, pid, 0, 0);
    } while (result == -2);
    return result;
}

int main(int argc, char **argv)
{
    if (argc != 2) return 2;
    long alpha = spawn("/bin/masked-select", "42", argv[1]);
    long beta = spawn("/bin/masked-select-original", "75", argv[1]);
    long gamma = spawn("/bin/btop-reference", "75", argv[1]);
    long btop = spawn("/bin/btop", NULL, NULL);
    if (alpha <= 0 || beta <= 0 || gamma <= 0 || btop <= 0) return 3;
    if (nanosleep(&(struct timespec){20, 0}, NULL)) return 5;
    long delta = spawn("/bin/route-scope", "54", argv[1]);
    if (delta <= 0) return 4;
    log_result("btop-exit", btop, wait_child(btop));
    const long children[] = {alpha, beta, gamma, delta};
    for (unsigned index = 0; index < sizeof children / sizeof *children; ++index)
        log_result("fixture-exit", children[index], wait_child(children[index]));
    return 0;
}
