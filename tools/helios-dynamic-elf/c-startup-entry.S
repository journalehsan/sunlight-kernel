.section .text
.global _start
.type _start, @function
_start:
    mov %rdx, %rcx
    mov %rsi, %rdx
    mov %rdi, %rsi
    mov %rsp, %rdi
    test $15, %rsp
    jnz .Lbad_alignment
    call helios_c_start
    mov %eax, %edi
    jmp .Lexit
.Lbad_alignment:
    mov $70, %edi
.Lexit:
#ifdef HELIOS_STARTUP_HOST_TEST
    and $-16, %rsp
    call capture_startup_exit
#else
    mov $60, %eax
    syscall
#endif
    ud2
.size _start, .-_start
#ifdef HELIOS_STARTUP_HOST_TEST
.global invoke_startup_entry
.type invoke_startup_entry, @function
invoke_startup_entry:
    mov %rdi, %rsp
    mov %rsi, %rdi
    mov %rdx, %rsi
    mov %rcx, %rdx
    jmp _start
.size invoke_startup_entry, .-invoke_startup_entry
#endif
.section .note.GNU-stack,"",@progbits
