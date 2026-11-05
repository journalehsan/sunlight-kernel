.global _start
.section .text
.macro query name
    mov $99,%eax
    lea \name(%rip),%rdi
    mov $(\name\()_end-\name),%esi
    syscall
.endm
.macro yield
    mov $21,%eax
    syscall
.endm
_start:
    xor %r15d,%r15d
1:  mov %r15,%rsi
    shl $14,%rsi
    lea stacks+16352(%rip),%rax
    add %rax,%rsi
    lea worker(%rip),%rdi
    mov %rdi,(%rsi)
    mov %r15,8(%rsi)
    mov %r15,%rdx
    shl $5,%rdx
    lea tls(%rip),%rax
    add %rax,%rdx
    mov $22,%eax
    syscall
    test %rax,%rax
    js failed
    lea tids(%rip),%rdi
    mov %rax,(%rdi,%r15,8)
    inc %r15d
    cmp $2,%r15d
    jb 1b
    mov $200000,%ebp
2:  lea tids(%rip),%r8
    mov (%r8),%rdi
    mov $156,%eax
    syscall
    cmp $1,%rax
    jne 3f
    mov tids+8(%rip),%rdi
    mov $156,%eax
    syscall
    cmp $1,%rax
    jne 3f
    query finish
    cmp $1,%rax
    je 4f
3:  yield
    dec %ebp
    jnz 2b
    jmp failed
4:  ud2
worker:
    test %rsi,%rsi
    jnz observer
    /* Only straight assembly and SYSCALL cross the preservation interval.
       RCX/R11 and RAX are ABI-clobbered and deliberately excluded. */
    movabs $0x123456789abcdef0,%rbx
    movabs $0x1020304050607080,%r12
    movabs $0x8877665544332211,%r13
    movabs $0x1122334455667788,%r14
    movabs $0x7162534435261708,%r15
    push %rbx
    fninit
    fldcw cw(%rip)
    fldl fp(%rip)
    movdqu vector(%rip),%xmm0
    movdqu vector2(%rip),%xmm15
    ldmxcsr mxcsr(%rip)
    fxsave64 before(%rip)
    mov $200000,%ebp
5:  query source
    cmp $1,%rax
    jne 6f
    yield
    dec %ebp
    jnz 5b
    jmp failed
6:  test %rax,%rax
    jne failed
    /* The observer changes affinity only after WAIT has descheduled us. */
    lea futex(%rip),%rdi
    xor %esi,%esi
    xor %edx,%edx
    mov $-1,%r10
    mov $155,%eax
    syscall
    test %rax,%rax
    jne failed
    fxsave64 after(%rip)
    /* Compare only architecturally defined seeded FP state, ignoring FIP,
       FDP and reserved FXSAVE padding. No floating point call ABI is crossed. */
    lea before(%rip),%rsi
    lea after(%rip),%rdi
    mov $5,%ecx
    repe cmpsb
    jne failed
    mov before+24(%rip),%eax
    cmp after+24(%rip),%eax
    jne failed
    lea before+32(%rip),%rsi
    lea after+32(%rip),%rdi
    mov $10,%ecx
    repe cmpsb
    jne failed
    lea before+160(%rip),%rsi
    lea after+160(%rip),%rdi
    mov $16,%ecx
    repe cmpsb
    jne failed
    lea before+400(%rip),%rsi
    lea after+400(%rip),%rdi
    mov $16,%ecx
    repe cmpsb
    jne failed
    movabs $0x123456789abcdef0,%rax
    cmp %rax,%rbx
    jne failed
    cmp %rax,(%rsp)
    jne failed
    cmp %rax,%fs:0
    jne failed
    movabs $0x1020304050607080,%rax
    cmp %rax,%r12
    jne failed
    cmp %rax,%fs:8
    jne failed
    movabs $0x8877665544332211,%rax
    cmp %rax,%r13
    jne failed
    movabs $0x1122334455667788,%rax
    cmp %rax,%r14
    jne failed
    movabs $0x7162534435261708,%rax
    cmp %rax,%r15
    jne failed
    cmp $200000,%ebp
    /* EBP is the bounded rendezvous counter, not an invariant test value. */
    query destination
    test %rax,%rax
    jne failed
    query state_ok
    test %rax,%rax
    jne failed
    jmp exit_ok
observer:
    movabs $0xbadc0ffee0ddf00d,%rbx
    movq %rbx,%xmm0
    fninit
    fld1
    mov $200000,%ebp
7:  query pulse
    cmp $2,%rax
    je observer_done
    cmp $1,%rax
    jne 8f
    lea futex(%rip),%rdi
    mov $1,%esi
    mov $1,%edx
    mov $155,%eax
    syscall
    cmp $1,%rax
    jne failed
8:  movq %xmm0,%rax
    cmp %rbx,%rax
    jne failed
    yield
    dec %ebp
    jnz 7b
    jmp failed
observer_done:
    movq %xmm0,%rax
    cmp %rbx,%rax
    jne failed
exit_ok:
    xor %edi,%edi
    mov $20,%eax
    syscall
    ud2
failed:
    query fail
    ud2
.section .rodata
.macro msg name,text
\name: .ascii "T3R \text"
\name\()_end:
.endm
msg source,source
msg destination,destination
msg pulse,pulse
msg finish,finish
msg state_ok,state-ok
msg fail,fail
cw: .word 0x027f
mxcsr: .long 0x3f80
fp: .double 12345.125
.balign 16
vector: .quad 0x1122334455667788,0x8877665544332211
vector2: .quad 0x1020304050607080,0x0123456789abcdef
.section .data
.balign 32
tls: .quad 0x123456789abcdef0,0x1020304050607080,0,0
     .quad 0xbadc0ffee0ddf00d,0,0,0
.section .bss
.balign 16
before: .skip 512
after: .skip 512
tids: .skip 16
futex: .skip 4
.balign 16
stacks: .skip 2*16384
.section .note.GNU-stack,"",@progbits
