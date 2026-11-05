/* Raw syscall test of the AF_UNIX SOCK_STREAM socketpair receive path. */
.section .text
.global _start
_start:
    mov $53, %eax
    mov $1, %edi                  /* AF_UNIX */
    mov $0x801, %esi              /* SOCK_STREAM | SOCK_NONBLOCK */
    xor %edx, %edx
    lea pair(%rip), %r10
    syscall
    test %rax, %rax
    jne fail
    mov $45, %eax                 /* empty nonblocking receive */
    mov pair(%rip), %edi
    lea output(%rip), %rsi
    mov $4, %edx
    xor %r10d, %r10d
    xor %r8d, %r8d
    xor %r9d, %r9d
    syscall
    cmp $-11, %rax
    jne fail
    mov $32, %eax                 /* duplicated endpoint shares status */
    mov pair(%rip), %edi
    syscall
    test %rax, %rax
    js fail
    mov %eax, duplicate(%rip)
    mov $72, %eax
    mov duplicate(%rip), %edi
    mov $3, %esi
    syscall
    test $0x800, %eax
    jz fail
    mov $44, %eax                 /* sendto(peer, payload, 4, 0, NULL, 0) */
    mov pair+4(%rip), %edi
    lea payload(%rip), %rsi
    mov $4, %edx
    xor %r10d, %r10d
    xor %r8d, %r8d
    xor %r9d, %r9d
    syscall
    cmp $4, %rax
    jne fail
    mov $45, %eax
    mov duplicate(%rip), %edi
    lea output(%rip), %rsi
    mov $4, %edx
    xor %r10d, %r10d
    xor %r8d, %r8d
    xor %r9d, %r9d
    syscall
    cmp $4, %rax
    jne fail
    cmpl $0x5a595859, output(%rip) /* YXYZ */
    jne fail
    mov $45, %eax                 /* bad user buffer -> EFAULT */
    mov pair(%rip), %edi
    movabs $0xffff800000000000, %rsi
    mov $4, %edx
    xor %r10d, %r10d
    xor %r8d, %r8d
    xor %r9d, %r9d
    syscall
    cmp $-14, %rax
    jne fail
    mov $45, %eax                 /* valid pipe/TTY is not a socket */
    xor %edi, %edi
    lea output(%rip), %rsi
    mov $4, %edx
    syscall
    cmp $-88, %rax
    jne fail
    mov $45, %eax                 /* invalid descriptor */
    mov $255, %edi
    syscall
    cmp $-9, %rax
    jne fail
    mov $3, %eax                  /* close peer -> EOF */
    mov pair+4(%rip), %edi
    syscall
    test %rax, %rax
    jne fail
    mov $45, %eax
    mov duplicate(%rip), %edi
    lea output(%rip), %rsi
    mov $4, %edx
    xor %r10d, %r10d
    xor %r8d, %r8d
    xor %r9d, %r9d
    syscall
    test %rax, %rax
    jne fail
    /* Tokio registers eventfd with EPOLLET. Readiness is delivered once,
       then rearmed when a read drains the counter and a write fills it. */
    mov $290, %eax                /* eventfd2(0, EFD_NONBLOCK) */
    xor %edi, %edi
    mov $0x800, %esi
    syscall
    test %rax, %rax
    js fail
    mov %eax, event_fd(%rip)
    mov $291, %eax                /* epoll_create1(0) */
    xor %edi, %edi
    syscall
    test %rax, %rax
    js fail
    mov %eax, epoll_fd(%rip)
    mov $233, %eax                /* epoll_ctl(ADD, eventfd, EPOLLIN|EPOLLET) */
    mov epoll_fd(%rip), %edi
    mov $1, %esi
    mov event_fd(%rip), %edx
    lea edge_interest(%rip), %r10
    syscall
    test %rax, %rax
    jne fail
    mov $1, %eax                  /* write counter = 1 */
    mov event_fd(%rip), %edi
    lea one(%rip), %rsi
    mov $8, %edx
    syscall
    cmp $8, %rax
    jne fail
    mov $232, %eax                /* first edge */
    mov epoll_fd(%rip), %edi
    lea edge_ready(%rip), %rsi
    mov $1, %edx
    xor %r10d, %r10d
    syscall
    cmp $1, %rax
    jne fail
    mov $232, %eax                /* level remains high: no repeated edge */
    mov epoll_fd(%rip), %edi
    lea edge_ready(%rip), %rsi
    mov $1, %edx
    xor %r10d, %r10d
    syscall
    test %rax, %rax
    jne fail
    xor %eax, %eax                /* read drains counter */
    mov event_fd(%rip), %edi
    lea counter(%rip), %rsi
    mov $8, %edx
    syscall
    cmp $8, %rax
    jne fail
    cmpq $1, counter(%rip)
    jne fail
    mov $1, %eax                  /* new transition */
    mov event_fd(%rip), %edi
    lea one(%rip), %rsi
    mov $8, %edx
    syscall
    cmp $8, %rax
    jne fail
    mov $232, %eax
    mov epoll_fd(%rip), %edi
    lea edge_ready(%rip), %rsi
    mov $1, %edx
    xor %r10d, %r10d
    syscall
    cmp $1, %rax
    jne fail
    mov $1, %eax
    mov $1, %edi
    lea pass(%rip), %rsi
    mov $IO_SOCKET_PASS_LEN, %edx
    syscall
    mov $60, %eax
    xor %edi, %edi
    syscall
fail:
    mov $1, %eax
    mov $1, %edi
    lea failure(%rip), %rsi
    mov $IO_SOCKET_FAIL_LEN, %edx
    syscall
    mov $60, %eax
    mov $1, %edi
    syscall
.section .rodata
pass: .ascii "IO_SOCKET PASS\n"
.set IO_SOCKET_PASS_LEN, . - pass
failure: .ascii "IO_SOCKET FAIL\n"
.set IO_SOCKET_FAIL_LEN, . - failure
payload: .ascii "YXYZ"
.section .data
pair: .space 8
duplicate: .long 0
output: .space 4
event_fd: .long 0
epoll_fd: .long 0
one: .quad 1
counter: .quad 0
edge_interest: .long 0x80000001
               .quad 0x1234
edge_ready: .space 12
