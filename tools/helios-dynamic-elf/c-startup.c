#include "c-startup.h"
#ifndef HELIOS_C_DSO
#include "c-startup-cases.h"
#endif
#include "bounded-malloc.h"

enum { WRITE = 1, MAP = 9, UNMAP = 11, GETPID = 39, EXIT = 60 };
#define STACK_TOP 0x00007ffffffff000UL
#define STACK_WINDOW 4096UL

extern const unsigned char __ehdr_start[];
extern void _start(void);

static long call(long number, unsigned long a, unsigned long b, unsigned long c,
                 unsigned long d, long e)
{
    register unsigned long arg4 __asm__("r10") = d;
    register long arg5 __asm__("r8") = e;
    register unsigned long arg6 __asm__("r9") = 0;
    __asm__ volatile("syscall" : "+a"(number) : "D"(a), "S"(b), "d"(c),
                     "r"(arg4), "r"(arg5), "r"(arg6) : "rcx", "r11", "memory");
    return number;
}

#define LOG(message) (call(WRITE, 1, (unsigned long)(message), sizeof(message) - 1, 0, 0) \
                      == (long)(sizeof(message) - 1))

static long map_pages(unsigned long length)
{
    return call(MAP, 0, length, 3, 0x22, -1);
}

static long unmap_pages(unsigned long base, unsigned long length)
{
    return call(UNMAP, base, length, 0, 0, 0);
}

static void __attribute__((noreturn)) fatal(void)
{
    call(EXIT, 71, 0, 0, 0, 0);
    __builtin_trap();
}

#ifndef HELIOS_C_DSO
static long reject_map(unsigned long length)
{
    (void)length;
    return PAGE_NO_MEMORY;
}

static const struct page_backend backend = {map_pages, unmap_pages, 0, fatal};
static const struct page_backend failing_backend = {reject_map, unmap_pages, 0, fatal};

static void append_decimal(char **cursor, unsigned long value)
{
    char digits[20];
    unsigned int count = 0;
    do {
        digits[count++] = '0' + value % 10;
        value /= 10;
    } while (value);
    while (count)
        *(*cursor)++ = digits[--count];
}

static int trace_values(unsigned long pid, char mode, const unsigned long *words)
{
    char line[256] = "[C-STARTUP-VALUES] pid=";
    char *cursor = line + sizeof("[C-STARTUP-VALUES] pid=") - 1;
    append_decimal(&cursor, pid);
    const char label[] = " mode=";
    for (unsigned int i = 0; i < sizeof(label) - 1; ++i)
        *cursor++ = label[i];
    *cursor++ = mode;
    const char fields[] = " stack_bytes,argv0_offset,argv1_offset,auxv_values=";
    for (unsigned int i = 0; i < sizeof(fields) - 1; ++i)
        *cursor++ = fields[i];
    const unsigned long base = (unsigned long)words;
    append_decimal(&cursor, STACK_TOP - base);
    *cursor++ = ',';
    append_decimal(&cursor, words[1] - base);
    *cursor++ = ',';
    append_decimal(&cursor, words[2] - base);
    for (unsigned int i = 0; i < sizeof(startup_tags) / sizeof(startup_tags[0]); ++i) {
        const unsigned long value = words[6 + 2*i];
        *cursor++ = ',';
        append_decimal(&cursor, startup_tags[i] == 31 || startup_tags[i] == 25 ? value - base : value);
    }
    *cursor++ = '\n';
    return call(WRITE, 1, (unsigned long)line, cursor - line, 0, 0) == cursor - line;
}

static int negative_startup(const unsigned char *stack, unsigned long base, unsigned long size,
                            struct startup_image image)
{
    unsigned long copy[STACK_WINDOW / sizeof(unsigned long)];
    unsigned char *bytes = (unsigned char *)copy;
    for (unsigned long i = 0; i < size; ++i)
        bytes[i] = stack[i];
    for (unsigned int i = 0; i < sizeof(startup_cases) / sizeof(startup_cases[0]); ++i)
        if (!reject_startup_case(copy, base, size, image, i))
            return 72;
    return LOG("[C-STARTUP] negatives=argc,argv-null,argv-pointer,argv-unterminated,envp-null,auxv-tag,auxv-value,auxv-null,alignment,overflow,truncated\n") ? 42 : 73;
}

#endif

#ifdef HELIOS_C_DSO
static int utility_probe(struct helios_heap *heap, struct helios_allocation handle)
{
    const struct utility_buffer buffer = {helios_resolve(heap, handle), handle.requested};
    unsigned long base = buffer.base;
    unsigned char *memory = (unsigned char *)base;
    if (helios_memset(buffer, base + 1, 'Q', 35) || memory[0] != 0x31 || memory[36] != 0x72)
        return 85;
    for (unsigned long i = 1; i < 36; ++i)
        if (memory[i] != 'Q')
            return 86;
    if (helios_memset(buffer, 0, 0, 1) != -14 ||
        helios_memset(buffer, ~0UL, 0, 2) != -75 ||
        helios_memcpy(buffer, base, 0, 1) != -14 ||
        helios_memcpy(buffer, base + 2, base + 1, 4) != -22 ||
        helios_strlen(buffer, base + 1, 35) != -61 ||
        helios_strlen(buffer, base + 36, 2) != -14 ||
        helios_write(buffer, base, 1, ~0UL) != -9 ||
        helios_memset(buffer, 0, 0, 0) || helios_memcpy(buffer, 0, 0, 0) ||
        helios_strlen(buffer, 0, 0) != -61 || helios_write(buffer, 0, 0, 1))
        return 87;
    if (memory[0] != 0x31 || memory[2] != 'Q' || memory[36] != 0x72)
        return 88;
    static const char marker[] = "HELIOS-C-DSO-OK\n";
    for (unsigned long i = 0; i < sizeof(marker); ++i)
        memory[1 + i] = marker[i];
    if (helios_memcpy(buffer, base + 18, base + 1, sizeof(marker)) ||
        helios_strlen(buffer, base + 18, sizeof(marker)) != sizeof(marker) - 1 ||
        memory[35] != 'Q' || memory[36] != 0x72)
        return 89;
    for (unsigned long i = 0; i < sizeof(marker); ++i)
        if (memory[18 + i] != (unsigned char)marker[i])
            return 89;
    if (!LOG("[C-DSO] buffer=slot-0 generation=1 bytes=37 src-offset=1 dst-offset=18 memset=0 memcpy=0 strlen=16 fault=-14 overflow=-75 unterminated=-61 overlap=-22 output-fault=-9 zero=checked\n"))
        return 90;
    return helios_write(buffer, base + 18, sizeof(marker) - 1, 1) == sizeof(marker) - 1 ? 0 : 91;
}
#endif

int helios_c_start(unsigned long base, unsigned long argc, unsigned long argv, unsigned long envp)
{
    if (base > STACK_TOP || STACK_TOP - base > STACK_WINDOW)
        return 70;
    const unsigned char *stack = (const unsigned char *)base;
    const struct startup_image image = {
        (unsigned long)__ehdr_start + 64,
        __ehdr_start[56] | (unsigned long)__ehdr_start[57] << 8,
        (unsigned long)_start,
    };
    const unsigned long size = STACK_TOP - base;
    char mode;
    if (parse_startup(stack, base, size, argc, argv, envp, image, &mode))
        return 74;

    /* Modify guest-owned stack storage and prove the two strings are disjoint. */
    volatile unsigned char *arg = (volatile unsigned char *)(STACK_TOP - START_RANDOM - sizeof(startup_name) - 2);
    arg[0] = 'X';
    if (arg[0] != 'X' || arg[2] != startup_name[0])
        return 75;
    arg[0] = mode;
#ifdef HELIOS_C_DSO
    if (mode != 'P' || !LOG("[C-DSO] _start argc=2 argv=verified envc=0 auxv=verified rsp_mod16=0 registers=match\n"))
        return 76;
#else
    if (!LOG("[C-STARTUP] argc=2 argv0=c-startup argv1=mode envc=0 auxv=3,4,5,6,9,11,12,13,14,23,31,25,0 rsp_mod16=0 argument_registers=match strings=guest-owned\n"))
        return 76;

#endif
    long pid = call(GETPID, 0, 0, 0, 0, 0);
    if (pid <= 0)
        return 77;
    struct helios_heap heap = {.pages.owner = (unsigned long)pid};
    struct helios_allocation handle = {0};
#ifdef HELIOS_C_DSO
    const struct page_backend backend = {map_pages, unmap_pages, 0, fatal};
#else
    if (!trace_values(pid, mode, (const unsigned long *)stack))
        return 77;
    if (mode == 'N')
        return negative_startup(stack, base, size, image);
    if (mode == 'A') {
        if (helios_malloc(&heap, &failing_backend, 37, 0, &handle) != PAGE_NO_MEMORY ||
            handle.requested || heap.pages.used || heap.pages.pages || heap.pages.generation ||
            heap.slots[0].requested)
            return 78;
        return LOG("[C-STARTUP] allocation-init=-12 owners=0 pages=0 unpublished=1\n") ? 43 : 76;
    }
#endif
    if (helios_malloc(&heap, &backend, 37, 0, &handle) || handle.backing.slot ||
        handle.backing.generation != 1 || heap.pages.used != 1 || heap.pages.pages != 1)
        return 79;
    volatile unsigned char *memory = (volatile unsigned char *)helios_resolve(&heap, handle);
    if (!memory || (unsigned long)memory % HELIOS_MALLOC_ALIGNMENT)
        return 80;
    for (unsigned long i = 0; i < PAGE_BYTES; ++i)
        if (memory[i])
            return 81;
    memory[0] = 0x31;
    memory[36] = 0x72;
    if (memory[0] != 0x31 || memory[36] != 0x72)
        return 82;
#ifndef HELIOS_C_DSO
    if (!LOG("[C-STARTUP] getpid=39 mmap=9 malloc=37 alignment=4096 zero=4096 markers=31,72 slot=0 generation=1\n"))
        return 76;
    if (mode == 'L')
        return LOG("[C-STARTUP] exit-live owners=1 pages=1 exit=60:0\n") ? 0 : 76;
    if (mode == 'W') {
        if (call(WRITE, 1, STACK_TOP, 1, 0, 0) != -14)
            return 83;
        return LOG("[C-STARTUP] write=1 fault=-14 exit-live owners=1 pages=1 exit=60:44\n") ? 44 : 76;
    }
#else
    const int utility_status = utility_probe(&heap, handle);
    if (utility_status)
        return utility_status;
#endif
    if (helios_free(&heap, &backend, handle) || helios_resolve(&heap, handle) ||
        helios_free(&heap, &backend, handle) != PAGE_INVALID || heap.pages.used ||
        heap.pages.pages || heap.slots[0].requested)
        return 84;
#ifdef HELIOS_C_DSO
    if (!LOG("[C-DSO] free=0 stale=-22 owners=0 pages=0 exit=60:0\n"))
        return 76;
    helios_exit(0);
#else
    return LOG("[C-STARTUP] output=HELIOS-C-STARTUP-OK write=1 munmap=11 free=0 stale=-22 owners=0 pages=0 exit=60:0\n") ? 0 : 76;
#endif
}
