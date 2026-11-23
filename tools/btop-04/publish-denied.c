#include <stdio.h>

int main(void)
{
    long result = 96; /* Native NetInfo */
    register unsigned long r8 __asm__("r8") = 0;
    register unsigned long r9 __asm__("r9") = 0;
    register unsigned long r10 __asm__("r10") = 0;
    __asm__ volatile("syscall" : "+a"(result)
                     : "D"(0UL), "S"(0UL), "d"(5UL), "r"(r8), "r"(r9), "r"(r10)
                     : "rcx", "r11", "memory");
    printf("untrusted native publication=%ld\n", result);
    return result == -1 ? 0 : 1;
}
