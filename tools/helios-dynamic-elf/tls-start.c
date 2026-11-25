static __thread volatile unsigned value __attribute__((tls_model("local-exec"))) = 17;
#ifdef TLS_BSS
static __thread volatile unsigned tail __attribute__((tls_model("local-exec")));
#endif

__asm__(".text\n.globl _start\n_start:\ncall entry\nud2\n");

__attribute__((used, noreturn)) static void entry(void)
{
    unsigned initial = value;
#ifdef TLS_BSS
    unsigned zero = tail;
    tail = 53;
    unsigned tail_first = tail;
    tail += 14;
    unsigned tail_second = tail;
#endif
    value = 29;
    unsigned first = value;
    value += 13;
    unsigned second = value;
    long correct = initial == 17 && first == 29 && second == 42;
#ifdef TLS_BSS
    correct = correct && zero == 0 && tail_first == 53 && tail_second == 67;
#endif
    if (correct) {
#ifdef TLS_BSS
        static const char marker[] = "[HELIOS-TLS] initial=17 write=29 read=29 modify=42 read=42 tail_initial=0 tail_write=53 tail_read=53 tail_modify=67 tail_read=67\n";
#else
        static const char marker[] = "[HELIOS-TLS] initial=17 write=29 read=29 modify=42 read=42\n";
#endif
        long result = 1;
        __asm__ volatile("syscall" : "+a"(result)
                         : "D"(1), "S"(marker), "d"(sizeof(marker) - 1)
                         : "rcx", "r11", "memory");
        correct = result == sizeof(marker) - 1;
    }
    __asm__ volatile("syscall" :: "a"(60L), "D"(!correct) : "rcx", "r11", "memory");
    __builtin_trap();
}
