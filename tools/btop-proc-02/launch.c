#define main proc01_main
#include "../btop-proc-01/launch.c"
#undef main

int main(int argc, char **argv)
{
    if (argc == 3 && !strcmp(argv[1], "native"))
        return nanosleep(&(struct timespec){12, 0}, NULL) != 0;
    if (argc == 3 && !strcmp(argv[1], "restricted")) {
        if (native(37, 1234, 0, 0)) return 8;
        long restricted = spawn("/bin/route-scope", "restricted", argv[2]);
        return restricted > 0 ? (int)wait_child(restricted) : 9;
    }
    if (argc != 2 && (argc != 3 || strcmp(argv[2], "registry"))) return 2;
    long alpha = spawn("/bin/masked-select", "42", argv[1]);
    long beta = spawn("/bin/masked-select-original", "75", argv[1]);
    long gamma = spawn("/bin/btop-reference", argc == 3 ? "registry" : "75", argv[1]);
    long btop = spawn("/bin/btop", NULL, NULL);
    if (alpha <= 0 || beta <= 0 || gamma <= 0 || btop <= 0) return 3;
    long restricted = spawn("/bin/tty-select", "restricted", argv[1]);
    if (restricted <= 0 || wait_child(restricted)) return 4;
    if (nanosleep(&(struct timespec){19, 0}, NULL)) return 5;
    long delta = spawn("/bin/tty-select", "native", argv[1]);
    if (delta <= 0) return 6;
    log_result("native-start", delta, 0);
    char target[24];
    snprintf(target, sizeof target, "%ld", delta);
    long observer = spawn("/bin/route-scope", "lifetime", target);
    if (observer <= 0) return 7;
    log_result("native-exit", delta, wait_child(delta));
    log_result("observer-exit", observer, wait_child(observer));
    log_result("btop-exit", btop, wait_child(btop));
    const long children[] = {alpha, beta, gamma};
    for (unsigned index = 0; index < sizeof children / sizeof *children; ++index)
        log_result("fixture-exit", children[index], wait_child(children[index]));
    return 0;
}
