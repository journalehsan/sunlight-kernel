#ifdef GRAPH_SHARED
extern int helios_graph_a(int value);
#define PROBE_RESULT helios_graph_a(9)
#define PROBE_TAG "[HELIOS-GRAPH]"
#elif defined(REAL_SHARED)
extern int helios_scale(int value);
#define PROBE_RESULT helios_scale(9)
#define PROBE_TAG "[HELIOS-REAL]"
#else
extern int helios_fixture_value(void);
#define PROBE_RESULT helios_fixture_value()
#define PROBE_TAG "[HELIOS-SHARED]"
#endif

static void print(const char *message, unsigned long length)
{
    long result = 1;
    __asm__ volatile("syscall" : "+a"(result) : "D"(1), "S"(message),
                     "d"(length) : "rcx", "r11", "memory");
}

__asm__(".text\n.globl _start\n_start:\ncall entry\nud2\n");

__attribute__((used, noreturn)) static void entry(void)
{
    static const char start[] = PROBE_TAG " application entered\n";
    static const char success[] = PROBE_TAG " result=42\n";
    print(start, sizeof(start) - 1);
    int correct = PROBE_RESULT == 42;
    if (correct)
        print(success, sizeof(success) - 1);
    __asm__ volatile("syscall" :: "a"(60L), "D"((long)!correct)
                     : "rcx", "r11", "memory");
    __builtin_trap();
}
