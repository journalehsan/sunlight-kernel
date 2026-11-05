/* Bounded replacement-image probe. Each image validates the canonical stack,
 * register conveniences, copied argv/envp, and stable PID before replacing itself.
 * The final image execs the originally failing /bin/true payload. */
.global _start
.section .text
_start:
    test $15, %rsp
    jnz failed
    cmpq $1, (%rsp)
    jne failed
    cmp $1, %rdi
    jne failed
    lea 8(%rsp), %rax
    cmp %rax, %rsi
    jne failed
    lea 24(%rsp), %rax
    cmp %rax, %rdx
    jne failed
    cmpq $0, 16(%rsp)
    jne failed
    cmpq $0, 32(%rsp)
    jne failed
    mov (%rsi), %r12
    cmpb $0, 1(%r12)
    jne failed
    movzbl (%r12), %r13d
    cmp $'1', %r13d
    jb failed
    cmp $'8', %r13d
    ja failed
    mov (%rdx), %rax
    cmpl $0x6f3d3252, (%rax) /* R2=o */
    jne failed
    cmpw $0x006b, 4(%rax)
    jne failed
    mov $33, %eax
    syscall
    cmp $61953, %rax
    jne failed
    /* A missing image must return to this image with the stack intact. */
    lea missing(%rip), %rdi
    lea empty(%rip), %rsi
    xor %edx, %edx
    mov $31, %eax
    syscall
    cmp $-1, %rax
    jne failed
    cmpq $1, (%rsp)
    jne failed
    lea checkpoint(%rip), %rdi
    mov $checkpoint_end-checkpoint, %esi
    mov $99, %eax
    syscall
    cmp $'1', %r13d
    je final
    decb (%r12)
    lea self_path(%rip), %rdi
    lea 8(%rsp), %rsi
    lea 24(%rsp), %rdx
    mov $31, %eax
    syscall
    jmp failed /* successful exec must never return */
final:
    lea done(%rip), %rdi
    mov $done_end-done, %esi
    mov $99, %eax
    syscall
    lea true_path(%rip), %rdi
    lea empty(%rip), %rsi
    xor %edx, %edx
    mov $31, %eax
    syscall
failed:
    mov $99, %edi
    mov $20, %eax
    syscall
    ud2
.section .rodata
self_path: .asciz "/test/exec-r2"
true_path: .asciz "/bin/true"
missing: .asciz "/test/exec-r2-missing"
checkpoint: .ascii "[EXEC-R2] user entry stack argv env PID and failed-exec PASS"
checkpoint_end:
done: .ascii "[EXEC-R2] eight replacements PASS; entering original /bin/true"
done_end:
.balign 8
empty: .quad 0
.section .note.GNU-stack,"",@progbits
