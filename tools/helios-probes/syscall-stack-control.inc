/* Opt-in T1 controls, executed with no call/push while RSP is hostile.
 * Ordinary scheduler fixture roles continue below this block. */
    cmp $5, %edi
    je t1_victim
    cmp $6, %edi
    je t1_attacker
    mov %rsp, %r14
    mov $0x70000000, %esp        /* unmapped, checked by fixture setup */
    mov $33, %eax
    syscall
    cmp $0x70000000, %rsp
    jne failed
    xor %esp, %esp              /* first inaccessible boundary */
    mov $33, %eax
    syscall
    test %rsp, %rsp
    jne failed
    movabs $0x0000800000000000, %rsp /* noncanonical user RSP */
    mov $33, %eax
    syscall
    movabs $0x0000800000000000, %rdx
    cmp %rdx, %rsp
    jne failed
    /* Exact lowest mapped stack page: the push target below is unmapped. */
    movabs $0x00007fffffdff000, %rsp
    mov $33, %eax
    syscall
    movabs $0x00007fffffdff000, %rdx
    cmp %rdx, %rsp
    jne failed
    mov %r14, %rsp
    jmp t1_controls_done
t1_victim:
    mov %rsp, %r14
    mov $32, %r13d
1:
    mov %rsi, %rsp
    mov $21, %eax
    syscall
    mov %r14, %rsp
    dec %r13d
    jnz 1b
    cmpq $1, 8(%rdx)
    jne failed
    movq $1, (%rdx)
    jmp exit_ok
t1_attacker:
    mov $4096, %r13d
2:
    movq $1, 8(%rdx)
    mov $16, %ecx
    mov %rsi, %r8
3:
    sub $8, %r8
    movq $0x5a5a5a5a, (%r8)
    loop 3b
    cmpq $1, (%rdx)
    je exit_ok
    mov $21, %eax
    syscall
    dec %r13d
    jnz 2b
    jmp failed
t1_controls_done:
