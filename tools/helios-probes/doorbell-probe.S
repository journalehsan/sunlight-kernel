.global _start
.section .text
_start:
    movabs $0x1735abcd98765432, %r12
    movq %r12, %xmm0
    test %edi, %edi
    jnz worker
consumer:
    xor %edi, %edi
    mov $0xf1a0, %eax
    syscall
    movq %xmm0, %rdx
    cmp %r12, %rdx
    jne failed
    mov $1, %edi
    mov $0xf1a0, %eax
    syscall
    test %eax, %eax
    jnz consumer
    jmp exit_ok
worker:
    mov $5000, %ecx
compute:
    rol $7, %r13
    add %r12, %r13
    dec %ecx
    jnz compute
    mov $2, %edi
    mov $0xf1a0, %eax
    syscall
    test %eax, %eax
    jnz exit_ok
    mov $21, %eax
    syscall
    movq %xmm0, %rdx
    cmp %r12, %rdx
    jne failed
    jmp worker
exit_ok:
    xor %edi, %edi
    mov $20, %eax
    syscall
    ud2
failed:
    mov $99, %edi
    mov $20, %eax
    syscall
    ud2
.section .note.GNU-stack,"",@progbits
