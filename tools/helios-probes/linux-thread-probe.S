/* Raw Linux x86-64 syscall probe: no libc, pthread, Tokio, or kernel markers. */
.section .text
.global _start

.macro print label, size
    mov $1, %eax
    mov $1, %edi
    lea \label(%rip), %rsi
    mov $\size, %edx
    syscall
.endm

_start:
    /* Both threads must retain their own FS base over scheduling. */
    mov $158, %eax
    mov $0x1002, %edi
    lea tls_main(%rip), %rsi
    syscall
    test %rax, %rax
    jne failed
    mov %fs:0, %rax
    mov %rax, main_tls_seen(%rip)

    mov $39, %eax
    syscall
    mov %eax, main_pid(%rip)
    mov $186, %eax
    syscall
    mov %eax, main_tid(%rip)

    /* Two independent descriptors: worker writes one and closes the other. */
    mov $290, %eax
    xor %edi, %edi
    mov $0x80800, %esi
    syscall
    test %rax, %rax
    js failed
    mov %eax, shared_fd(%rip)
    mov $290, %eax
    xor %edi, %edi
    mov $0x80800, %esi
    syscall
    test %rax, %rax
    js failed
    mov %eax, close_fd(%rip)

    /* Failed clones must not publish a TID or create a runnable child. */
    mov $56, %eax
    mov $0x10000, %edi              /* CLONE_THREAD without CLONE_SIGHAND */
    lea child_stack_top(%rip), %rsi
    lea parent_tid(%rip), %rdx
    lea child_tid(%rip), %r10
    lea tls_worker(%rip), %r8
    syscall
    cmp $-22, %rax
    jne failed
    mov $56, %eax
    mov $0x7d0f00, %edi
    lea child_stack_top(%rip), %rsi
    movabs $0xffff800000000000, %rdx /* invalid parent_tid */
    lea child_tid(%rip), %r10
    lea tls_worker(%rip), %r8
    syscall
    cmp $-14, %rax
    jne failed
    mov $56, %eax
    mov $0x7d0f00, %edi
    mov $0x70000000, %esi          /* unmapped child stack */
    lea parent_tid(%rip), %rdx
    lea child_tid(%rip), %r10
    lea tls_worker(%rip), %r8
    syscall
    cmp $-14, %rax
    jne failed
    mov $56, %eax
    mov $0x207d0f00, %edi         /* unsupported PID namespace */
    lea child_stack_top(%rip), %rsi
    lea parent_tid(%rip), %rdx
    lea child_tid(%rip), %r10
    lea tls_worker(%rip), %r8
    syscall
    cmp $-38, %rax
    jne failed

    mov $56, %eax
    mov $0x7d0f00, %edi
    lea child_stack_top(%rip), %rsi
    lea parent_tid(%rip), %rdx
    lea child_tid(%rip), %r10
    lea tls_worker(%rip), %r8
    syscall
    test %rax, %rax
    js failed
    jz worker
    mov %eax, clone_tid(%rip)

wait_for_clear:
    cmpl $0, child_tid(%rip)
    je complete
    mov $202, %eax
    lea child_tid(%rip), %rdi
    mov $128, %esi                 /* FUTEX_WAIT | FUTEX_PRIVATE_FLAG */
    mov $55, %edx
    xor %r10d, %r10d
    syscall
    jmp wait_for_clear

worker:
    mov $39, %eax
    syscall
    mov %eax, worker_pid(%rip)
    mov $186, %eax
    syscall
    mov %eax, worker_tid(%rip)
    mov %fs:0, %rax
    mov %rax, worker_tls_seen(%rip)
    /* Handler disposition is shared; the worker's blocked mask is not. */
    mov $13, %eax
    mov $10, %edi                  /* SIGUSR1 */
    lea new_action(%rip), %rsi
    xor %edx, %edx
    mov $8, %r10d
    syscall
    test %rax, %rax
    jne failed
    mov $14, %eax
    xor %edi, %edi                 /* SIG_BLOCK */
    lea blocked_usr1(%rip), %rsi
    xor %edx, %edx
    mov $8, %r10d
    syscall
    test %rax, %rax
    jne failed
    mov $1, %eax
    mov shared_fd(%rip), %edi
    lea event_value(%rip), %rsi
    mov $8, %edx
    syscall
    cmp $8, %rax
    jne failed
    mov $3, %eax
    mov close_fd(%rip), %edi
    syscall
    test %rax, %rax
    jne failed
    mov $80, %eax
    lea tmp_path(%rip), %rdi
    syscall
    test %rax, %rax
    jne failed
    movl $1, shared_word(%rip)
    mov $60, %eax
    xor %edi, %edi
    syscall
    ud2

complete:
    /* A second syscall and scheduler yield must preserve the main FS base. */
    mov $24, %eax
    syscall
    mov %fs:0, %rax
    cmpq tls_main(%rip), %rax
    jne failed
    mov main_tls_seen(%rip), %rax
    cmpq tls_main(%rip), %rax
    jne failed
    mov worker_tls_seen(%rip), %rax
    cmpq tls_worker(%rip), %rax
    jne failed
    mov parent_tid(%rip), %eax
    cmp clone_tid(%rip), %eax
    jne failed
    mov worker_tid(%rip), %eax
    test %eax, %eax
    je failed
    cmp main_tid(%rip), %eax
    je failed
    mov worker_pid(%rip), %eax
    cmp main_pid(%rip), %eax
    jne failed
    cmpl $1, shared_word(%rip)
    jne failed
    mov $79, %eax
    lea cwd_buf(%rip), %rdi
    mov $32, %esi
    syscall
    cmpl $0x706d742f, cwd_buf(%rip) /* /tmp set by the worker */
    jne failed
    cmpb $0, cwd_buf+4(%rip)
    jne failed
    mov $13, %eax
    mov $10, %edi
    xor %esi, %esi
    lea old_action(%rip), %rdx
    mov $8, %r10d
    syscall
    test %rax, %rax
    jne failed
    mov old_action(%rip), %rax
    lea signal_handler(%rip), %rsi
    cmp %rsi, %rax
    jne failed
    mov $14, %eax
    mov $2, %edi                  /* SIG_SETMASK: get old, leave unchanged */
    xor %esi, %esi
    lea old_mask(%rip), %rdx
    mov $8, %r10d
    syscall
    test %rax, %rax
    jne failed
    cmpq $0, old_mask(%rip)
    jne failed
    mov $0, %eax
    mov shared_fd(%rip), %edi
    lea event_read(%rip), %rsi
    mov $8, %edx
    syscall
    cmp $8, %rax
    jne failed
    cmpq $1, event_read(%rip)
    jne failed
    mov $1, %eax
    mov close_fd(%rip), %edi
    lea event_value(%rip), %rsi
    mov $8, %edx
    syscall
    cmp $-9, %rax                 /* EBADF: worker's close is group-wide */
    jne failed
    mov $3, %eax
    mov shared_fd(%rip), %edi
    syscall
    test %rax, %rax
    jne failed

    print msg_pass, msg_pass_len
    print msg_main, msg_main_len
    mov main_pid(%rip), %edi
    call print_hex
    print msg_tid, msg_tid_len
    mov main_tid(%rip), %edi
    call print_hex
    print msg_nl, 1
    print msg_worker, msg_worker_len
    mov worker_pid(%rip), %edi
    call print_hex
    print msg_worker_tid, msg_worker_tid_len
    mov worker_tid(%rip), %edi
    call print_hex
    print msg_nl, 1
    print msg_shared, msg_shared_len
    print msg_fd, msg_fd_len
    print msg_tls, msg_tls_len
    print msg_clear, msg_clear_len
    mov $60, %eax
    xor %edi, %edi
    syscall
    ud2

failed:
    print msg_fail, msg_fail_len
    mov $60, %eax
    mov $1, %edi
    syscall
    ud2

/* Print the actual 32-bit syscall PID/TID as eight hexadecimal digits. */
print_hex:
    lea hexbuf(%rip), %rsi
    mov $8, %ecx
1:
    rol $4, %edi
    mov %edi, %eax
    and $15, %eax
    cmp $10, %eax
    jb 2f
    add $('a' - 10), %eax
    jmp 3f
2:
    add $'0', %eax
3:
    mov %al, (%rsi)
    inc %rsi
    loop 1b
    mov $1, %eax
    mov $1, %edi
    lea hexbuf(%rip), %rsi
    mov $8, %edx
    syscall
    ret

signal_handler:
    ret

.section .rodata
msg_main:   .ascii "THREAD_PROBE main_pid=0x"
.set msg_main_len, . - msg_main
msg_worker: .ascii "THREAD_PROBE worker_pid=0x"
.set msg_worker_len, . - msg_worker
msg_tid:    .ascii " main_tid=0x"
.set msg_tid_len, . - msg_tid
msg_worker_tid: .ascii " worker_tid=0x"
.set msg_worker_tid_len, . - msg_worker_tid
msg_nl:     .ascii "\n"
msg_shared: .ascii "shared_memory=ok\n"
.set msg_shared_len, . - msg_shared
msg_fd:     .ascii "fd_table=ok eventfd=ok\n"
.set msg_fd_len, . - msg_fd
msg_tls:    .ascii "tls=ok\n"
.set msg_tls_len, . - msg_tls
msg_clear:  .ascii "child_tid_clear=ok\n"
.set msg_clear_len, . - msg_clear
msg_pass:   .ascii "THREAD_PROBE PASS\n"
.set msg_pass_len, . - msg_pass
msg_fail:   .ascii "THREAD_PROBE FAIL\n"
.set msg_fail_len, . - msg_fail

.section .data
.align 8
tls_main:   .quad 0x1122334455667788
tls_worker: .quad 0x8877665544332211
child_tid:  .long 55
event_value: .quad 1
tmp_path: .asciz "/tmp"
new_action: .quad signal_handler, 0, 0, 0
blocked_usr1: .quad 512

.section .bss
.align 16
child_stack: .skip 16384
child_stack_top:
hexbuf:     .skip 8
main_pid:   .skip 4
main_tid:   .skip 4
worker_pid: .skip 4
worker_tid: .skip 4
clone_tid:  .skip 4
parent_tid: .skip 4
shared_word: .skip 4
shared_fd: .skip 4
close_fd: .skip 4
event_read: .skip 8
cwd_buf: .skip 32
old_action: .skip 32
old_mask: .skip 8
main_tls_seen: .skip 8
worker_tls_seen: .skip 8
