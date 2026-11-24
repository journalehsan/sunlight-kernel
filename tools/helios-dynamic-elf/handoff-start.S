.text
.globl _start
_start:
    ud2
#ifdef FIXED_EXEC
.section .interp,"a",@progbits
.asciz "/lib/helios/ld-handoff.so.1"
#endif
.section .note.GNU-stack,"",@progbits
