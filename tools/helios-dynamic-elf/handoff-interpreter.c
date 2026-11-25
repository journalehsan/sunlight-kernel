typedef unsigned long word;

__asm__(".text\n.globl _start\n_start:\n"
        "mov %rsp,%rdi\ncall inspect\nud2\n");

static long syscall3(long nr, long first, long second, long third)
{
    __asm__ volatile("syscall" : "+a"(nr) : "D"(first), "S"(second),
                     "d"(third) : "rcx", "r11", "memory");
    return nr;
}

#ifdef STARTUP_TLS
struct tls_index { word module, offset; };
struct tls_module {
    unsigned char digest[32];
    word id, present, image_offset, filesz, memsz, align, base, first, second, identity;
};
struct tls_resolver {
    const struct tls_module *table;
    const struct tls_index *index;
    word dso_base, dso_size;
};
_Static_assert(sizeof(struct tls_module) == 112, "startup module row size");

__attribute__((used, section(".tlsresolver"), visibility("hidden")))
word __tls_get_addr(const struct tls_index *index)
{
    const struct tls_resolver *resolver;
    __asm__ volatile("mov %%fs:0,%0" : "=r"(resolver));
    if (resolver && resolver->table && index && index == resolver->index &&
        index->module == 2 && index->offset == 0 &&
        resolver->table[0].id == 1 && resolver->table[0].present == 1 &&
        resolver->table[1].id == 2 && resolver->table[1].present == 1) {
        const struct tls_module *module = &resolver->table[1];
        if (!module->base || module->base != resolver->dso_base ||
            module->memsz != resolver->dso_size || module->memsz != 8 ||
            module->base > (word)-1 - module->memsz ||
            resolver->table[0].base == module->base)
            goto fail;
        static const char ok[] = "[HELIOS-TLS-SHARED] resolver module=2 offset=0 result=dso-block\n";
        syscall3(1, 1, (long)ok, sizeof(ok) - 1);
        return module->base;
    }
fail:
    syscall3(60, 1, 0, 0);
    __builtin_trap();
}
#endif

static int equal(const char *left, const char *right)
{
    while (*left && *left == *right) {
        ++left;
        ++right;
    }
    return *left == *right;
}

#ifdef MALLOC_FIXTURE
#include "bounded-malloc.h"

static long setup_map(word bytes)
{
    long nr = 9;
    register long flags __asm__("r10") = 0x22;
    register long fd __asm__("r8") = -1;
    register long offset __asm__("r9") = 0;
    __asm__ volatile("syscall" : "+a"(nr) : "D"(0L), "S"(bytes), "d"(3L),
                     "r"(flags), "r"(fd), "r"(offset) : "rcx", "r11", "memory");
    return nr;
}

static long setup_unmap(word base, word bytes)
{
    return syscall3(11, base, bytes, 0);
}

static void __attribute__((noreturn)) setup_fatal(void)
{
    syscall3(60, 1, 0, 0);
    __builtin_trap();
}

static void __attribute__((noreturn)) reject_setup(void)
{
    const struct page_backend backend = {setup_map, setup_unmap, 0, setup_fatal};
    struct helios_heap heap = {.pages.owner = (word)syscall3(39, 0, 0, 0)};
    struct helios_allocation allocation;
    if (helios_malloc(&heap, &backend, PAGE_BYTES + 1, 0, &allocation))
        setup_fatal();
    volatile unsigned char *bytes = (volatile unsigned char *)helios_resolve(&heap, allocation);
    if (!bytes || bytes[0] || bytes[PAGE_BYTES])
        setup_fatal();
    bytes[0] = 0x49;
    bytes[PAGE_BYTES] = 0x49;
    if (bytes[0] != 0x49 || bytes[PAGE_BYTES] != 0x49)
        setup_fatal();
    static const char message[] = "[MALLOC-CHECK] interpreter-setup-aborted live=2 marker=49 exit=44\n";
    if (syscall3(1, 1, (long)message, sizeof(message) - 1) != sizeof(message) - 1)
        setup_fatal();
    syscall3(60, 44, 0, 0);
    __builtin_trap();
}
#endif

#ifdef C_DSO_FIXTURE
#include "c-utility.h"

static void __attribute__((noreturn)) enter_c_dso(const word *stack, word entry)
{
    __asm__ volatile("mov %[stack], %%rsp; jmp *%[entry]"
                     :: "D"(2UL), "S"(stack + 1), "d"(stack + 4),
                        [stack] "r"(stack), [entry] "r"(entry) : "memory");
    __builtin_unreachable();
}
#endif

__attribute__((used, noreturn)) static void inspect(const word *stack)
{
    const char *prefix = "/Applications/elf-pie.sunapp/Program/probe";
#ifdef C_DSO_FIXTURE
    if (!((word)stack & 15) && stack[0] == 2 && stack[1] &&
        equal((const char *)stack[1], HELIOS_C_DSO_PATH)) {
        const word app_base = 0x20000000;
        const word entry = *(const word *)(app_base + 24) + app_base;
        if (!stack[2] || stack[3] || stack[4] ||
            stack[13] != 9 || stack[14] != entry ||
            stack[25] != 7 || stack[26] != 0x80000000)
            goto fail;
        enter_c_dso(stack, entry);
    }
#endif
    if (((word)stack & 15) || stack[0] != 1 || !stack[1] || stack[2])
        goto fail;
    const char *argv0 = (const char *)stack[1];
    const word *aux = stack + 3;
    unsigned envc = 0;
    int found_env = 0;
#ifdef MALLOC_FIXTURE
    int reject_malloc = 0;
#endif
    for (; *aux; ++aux) {
        const char *env = (const char *)*aux;
#ifdef MALLOC_FIXTURE
        if (equal(env, "HELIOS_MALLOC_SETUP=reject"))
            reject_malloc = 1;
#endif
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
    const int shared = equal(argv0, "/Applications/elf-shared.sunapp/Program/probe") ||
        equal(argv0, "/Applications/elf-real.sunapp/Program/probe") ||
        equal(argv0, "/Applications/elf-graph.sunapp/Program/probe");
    const int tls = equal(argv0, "/Applications/elf-tls.sunapp/Program/probe");
    const int tls_init = equal(argv0, "/Applications/elf-tls-init.sunapp/Program/probe");
    const int tls_shared = equal(argv0, "/Applications/elf-tls-shared.sunapp/Program/probe");
    const int relocate = shared || tls || tls_init || tls_shared || equal(argv0, "/Applications/elf-relocation.sunapp/Program/probe");
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
    if (tls || tls_init) {
        unsigned initial;
        if (tls)
            __asm__ volatile("movl %%fs:-8,%0" : "=r"(initial) :: "memory");
        else
            __asm__ volatile("movl %%fs:-4,%0" : "=r"(initial) :: "memory");
        static const char ready[] = "[HELIOS-TLS] interpreter initial=17\n";
        static const char control[] = "[HELIOS-TLS-INIT] interpreter initial=17\n";
        long length = tls ? sizeof(ready) - 1 : sizeof(control) - 1;
        if (initial != 17 || syscall3(1, 1, (long)(tls ? ready : control), length) != length)
            goto fail;
    }
    if (relocate) {
        static const char ready[] = "[HELIOS-RELOC] interpreter verified initial stack; entering application\n";
        if (syscall3(1, 1, (long)ready, sizeof(ready) - 1) != sizeof(ready) - 1)
            goto fail;
        __asm__ volatile("mov %0, %%rsp; xor %%edx, %%edx; jmp *%1"
                         :: "r"(stack), "r"(values[9]) : "rdx", "memory");
        __builtin_unreachable();
    }
#ifdef MALLOC_FIXTURE
    if (reject_malloc && equal(argv0, prefix))
        reject_setup();
#endif
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
