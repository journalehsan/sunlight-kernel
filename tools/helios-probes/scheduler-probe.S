/* Native syscall ABI. Role 0 spins until killed; role 1 yields and self-exits.
 * Preserve callee-saved integer and SSE cookies across every real handoff. */
.global _start
.section .text
_start:
    movabs $0x123456789abcdef0, %r12
    movq %r12, %xmm0
    test %edi, %edi
    jz spin
    mov $32, %ebx
again:
    mov $21, %eax
    syscall
    movq %xmm0, %rdx
    cmp %r12, %rdx
    jne failed
    dec %ebx
    jnz again
    xor %edi, %edi
    mov $20, %eax
    syscall
    ud2
failed:
    mov $99, %edi
    mov $20, %eax
    syscall
    ud2
spin:
    movq %xmm0, %rdx
    cmp %r12, %rdx
    jne failed
    pause
    jmp spin
.section .note.GNU-stack,"",@progbits
