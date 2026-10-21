/* Native syscall ABI. Role 0 spins until killed; role 1 yields and self-exits.
 * Preserve callee-saved integer and SSE cookies across every real handoff. */
.global _start
.section .text
_start:
    movabs $0x123456789abcdef0, %r12
    movq %r12, %xmm0
    xor %ebp, %ebp
    cmp $4, %edi
    jne ordinary
    inc %ebp
    jmp once
ordinary:
    cmp $2, %edi
    je exit_ok
    cmp $3, %edi
    je once
    test %edi, %edi
    jz spin
    mov $32, %ebx
    jmp again
once:
    mov $1, %ebx
again:
    mov $21, %eax
    syscall
    movq %xmm0, %rdx
    cmp %r12, %rdx
    jne failed
    dec %ebx
    jnz again
exit_ok:
    test %ebp, %ebp
    jz self_exit
    ud2
self_exit:
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
