typedef unsigned long word;

__asm__(".text\n.globl _start\n_start:\n"
        "mov %rsp,%rdi\ncall inspect\nud2\n");

static long syscall3(long nr, long first, long second, long third)
{
    __asm__ volatile("syscall" : "+a"(nr) : "D"(first), "S"(second),
                     "d"(third) : "rcx", "r11", "memory");
    return nr;
}

static int equal(const char *left, const char *right)
{
    while (*left && *left == *right) {
        ++left;
        ++right;
    }
    return *left == *right;
}

__attribute__((used, noreturn)) static void inspect(const word *stack)
{
    const char *prefix = "/Applications/elf-pie.sunapp/Program/probe";
    if (((word)stack & 15) || stack[0] != 1 || !stack[1] || stack[2])
        goto fail;
    const char *argv0 = (const char *)stack[1];
    const word *aux = stack + 3;
    unsigned envc = 0;
    int found_env = 0;
    for (; *aux; ++aux) {
        const char *env = (const char *)*aux;
        if (equal(env, "HELIOS_HANDOFF=fixture"))
            found_env = 1;
        while (*env && *env != '=')
            ++env;
        if (*env != '=' || ++envc > 128)
            goto fail;
    }
    if (!found_env)
        goto fail;
    word values[32] = {0};
    unsigned seen = 0;
    for (++aux; aux[0]; aux += 2) {
        if (aux[0] >= 32 || (seen & (1u << aux[0])))
            goto fail;
        seen |= 1u << aux[0];
        values[aux[0]] = aux[1];
    }
    const int relocate = equal(argv0, "/Applications/elf-relocation.sunapp/Program/probe");
    const int pie = relocate || equal(argv0, prefix);
    if (!pie && !equal(argv0, "/Applications/elf-exec.sunapp/Program/probe"))
        goto fail;
    const word app_base = pie ? 0x20000000 : 0x400000;
    if (aux[1] || values[7] != 0x80000000 || values[3] != app_base + 64 ||
        values[4] != 56 || values[6] != 4096 || values[31] != stack[1] ||
        !values[25] || *(const unsigned *)values[7] != 0x464c457f)
        goto fail;
    const unsigned char *ehdr = (const unsigned char *)app_base;
    if (*(const unsigned *)ehdr != 0x464c457f ||
        values[5] != *(const unsigned short *)(ehdr + 56) ||
        values[9] != *(const word *)(ehdr + 24) + (pie ? app_base : 0) ||
        (!relocate && *(const unsigned short *)values[9] != 0x0b0f))
        goto fail;
    if (relocate) {
        static const char ready[] = "[HELIOS-RELOC] interpreter verified initial stack; entering application\n";
        if (syscall3(1, 1, (long)ready, sizeof(ready) - 1) != sizeof(ready) - 1)
            goto fail;
        __asm__ volatile("mov %0, %%rsp; xor %%edx, %%edx; jmp *%1"
                         :: "r"(stack), "r"(values[9]) : "rdx", "memory");
        __builtin_unreachable();
    }
    static const char ok[] = "[HELIOS-HANDOFF] interpreter entry verified; application entry not called\n";
    if (syscall3(1, 1, (long)ok, sizeof(ok) - 1) == sizeof(ok) - 1)
        syscall3(60, 0, 0, 0);
fail:
    {
        static const char bad[] = "[HELIOS-HANDOFF] initial stack verification failed\n";
        syscall3(1, 1, (long)bad, sizeof(bad) - 1);
        syscall3(60, 1, 0, 0);
    }
    __builtin_trap();
}
