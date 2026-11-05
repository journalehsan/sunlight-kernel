/* Authenticated by the debug kernel's immutable ELF loader/VM registration.
 * Every userspace loop has a finite iteration bound; host has a guest deadline.
 * Deliberately uses small, distinct mapped stacks; never stresses the host. */
.global _start
.section .text
.macro query label, length
.ifdef LINUX
    movabs $0x8000000000000063, %rax
.else
    mov $99, %eax
.endif
    lea \label(%rip), %rdi
    mov $\length, %esi
    syscall
.endm
.macro yield
.ifdef LINUX
    mov $24, %eax
.else
    mov $21, %eax
.endif
    syscall
.endm
_start:
    mov %rdi, %r12                  /* immutable owner role */
    mov $200000, %ebp
wait_my_phase:
    query phase, 8
    cmp $3, %r12d
    je observer
    cmp $2, %r12d
    je wait_pressure
    cmp %r12, %rax
    je fault_test
    yield
    dec %ebp
    jnz wait_my_phase
    jmp failed
fault_test:
    /* Warm all bounded scheduler storage before exact heap comparisons. */
    xor %r15d, %r15d
    call spawn
    test %rax, %rax
    js failed
    call quiescent
    mov $1, %r14d
fault_loop:
    mov %r14d, %eax
    xor %edx, %edx
    mov $10, %ecx
    div %ecx
    add $48, %dl
    mov %dl, arm+9(%rip)
    xor %edx, %edx
    div %ecx
    add $48, %dl
    mov %dl, arm+8(%rip)
    add $48, %al
    mov %al, arm+7(%rip)
    query arm, 10
    movl $0x12345678, parent_tid(%rip)
    xor %r15d, %r15d
    call spawn
    mov %rax, %r13
    query check, 8
    test %rax, %rax
    jz faults_complete
.ifdef LINUX
    cmp $-12, %r13
    jne failed
    cmpl $0x12345678, parent_tid(%rip)
    jne failed
.else
    cmp $-2, %r13
    jne failed
.endif
    inc %r14d
    cmp $128, %r14d
    jbe fault_loop
    jmp failed
faults_complete:
    test %r13, %r13
    js failed
    call quiescent
    query fault_done, 13
    mov $200000, %ebp
wait_pressure:
    query phase, 8
    cmp $4, %eax
    je exit_ok
    cmp $2, %eax
    je pressure_start
    yield
    dec %ebp
    jnz wait_pressure
    jmp failed
pressure_start:
    movl $0, release(%rip)
    xor %r15d, %r15d
create_pressure:
    call spawn
    test %rax, %rax
    js pressure_limit
    inc %r15d
    cmp $32, %r15d
    jbe create_pressure
    jmp failed
pressure_limit:
    cmp $32, %r15d
    jne failed
.ifdef LINUX
    cmp $-11, %rax
.else
    cmp $-2, %rax
.endif
    jne failed
    query full, 7
    mov $200000, %ebp
wait_release:
    query phase, 8
    cmp $3, %eax
    je release_all
    yield
    dec %ebp
    jnz wait_release
    jmp failed
release_all:
    movl $1, release(%rip)
    call quiescent
    query drained, 10
    cmp $1, %eax
    jne failed
    mov $200000, %ebp
    jmp wait_pressure
observer:
    query phase, 8
    cmp $4, %eax
    je exit_ok
    query global_ready, 15
    test %rax, %rax
    jz observer_yield
    xor %r15d, %r15d
    call spawn
    cmp $-2, %rax
    jne failed
    query pressure, 11
observer_yield:
    yield
    dec %ebp
    jnz observer
    jmp failed
/* Never allocate userspace storage in the request loop. All stacks/TLS are
 * static mapped ELF storage, making raw syscall capacity deterministic. */
spawn:
    mov %r15, %rsi
    shl $13, %rsi
    lea stacks+8160(%rip), %rax
    add %rax, %rsi
.ifdef LINUX
    mov $56, %eax
    mov $0x7d0f00, %edi
    lea parent_tid(%rip), %rdx
    lea tids(%rip), %r10
    lea (%r10,%r15,4), %r10
    lea tls(%rip), %r8
    syscall
    test %rax, %rax
    jz worker
.else
    lea worker(%rip), %rax
    mov %rax, (%rsi)
    movq $0, 8(%rsi)
    mov %rax, %rdi
    lea tls(%rip), %rdx
    mov $22, %eax
    test $1, %r15d
    jz 1f
    mov $158, %eax
1:  syscall
.endif
    ret
quiescent:
    mov $200000, %ebx
1:  query quiet, 12
    cmp $1, %eax
    je 2f
    yield
    dec %ebx
    jnz 1b
    jmp failed
2:  ret
worker:
    /* Useful user computation on each admitted context, with SSE isolation. */
    movabs $0x123456789abcdef0, %rbx
    movq %rbx, %xmm0
    mov $200000, %r14d
1:  cmpl $0, release(%rip)
    jne exit_ok
    query pulse, 8
    yield
    movq %xmm0, %rax
    cmp %rbx, %rax
    jne failed
    dec %r14d
    jnz 1b
    jmp failed
exit_ok:
    xor %edi, %edi
.ifdef LINUX
    mov $60, %eax
.else
    mov $20, %eax
.endif
    syscall
    ud2
failed:
    query fail, 7
    ud2
.section .data
phase: .ascii "T2 phase"
arm: .ascii "T2 arm 001"
check: .ascii "T2 check"
fault_done: .ascii "T2 fault-done"
quiet: .ascii "T2 quiescent"
full: .ascii "T2 full"
global_ready: .ascii "T2 global-ready"
pressure: .ascii "T2 pressure"
drained: .ascii "T2 drained"
pulse: .ascii "T2 pulse"
fail: .ascii "T2 fail"
.balign 4
release: .long 1
parent_tid: .long 0
.section .bss
.balign 16
tls: .skip 32
tids: .skip 128
.balign 16
stacks: .skip 33*8192
.section .note.GNU-stack,"",@progbits
