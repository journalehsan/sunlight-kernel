enum { MAP = 9, PROTECT = 10, UNMAP = 11, PAGE = 4096, READ = 1, RW = 3 };

static long call(long number, unsigned long address, unsigned long length,
                 unsigned long protection, unsigned long flags, long descriptor)
{
    register unsigned long arg4 __asm__("r10") = flags;
    register long arg5 __asm__("r8") = descriptor;
    register unsigned long arg6 __asm__("r9") = 0;
    __asm__ volatile("syscall" : "+a"(number) : "D"(address), "S"(length),
                     "d"(protection), "r"(arg4), "r"(arg5), "r"(arg6)
                     : "rcx", "r11", "memory");
    return number;
}

#define LOG(message) (call(1, 1, (unsigned long)(message), sizeof(message) - 1, 0, 0) \
                      == sizeof(message) - 1)

#ifdef PAGE_ALLOCATOR_FIXTURE
#include "page-allocator-probe.c"
#endif

static int live_probe(unsigned char marker)
{
    const unsigned long length = (marker == '4' || marker == '8') ? 2 * PAGE : PAGE + 1;
    volatile unsigned char *page;
    long base = call(MAP, 0, length, RW, 0x22, -1);
    if (base <= 0 || (base & (PAGE - 1)) != 0)
        return 1;
    page = (volatile unsigned char *)base;
    if (page[0] || page[2 * PAGE - 1])
        return 1;
    page[0] = marker;
    page[2 * PAGE - 1] = marker;
    if (call(PROTECT, base, PAGE + 1, READ, 0, 0) ||
        page[0] != marker || page[2 * PAGE - 1] != marker ||
        call(PROTECT, base, PAGE + 1, RW, 0, 0))
        return 1;

    long scratch = call(MAP, 0, PAGE + 1, READ, 0x22, -1);
    if (scratch != base + 2 * PAGE ||
        call(UNMAP, scratch, PAGE + 1, 0, 0, 0) ||
        call(PROTECT, scratch + PAGE, PAGE, READ, 0, 0) != -12)
        return 1;
    char message[] = "[ANON-MMAP-LIVE] zero=both marker=? exit=live\n";
    message[sizeof("[ANON-MMAP-LIVE] zero=both marker=") - 1] = (char)marker;
    return !LOG(message);
}

static int exhaust_probe(void)
{
    enum { REGION_LIMIT = 256 };
    volatile unsigned char *pages[REGION_LIMIT];
    unsigned int count = 0;
    for (; count < REGION_LIMIT; ++count) {
        long protection = count % 2 ? READ : RW;
        long address = call(MAP, 0, PAGE, protection, 0x22, -1);
        if (address == -12)
            break;
        if (address <= 0 || (address & (PAGE - 1)))
            return 1;
        pages[count] = (volatile unsigned char *)address;
        if (*pages[count])
            return 1;
        if (protection == RW)
            *pages[count] = '6';
    }
    if (count == 0 || count == REGION_LIMIT)
        return 1;
    unsigned long rejected = (unsigned long)pages[count - 1] + PAGE;
    if (call(PROTECT, rejected, PAGE, READ, 0, 0) != -12)
        return 1;
    for (unsigned int i = 0; i < count; ++i) {
        if (*pages[i] != (i % 2 ? 0 : '6'))
            return 1;
    }
    return !LOG("[ANON-MMAP-LIVE] exhausted errno=-12 existing=valid stale=-12 exit=live\n");
}

static int fault_probe(unsigned char marker)
{
    long base = call(MAP, 0, 2 * PAGE, RW, 0x22, -1);
    if (base <= 0 || (base & (PAGE - 1)))
        return 1;
    volatile unsigned char *page = (volatile unsigned char *)base;
    if (page[0] || page[PAGE])
        return 1;
    page[0] = marker;
    page[PAGE] = marker;

    if (marker == 'P') {
        if (call(PROTECT, base, 2 * PAGE, READ, 0, 0) != -12 ||
            page[0] != marker || page[PAGE] != marker)
            return 1;
        page[0] = 'p';
        page[PAGE] = 'p';
        if (page[0] != 'p' || page[PAGE] != 'p' ||
            call(UNMAP, base, 2 * PAGE, 0, 0, 0) ||
            call(PROTECT, base, 2 * PAGE, READ, 0, 0) != -12)
            return 1;
        return !LOG("[ANON-MMAP-FAULT-GUEST] protect rc=-12 markers=P,P writable=both unmapped=both\n");
    }
    if (marker == 'S') {
        if (call(MAP, 0, PAGE + 1, READ, 0x22, -1) != -22 ||
            page[0] != marker || page[PAGE] != marker ||
            call(PROTECT, base + 2 * PAGE, PAGE, READ, 0, 0) != -12 ||
            !LOG("[ANON-MMAP-FAULT-GUEST] setup rc=-22 markers=S,S exit=42 live=both\n"))
            return 1;
        return 42;
    }
    return !LOG("[ANON-MMAP-FAULT-GUEST] replacement zero=both old-marker=absent\n");
}

__attribute__((used, noreturn)) static void probe(unsigned long *stack)
{
    const char *const *environment = (const char *const *)(stack + stack[0] + 2);
    unsigned char marker = '8';
#ifdef PAGE_ALLOCATOR_FIXTURE
    int malloc_mode = 0;
#endif
#ifdef MALLOC_FIXTURE
    const char *foreign_token = 0;
#endif
    for (; *environment; ++environment) {
#ifdef MALLOC_FIXTURE
        if ((*environment)[0] == 'C' && (*environment)[1] == '=')
            foreign_token = *environment + 2;
        if ((*environment)[0] == 'H' && (*environment)[1] == '=' && (*environment)[2] == '1')
            malloc_mode = 1;
#endif
        if ((*environment)[0] == 'M' && (*environment)[1] == '=')
            marker = (*environment)[2];
    }
    long status = 1;
#ifdef MALLOC_FIXTURE
    if (marker == 'X' || marker == 'Y') {
        if ((marker == 'Y') == !!foreign_token)
            status = malloc_foreign_probe(foreign_token);
        goto finish;
    }
    if (marker == 'B' || marker == 'J') {
        status = marker == 'B' ? malloc_contract() : malloc_setup_failure();
        goto finish;
    }
#endif
#ifdef PAGE_ALLOCATOR_FIXTURE
    if ((marker >= 'a' && marker <= 'g') || marker == 'A' || marker == 'E' ||
        marker == 'F' || marker == 'T' || marker == 'L' || marker == 'Q') {
        status = allocator_probe(marker, malloc_mode);
        goto finish;
    }
#endif
    if (marker == 'P' || marker == 'S' || marker == 'Z') {
        status = fault_probe(marker);
        goto finish;
    }
    if (marker >= '4' && marker <= '8') {
        status = marker == '6' ? exhaust_probe() : live_probe(marker);
        goto finish;
    }
    if (marker < '1' || marker > '3')
        goto finish;
    if (call(MAP, 0, 0, RW, 0x22, -1) != -22 ||
        call(MAP, 0, ~0UL, RW, 0x22, -1) != -22 ||
        call(MAP, 0, PAGE, 8, 0x22, -1) != -22 ||
        call(MAP, 0, PAGE, RW, 0x20, -1) != -22 ||
        call(MAP, 0, PAGE, RW, 0x22, 0) != -22 ||
        !LOG("[ANON-MMAP] mmap invalid requests rejected\n"))
        goto finish;
    long mapping = call(MAP, 0, PAGE, RW, 0x22, -1);
    if (mapping <= 0 || (mapping & (PAGE - 1)) != 0)
        goto finish;
    unsigned long base = (unsigned long)mapping;
    volatile unsigned char *page = (volatile unsigned char *)base;
    if (*page != 0 ||
        !LOG("[ANON-MMAP] length=4096 mapping=slot-1 flags=0x22 fd=-1 offset=0 initial=RW zero=1\n"))
        goto finish;
    *page = marker;
    if (call(PROTECT, base + 1, PAGE, READ, 0, 0) != -22 ||
        call(PROTECT, base, ~0UL, READ, 0, 0) != -22 ||
        call(PROTECT, base, PAGE, 8, 0, 0) != -22 ||
        call(PROTECT, base, 2 * PAGE, READ, 0, 0) != -12 ||
        !LOG("[ANON-MMAP] mprotect invalid requests rejected\n"))
        goto finish;
    if (call(PROTECT, base, PAGE, READ, 0, 0) != 0 || *page != marker ||
        !LOG("[ANON-MMAP] mprotect result=0 requested=R readback=match\n"))
        goto finish;
    if (call(PROTECT, base, PAGE, RW, 0, 0) != 0 || *page != marker)
        goto finish;
    *page = marker;
    char readback[] = "[ANON-MMAP] mprotect result=0 requested=RW marker=?\n";
    readback[sizeof(readback) - 3] = (char)*page;
    if (!LOG(readback))
        goto finish;
    if (call(UNMAP, base + 1, PAGE, 0, 0, 0) != -22 ||
        call(UNMAP, base, ~0UL, 0, 0, 0) != -22 || *page != marker ||
        call(UNMAP, base, PAGE, 0, 0, 0) != 0 ||
        call(PROTECT, base, PAGE, READ, 0, 0) != -12 ||
        call(UNMAP, base, PAGE, 0, 0, 0) != 0 ||
        !LOG("[ANON-MMAP] munmap result=0 repeat=0 stale-mprotect=-12 invalid=-22\n"))
        goto finish;
    status = !LOG("[ANON-MMAP] exit=0\n");
finish:
#ifdef MALLOC_FIXTURE
    if (marker == 'X')
        goto exit;
#endif
    if (status == 1)
        (void)LOG("[ANON-MMAP] exit=1\n");
#ifdef MALLOC_FIXTURE
exit:
#endif
    call(60, status, 0, 0, 0, 0);
    __builtin_unreachable();
}

__asm__(".text\n.globl _start\n_start:\n"
        "mov %rsp,%rdi\n"
        "and $-16,%rsp\n"
        "call probe\n");
