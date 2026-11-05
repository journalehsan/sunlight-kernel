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
    mov %rdi,%r12
    query case
    mov %rax,%r13
    xor %r15d,%r15d
    xor %r14d,%r14d
    test %r13,%r13
    jnz not_global
    mov $32,%r14d
    cmp $2,%r12d
    jb fill
    mov $15,%r14d
    cmp $2,%r12d
    je fill
    inc %r14d
    jmp fill
not_global:
    cmp $1,%r13d
    jne filled
    test %r12,%r12
    jnz filled
    mov $31,%r14d
fill:
    cmp %r14,%r15
    jae filled
    call spawn
    test %rax,%rax
    js failed
    inc %r15
    jmp fill
filled:
    test %r13,%r13
    jnz not_global_actor
    cmp $2,%r12d
    jb ready_owner
    query actor
    jmp ready_owner
not_global_actor:
    cmp $2,%r13d
    jne ready_owner
    test %r12,%r12
    jnz ready_owner
    query actor
ready_owner:
    query ready
    call wait_go
    test %r13,%r13
    jnz immediate_owner
    cmp $2,%r12d
    jb drain
    jmp contender
immediate_owner:
    cmp $2,%r13d
    jne drain
    test %r12,%r12
    jnz drain
    jmp contender
contender:
    mov $32,%r15d
    call spawn
    test %rax,%rax
    js rejected
    query accepted
    jmp after_contend
rejected:
    cmp $-2,%rax
    jne failed
    query rejected_msg
after_contend:
    cmp $1,%r13d
    je exit_ok
drain:
    mov $200000,%ebp
1:  query finish
    test %rax,%rax
    jnz exit_ok
    yield
    dec %ebp
    jnz 1b
    jmp failed
wait_go:
    mov $200000,%ebp
1:  query go
    test %rax,%rax
    jnz 2f
    yield
    dec %ebp
    jnz 1b
    jmp failed
2:  ret
spawn:
    mov %r15,%rsi
    shl $13,%rsi
    lea stacks+8160(%rip),%rax
    add %rax,%rsi
    lea worker(%rip),%rax
    mov %rax,(%rsi)
    mov %r15,8(%rsi)
    mov %rax,%rdi
    lea tls(%rip),%rdx
    mov $22,%eax
    syscall
    ret
worker:
    mov %rsi,%r15
    query case
    mov %rax,%r13
    cmp $2,%r13d
    je immediate_exit
    cmp $1,%r13d
    jne ordinary
    cmp $29,%r15d
    jb ordinary
    cmp $31,%r15d
    jae ordinary
    query actor
    call wait_go
    /* Distinct static stacks for the two competing children. */
    add $3,%r15
    call spawn
    test %rax,%rax
    js worker_reject
    query accepted
    jmp ordinary
worker_reject:
    cmp $-2,%rax
    jne failed
    query rejected_msg
    jmp ordinary
ordinary:
    mov $200000,%ebp
1:  query release
    test %rax,%rax
    jnz exit_ok
    yield
    dec %ebp
    jnz 1b
immediate_exit:
    query child_exit
    jmp exit_ok
failed:
    query fail
    ud2
exit_ok:
    xor %edi,%edi
    mov $20,%eax
    syscall
    ud2
.section .rodata
.macro msg name,text
\name: .ascii "T2R \text"
\name\()_end:
.endm
msg child_exit,child-exit
msg case,case
msg actor,actor
msg ready,ready
msg go,go
msg accepted,accepted
msg rejected_msg,rejected
msg release,release
msg finish,finish
msg fail,fail
.section .bss
.balign 16
tls: .skip 32
stacks: .skip 35*8192
.section .note.GNU-stack,"",@progbits
