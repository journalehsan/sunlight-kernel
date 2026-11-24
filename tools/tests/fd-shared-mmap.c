#if defined(HOST_POOL_PROBE) || defined(HOST_TEMP_POOL)
static const char name[] = "wl-shm-measurement.sock";
#else
static const char name[] = "/sunlight/native-unix/fd-shared";
#endif
#if !defined(POOL_FIXTURE) && (!defined(SCM_FIXTURE) || !defined(NATIVE))
static const char path[] = "/dev/sunlight-shared-page";
#endif

static long syscall6(long number, long first, long second, long third,
                     long fourth, long fifth, long sixth)
{
    register long r10 __asm__("r10") = fourth;
    register long r8 __asm__("r8") = fifth;
    register long r9 __asm__("r9") = sixth;
    __asm__ volatile("syscall" : "+a"(number)
                     : "D"(first), "S"(second), "d"(third), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return number;
}

#define call(number, first, second, third) syscall6(number, first, second, third, 0, 0, 0)

#ifdef NATIVE
enum { OPEN = 40, READ = 42, WRITE = 43, CLOSE = 41, MMAP = 50,
       YIELD = 21, EXIT = 20, LOG = 99, DENIED = -1, INVALID = -1, BAD_FD = -1 };
#else
enum { OPEN = 2, READ = 0, WRITE = 1, CLOSE = 3, MMAP = 9,
       YIELD = 24, EXIT = 60, LOG = (1UL << 63) | 99,
       DENIED = -13, INVALID = -22, BAD_FD = -9 };
#endif

static void log_line(const char *message)
{
    long length = 0;
    while (message[length]) ++length;
    call(LOG, (long)message, length, 0);
}

static void require(int condition, const char *message)
{
    if (condition) return;
    log_line(message);
    call(EXIT, 1, 0, 0);
    __builtin_trap();
}

static void send_stage(long stream, char stage)
{
    require(call(WRITE, stream, (long)&stage, 1) == 1, "[FD-SHARED] FAIL send stage");
}

static void expect_stage(long stream, char expected)
{
    char stage = 0;
    require(call(READ, stream, (long)&stage, 1) == 1 && stage == expected,
            "[FD-SHARED] FAIL receive stage");
}

#if !defined(SCM_FIXTURE) && !defined(POOL_FIXTURE)
static volatile unsigned char *map_page(void)
{
    long readonly = call(OPEN, (long)path, 0, 0);
    require(readonly >= 0, "[FD-SHARED] FAIL open read-only");
    require(syscall6(MMAP, 0, 4096, 3, 1, readonly, 0) == DENIED,
            "[FD-SHARED] FAIL writable mapping via read-only descriptor");
    require(call(CLOSE, readonly, 0, 0) == 0, "[FD-SHARED] FAIL close read-only");
    long fd = call(OPEN, (long)path, 2, 0);
    require(fd >= 0, "[FD-SHARED] FAIL open read/write");
    require(syscall6(MMAP, 0, 4096, 3, 1, fd, 1) == INVALID &&
            syscall6(MMAP, 0, 4096, 3, 1, fd, 4096) == INVALID &&
            syscall6(MMAP, 0, 8192, 3, 1, fd, 0) == INVALID,
            "[FD-SHARED] FAIL invalid offset or range accepted");
    long mapped = syscall6(MMAP, 0, 4096, 3, 1, fd, 0);
    require(mapped == 0x300000000L, "[FD-SHARED] FAIL mapping or rejected request consumed VA");
    require(call(CLOSE, fd, 0, 0) == 0, "[FD-SHARED] FAIL close mapped FD");
    require(syscall6(MMAP, 0, 4096, 3, 1, fd, 0) == BAD_FD,
            "[FD-SHARED] FAIL closed FD accepted");
    return (volatile unsigned char *)mapped;
}
#endif

#if !defined(POOL_FIXTURE) && (!defined(SCM_FIXTURE) || !defined(NATIVE))
static void set_pattern(volatile unsigned char *page, unsigned seed)
{
    for (unsigned index = 0; index < 4096; ++index)
        page[index] = (unsigned char)(seed + index * 19);
}
#endif

#if !defined(POOL_FIXTURE) && (!defined(SCM_FIXTURE) || defined(NATIVE))
static int matches_pattern(volatile unsigned char *page, unsigned seed)
{
    for (unsigned index = 0; index < 4096; ++index)
        if (page[index] != (unsigned char)(seed + index * 19)) return 0;
    return 1;
}
#endif

#ifdef SCM_FIXTURE
#ifdef NATIVE
static long recv_right(long stream, unsigned char expected)
{
    long result = call(185, stream, 0, 0);
    require(result >= 0 && result >> 32 == expected, "[FD-SHARED] FAIL receive marked byte");
    return (int)result;
}
#else
static long send_right(long stream, int fd, char byte, unsigned size, unsigned offset)
{
    struct { const void *base; unsigned long len; } iov = {&byte, 1};
    struct { unsigned long len; int level, type, fd, padding; } control = {20, 1, 1, fd, 0};
    struct {
        void *name;
        unsigned name_len;
        const void *iov;
        unsigned long iov_len;
        const void *control;
        unsigned long control_len;
        int flags;
    } msg = {0, 0, &iov, 1, (const char *)&control + offset, size, 0};
    return call(46, stream, (long)&msg, 0x4000);
}
#endif
#endif

#ifdef POOL_FIXTURE
#include "fd-shm-pool.h"
#else
static void run(void)
{
#ifdef NATIVE
    long listener = call(183, (long)name, sizeof name - 1, 0);
    require(listener >= 0, "[FD-SHARED] FAIL listen");
    long stream;
    do {
        stream = call(184, listener, 0, 0);
        if (stream == -11) call(YIELD, 0, 0, 0);
    } while (stream == -11);
    require(stream >= 0, "[FD-SHARED] FAIL accept");
#ifdef SCM_FIXTURE
    send_stage(stream, 'R');
    expect_stage(stream, 'P');
    require(call(185, stream, 0, 0) == -11, "[FD-SHARED] FAIL invalid send queued a byte or right");
    send_stage(stream, 'G');
    long coordination = stream;
    expect_stage(coordination, 'C');
    stream = call(184, listener, 0, 0);
    require(stream >= 0, "[FD-SHARED] FAIL accept transfer stream");
    unsigned char prefix[4] = {0};
    require(call(READ, stream, (long)prefix, sizeof prefix) == 1 && prefix[0] == 'B',
            "[FD-SHARED] FAIL read crossed the FD boundary");
    require(call(READ, stream, (long)prefix, sizeof prefix) == -95,
            "[FD-SHARED] FAIL ordinary read consumed marked byte");

    long filled[256];
    unsigned count = 0;
    long duplicate;
    while ((duplicate = call(45, stream, 0, 0)) >= 0)
        filled[count++] = duplicate;
    require(count > 0 && call(185, stream, 0, 0) == -24,
            "[FD-SHARED] FAIL full destination table");
    require(call(CLOSE, filled[--count], 0, 0) == 0, "[FD-SHARED] FAIL free FD slot");
    long fd = recv_right(stream, 'W');
    while (count)
        require(call(CLOSE, filled[--count], 0, 0) == 0, "[FD-SHARED] FAIL close duplicate");
    require(call(185, stream, 0, 0) == -11, "[FD-SHARED] FAIL duplicate delivery");
    long mapped = syscall6(MMAP, 0, 4096, 3, 1, fd, 0);
    require(mapped > 0, "[FD-SHARED] FAIL mmap transferred read/write FD");
    require(call(CLOSE, fd, 0, 0) == 0, "[FD-SHARED] FAIL close received FD");
    volatile unsigned char *page = (volatile unsigned char *)mapped;
    require(matches_pattern(page, 31), "[FD-SHARED] FAIL initial transferred pattern");
    log_line("[SCM-RIGHTS] sender FD closed before delivery; EMFILE retry preserved byte and FD");
    send_stage(stream, 'M');
    expect_stage(stream, 'U');
    require(matches_pattern(page, 173), "[FD-SHARED] FAIL mapped update");
    expect_stage(coordination, 'O');
    fd = recv_right(stream, 'O');
    require(syscall6(MMAP, 0, 4096, 3, 1, fd, 0) == DENIED,
            "[FD-SHARED] FAIL writable mmap of transferred read-only FD");
    mapped = syscall6(MMAP, 0, 4096, 1, 1, fd, 0);
    require(mapped > 0 && matches_pattern((volatile unsigned char *)mapped, 173),
            "[FD-SHARED] FAIL read-only mapping");
    require(call(CLOSE, fd, 0, 0) == 0, "[FD-SHARED] FAIL close read-only FD");
    log_line("[SCM-RIGHTS] observed mapping update and rejected writable mmap on read-only FD");
    send_stage(stream, 'D');
    expect_stage(coordination, 'X');
    require(call(CLOSE, stream, 0, 0) == 0, "[FD-SHARED] FAIL close with queued FD");
    long exit_code;
    do {
        exit_code = call(32, 712, 0, 0);
    } while (exit_code == -2);
    require(exit_code == 0, "[FD-SHARED] FAIL wait for sender");
    log_line("[SCM-RIGHTS] PASS");
#else
    volatile unsigned char *page = map_page();
    log_line("[FD-SHARED] native rejected invalid maps and closed backing FD");
    send_stage(stream, 'R');
    expect_stage(stream, 'H');
    require(matches_pattern(page, 31), "[FD-SHARED] FAIL native read Helios pattern");
    set_pattern(page, 97);
    send_stage(stream, 'N');
    long exit_code;
    do {
        exit_code = call(32, 712, 0, 0);
    } while (exit_code == -2);
    require(exit_code == 0, "[FD-SHARED] FAIL wait for Helios exit");
    require(matches_pattern(page, 173), "[FD-SHARED] FAIL native read after Helios exit");
    log_line("[FD-SHARED] PASS native read after Helios exit");
#endif
#else
    struct { unsigned short family; char path[108]; } address = {1, {0}};
    for (unsigned index = 0; index < sizeof name; ++index) address.path[index] = name[index];
    long stream = call(41, 1, 1, 0);
    require(stream >= 0, "[FD-SHARED] FAIL socket");
    long result;
    do {
        result = call(42, stream, (long)&address, sizeof name + 2);
        if (result == -111) call(YIELD, 0, 0, 0);
    } while (result == -111);
    require(result == 0, "[FD-SHARED] FAIL connect");
#ifdef SCM_FIXTURE
    expect_stage(stream, 'R');
    require(send_right(stream, -1, 'I', 24, 0) == -9 &&
            send_right(stream, -1, 'I', 19, 0) == -22 &&
            send_right(stream, -1, 'I', 20, 1) == -22,
            "[FD-SHARED] FAIL invalid FD or malformed control");
    log_line("[SCM-RIGHTS] invalid FD and malformed control rejected");
    send_stage(stream, 'P');
    expect_stage(stream, 'G');
    long coordination = stream;
    stream = call(41, 1, 1, 0);
    require(stream >= 0 && call(42, stream, (long)&address, sizeof name + 2) == 0,
            "[FD-SHARED] FAIL connect transfer stream");
    long fd = call(OPEN, (long)path, 2, 0);
    require(fd >= 0, "[FD-SHARED] FAIL sender open");
    long mapped = syscall6(MMAP, 0, 4096, 3, 1, fd, 0);
    require(mapped > 0, "[FD-SHARED] FAIL sender mmap");
    volatile unsigned char *page = (volatile unsigned char *)mapped;
    set_pattern(page, 31);
    send_stage(stream, 'B');
    require(send_right(stream, (int)fd, 'W', 24, 0) == 1,
            "[FD-SHARED] FAIL send writable FD");
    require(call(CLOSE, fd, 0, 0) == 0, "[FD-SHARED] FAIL close sender FD");
    long readonly = call(OPEN, (long)path, 0, 0);
    require(readonly == fd, "[FD-SHARED] FAIL reuse sender FD");
    require(send_right(stream, (int)readonly, 'J', 20, 0) == -11,
            "[FD-SHARED] FAIL pending right admission");
    send_stage(coordination, 'C');
    expect_stage(stream, 'M');
    set_pattern(page, 173);
    send_stage(stream, 'U');
    require(send_right(stream, (int)readonly, 'O', 20, 0) == 1,
            "[FD-SHARED] FAIL send read-only FD");
    send_stage(coordination, 'O');
    expect_stage(stream, 'D');
    require(send_right(stream, (int)readonly, 'Z', 24, 0) == 1,
            "[FD-SHARED] FAIL queue close case");
    require(call(CLOSE, readonly, 0, 0) == 0, "[FD-SHARED] FAIL close sender read-only FD");
    require(call(CLOSE, stream, 0, 0) == 0, "[FD-SHARED] FAIL close sender stream");
    send_stage(coordination, 'X');
#else
    volatile unsigned char *page = map_page();
    log_line("[FD-SHARED] Helios rejected invalid maps and closed backing FD");
    expect_stage(stream, 'R');
    require(matches_pattern(page, 0) == 0 && page[0] == 0, "[FD-SHARED] FAIL initial page");
    set_pattern(page, 31);
    send_stage(stream, 'H');
    expect_stage(stream, 'N');
    require(matches_pattern(page, 97), "[FD-SHARED] FAIL Helios read native update");
    log_line("[FD-SHARED] Helios observed native update");
    set_pattern(page, 173);
#endif
#endif
}
#endif

__attribute__((force_align_arg_pointer, noreturn))
void _start(void)
{
    run();
    call(EXIT, 0, 0, 0);
    __builtin_trap();
}
