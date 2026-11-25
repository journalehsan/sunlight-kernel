#include <assert.h>
#include <stdio.h>
#include <setjmp.h>
#include <string.h>

#include "c-startup.h"
#include "c-startup-cases.h"

static jmp_buf exit_context;
static _Alignas(16) unsigned long entry_stack[2048];
static unsigned long expected_stack;
static int status, entry_calls, exit_status;

_Noreturn void invoke_startup_entry(unsigned long, unsigned long, unsigned long, unsigned long);

int helios_c_start(unsigned long rsp, unsigned long argc, unsigned long argv, unsigned long envp)
{
    assert(rsp == expected_stack && argc == 2 && argv == rsp + 8 && envp == rsp + 32);
    ++entry_calls;
    return status;
}

_Noreturn void capture_startup_exit(int result)
{
    exit_status = result;
    longjmp(exit_context, 1);
}

static void check_exit(int requested, unsigned long skew, int expected, int calls)
{
    status = requested;
    expected_stack = (unsigned long)&entry_stack[2040] + skew;
    entry_calls = 0;
    if (!setjmp(exit_context))
        invoke_startup_entry(expected_stack, 2, expected_stack + 8, expected_stack + 32);
    assert(exit_status == expected && entry_calls == calls);
}

int main(void)
{
    unsigned long source[36] = {0}, copy[36];
    const unsigned long base = 0x10000, size = sizeof(source);
    const unsigned long random = base + size - START_RANDOM;
    const struct startup_image image = {0x400040, 4, 0x401000};
    source[0] = 2;
    source[1] = random - sizeof(startup_name);
    source[2] = source[1] - 2;
    memcpy((char *)source + source[1] - base, startup_name, sizeof(startup_name));
    memcpy((char *)source + source[2] - base, "P", 2);
    const unsigned long values[] = {
        image.phdr, 56, image.phnum, 4096, image.entry, 7, 7, 8, 8, 0, source[1], random, 0
    };
    for (unsigned int i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
        source[5 + 2*i] = startup_tags[i];
        source[6 + 2*i] = values[i];
    }
    memcpy(copy, source, size);
    memset(source, 0, size);
    char mode;
    assert(parse_startup(copy, base, size, 2, base + 8, base + 32, image, &mode) == START_OK);
    assert(mode == 'P');
    puts("ok copied-stack ownership and lifetime");

    for (unsigned int i = 0; i < sizeof(startup_cases) / sizeof(startup_cases[0]); ++i) {
        assert(reject_startup_case(copy, base, size, image, i));
        printf("ok %s\n", startup_cases[i].name);
    }
    assert(parse_startup(copy, base, size, 2, base + 16, base + 32, image, &mode) == START_ARGV);
    assert(parse_startup(copy, base, size, 2, base + 8, base + 40, image, &mode) == START_ENV);
    puts("ok argument registers");
    check_exit(0, 0, 0, 1);
    check_exit(43, 0, 43, 1);
    check_exit(0, 8, 70, 0);
    puts("ok entry exit status zero, failure and stack rejection");
    puts("16 tests passed");
}
