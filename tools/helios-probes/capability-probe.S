/* Native ABI security fixture. Secrets are passed only to owner/authorized
 * reader at launch. Attacker receives no private object/endpoint identifiers.
 * rdi role: 0 owner, 1 attacker/stress, 2 authorized reader. */
.global _start
.section .text
_start:
    cmp $0, %edi
    je owner
    cmp $2, %edi
    je reader

    /* Obtain a legitimate endpoint in this process, then try the historical
     * 62 low-mask predictions against private SEND admission. */
    mov $10, %eax
    syscall
    test %rax, %rax
    jz failed
    mov %rax, %rbp
    mov $1, %ebx
    mov $62, %r15d
guess_ipc:
    mov %rbp, %rsi
    xor %rbx, %rsi
    xor %edi, %edi
    xor %edx, %edx
    mov $1, %eax
    syscall
    cmp $1, %rax /* InvalidCapability */
    jne failed
    shl $1, %rbx
    or $1, %rbx
    dec %r15d
    jnz guess_ipc

    /* Own SHM token, no victim handle: every guessed mapping must fail. */
    mov $92, %eax
    mov $4096, %edi
    xor %esi, %esi
    syscall
    cmp $-1, %rax
    je failed
    mov %r13, %rbx
    mov $1, %r15
    mov $62, %ebp
guess_shm:
    mov %rbx, %rdi
    xor %r15, %rdi
    mov $93, %eax
    syscall
    cmp $-1, %rax
    jne failed
    shl $1, %r15
    or $1, %r15
    dec %ebp
    jnz guess_shm
    mov %rbx, %rdi
    mov $94, %eax
    syscall
    test %rax, %rax
    jnz failed
    /* Repeated endpoint issuance, attenuation, destruction and stale SEND. */
    mov $128, %ebp
stress:
    mov $10, %eax
    syscall
    test %rax, %rax
    jz failed
    mov %rax, %rbx
    mov %rax, %rdi
    mov $11, %eax
    syscall
    test %rax, %rax
    jz failed
    mov %rax, %r15
    mov %r15, %rsi
    mov $4, %eax /* SEND derivation must not authorize RECEIVE */
    syscall
    cmp $1, %rax
    jne failed
    mov %r15, %rdi
    mov $13, %eax /* or owner destruction */
    syscall
    cmp $1, %rax
    jne failed
    mov %rbx, %rdi
    mov $13, %eax
    syscall
    test %rax, %rax /* endpoint_destroy returns 0 on success */
    jnz failed
    mov %r15, %rsi
    xor %edi, %edi
    xor %edx, %edx
    mov $1, %eax
    syscall
    cmp $1, %rax
    jne failed
    dec %ebp
    jnz stress
    jmp success

reader:
    mov %rcx, %r13 /* optional known-blocked exec diagnostic */
    mov %rsi, %rbx /* explicitly delegated SHM */
    mov %rdx, %rbp /* explicitly delegated SEND */
    mov %rsi, %rdi
    mov $93, %eax
    syscall
    cmp $-1, %rax
    je failed
    cmpq $0x51a7, (%rax)
    jne failed
    movq $0x51a7, (%rax) /* authorized legacy RW */
    mov %rbx, %rdi
    mov $94, %eax
    syscall
    test %rax, %rax
    jnz failed
    mov $16, %r14d
reader_call:
    mov %rbp, %rsi
    mov $0x123, %edi
    xor %edx, %edx
    mov $1, %eax
    syscall
    cmp $3, %rax
    jne reader_reply
    mov $21, %eax
    syscall
    jmp reader_call
reader_reply:
    test %rax, %rax
    jnz failed
    cmp $0x456, %rdi
    jne failed
    dec %r14d
    jnz reader_call
    test %r13, %r13
    jz success
    /* Exec must withdraw the old image's 32 unconsumed grants before the
     * replacement exits. The opt-in kernel gate checks that exact boundary. */
    lea exec_path(%rip), %rdi
    lea exec_argv(%rip), %rsi
    lea exec_envp(%rip), %rdx
    mov $31, %eax
    syscall
    cmp $-1, %rax
    je failed
    jmp success

owner:
    mov %rsi, %rbx /* private mapped address */
    mov %rdx, %rbp /* private receive token */
    mov %rcx, %r15 /* authorized reader PID */
    mov $16, %r14d
owner_recv:
    cmpq $0x51a7, (%rbx)
    jne failed
    mov %rbp, %rsi
    mov $4, %eax
    syscall
    cmp $3, %rax
    je owner_recv
    test %rax, %rax
    jnz failed
    cmp %r15, %rsi /* kernel badge must be exact authorized reader */
    jne failed
    cmp $0x123, %rdi
    jne failed
    /* Mint actual recipient-bound grants while handling the exact pending
     * request. Leave them outstanding: recipient exit must withdraw all 32. */
    mov %r15, %rdi
    xor %esi, %esi
    xor %edx, %edx
    mov $102, %eax
    syscall
    cmp $-1, %rax
    je failed
    mov %r15, %rdi
    mov $2, %esi /* GroupCreate */
    xor %edx, %edx
    mov $1, %r12d
    xor %r8d, %r8d
    xor %r9d, %r9d
    mov $151, %eax
    syscall
    cmp $-1, %rax
    je failed
    mov $0x456, %edi
    xor %edx, %edx
    mov $2, %eax
    syscall
    dec %r14d
    jnz owner_recv
owner_wait_reaped:
    cmpq $0xc1ea, (%rbx)
    je owner_late_grants
    mov $21, %eax
    syscall
    jmp owner_wait_reaped
owner_late_grants:
    mov %r15, %rdi
    xor %esi, %esi
    xor %edx, %edx
    mov $102, %eax
    syscall
    cmp $-1, %rax
    jne failed
    mov %r15, %rdi
    mov $2, %esi
    xor %edx, %edx
    mov $1, %r12d
    xor %r8d, %r8d
    xor %r9d, %r9d
    mov $151, %eax
    syscall
    cmp $-1, %rax
    jne failed
    jmp success
success:
    xor %edi, %edi
    mov $20, %eax
    syscall
    ud2
failed:
    mov $99, %edi
    mov $20, %eax
    syscall
    ud2
.section .rodata
exec_path: .asciz "/test/exec-r2"
exec_count: .asciz "8"
exec_env: .asciz "R2=ok"
.balign 8
exec_argv: .quad exec_count, 0
exec_envp: .quad exec_env, 0
.balign 8
empty_argv: .quad 0
.section .note.GNU-stack,"",@progbits
