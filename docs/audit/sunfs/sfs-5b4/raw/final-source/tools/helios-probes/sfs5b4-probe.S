.global _start
.section .text
_start:
    mov %rdi, %r12
loop:
    mov %r12, %rdi
    mov $0xf5b4, %eax
    syscall
    test %eax, %eax
    jnz exit
    mov $21, %eax
    syscall
    jmp loop
exit:
    xor %edi, %edi
    mov $20, %eax
    syscall
    ud2
.section .note.GNU-stack,"",@progbits
