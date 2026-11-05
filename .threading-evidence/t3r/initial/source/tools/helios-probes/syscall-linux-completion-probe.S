/* Isolated Linux clone/futex/clear-child-TID reuse control. Bounded by the
 * host guest deadline. Child returns through the real syscall exit path. */
.global _start
.section .text
_start:
    mov $32, %r12d
next_child:
    movl $55, child_tid(%rip)
    mov $56, %eax
    mov $0x7d0f00, %edi
    lea child_stack_top(%rip), %rsi
    lea parent_tid(%rip), %rdx
    lea child_tid(%rip), %r10
    lea tls(%rip), %r8
    syscall
    test %rax, %rax
    js failed
    jz child
    mov $65536, %r13d
wait_clear:
    cmpl $0, child_tid(%rip)
    je reuse
    mov $202, %eax
    lea child_tid(%rip), %rdi
    mov $128, %esi
    mov $55, %edx
    xor %r10d, %r10d
    syscall
    dec %r13d
    jnz wait_clear
    jmp failed
reuse:
    /* Immediately destroy the completed child's entire user stack and TLS.
     * The next clone reuses those exact virtual addresses. */
    lea child_stack(%rip), %rdi
    mov $8192, %ecx
    movabs $0xdeadbeefdeadbeef, %rax
    rep stosq
    lea tls(%rip), %rdi
    mov $4, %ecx
    rep stosq
    dec %r12d
    jnz next_child
    xor %edi, %edi
    mov $60, %eax
    syscall
    ud2
child:
    /* The final user TLS access is strictly before syscall entry. */
    movq $42, %fs:0
    mov $24, %eax
    syscall
    cmpq $42, %fs:0
    jne failed
    xor %edi, %edi
    xor %esp, %esp
    mov $60, %eax
    syscall
    ud2
failed:
    mov $99, %edi
    mov $60, %eax
    syscall
    ud2
.section .bss
.balign 16
tls: .skip 32
child_tid: .skip 4
parent_tid: .skip 4
.balign 16
child_stack: .skip 65536
child_stack_top:
.section .note.GNU-stack,"",@progbits
