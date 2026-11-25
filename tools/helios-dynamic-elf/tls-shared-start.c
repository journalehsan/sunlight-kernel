extern unsigned helios_tls_value(unsigned delta);
static __thread volatile unsigned application = 0x17;

static void resolver_negative(unsigned long *stack, unsigned test)
{
    unsigned long *aux = stack + stack[0] + 2;
    while (*aux++)
        ;
    while (aux[0] && aux[0] != 7)
        aux += 2;
    if (aux[0] != 7 || !aux[1])
        return;
    unsigned long (*resolve)(volatile unsigned long *) =
        (void *)(aux[1] + 0x4000);
    const unsigned long *descriptor;
    __asm__ volatile("mov %%fs:0,%0" : "=r"(descriptor));
    volatile unsigned long *index = (void *)descriptor[1];
    if (index[0] != 2 || index[1] != 0)
        return;
    if (test == 'm')
        index[0] = 3;
    else if (test == 'o')
        index[1] = 8;
    else if (test == 'x')
        index[1] = (unsigned long)-1;
    else if (test == 'p')
        index = (void *)1;
    else
        return;
    char marker[] = "[HELIOS-TLS-NEG] call=?\n";
    marker[sizeof(marker) - 3] = test;
    long written = 1;
    __asm__ volatile("syscall" : "+a"(written) : "D"(1L), "S"(marker),
                     "d"(sizeof(marker) - 1) : "rcx", "r11", "memory");
    if (written == sizeof(marker) - 1)
        (void)resolve(index);
}

__attribute__((used)) static void run(unsigned long *stack)
{
    unsigned delta = stack[0] > 1 ? ((const char *)stack[2])[0] - '0' : 1;
    unsigned status = 1;
    static const char key[] = "HELIOS_TLS_NEG=";
    for (unsigned long *env = stack + stack[0] + 2; *env; ++env) {
        const char *value = (const char *)*env;
        unsigned matched = 0;
        while (key[matched] && value[matched] == key[matched])
            ++matched;
        if (!key[matched]) {
            resolver_negative(stack, value[matched]);
            status = 97;
            goto exit;
        }
    }
    if (delta >= 1 && delta <= 9 && application == 0x17) {
        application += delta;
        unsigned first = helios_tls_value(delta);
        unsigned second = helios_tls_value(1);
        unsigned expected_first = (0x31u << 24) | ((0x31 + delta) << 8)
                                  | (0x51 + delta);
        unsigned expected_second = ((0x31 + delta) << 24) | ((0x51 + delta) << 16)
                                   | ((0x32 + delta) << 8) | (0xa3 + delta);
        status = first != expected_first || second != expected_second
                 || application != 0x17 + delta;
    }
    if (!status) {
        static const char ok[] =
            "[HELIOS-TLS-SHARED] app=1 initial=17 dso=2 initial=31 tail=0 writes=verified distinct=1\n";
        long result = 1;
        __asm__ volatile("syscall" : "+a"(result) : "D"(1L), "S"(ok), "d"(sizeof(ok) - 1)
                         : "rcx", "r11", "memory");
        status = result != sizeof(ok) - 1;
    }
exit:
    __asm__ volatile("syscall" : : "a"(60L), "D"((unsigned long)status)
                     : "rcx", "r11", "memory");
    __builtin_unreachable();
}

__asm__(".text\n.globl _start\n_start:\nmov %rsp,%rdi\nand $-16,%rsp\ncall run");
