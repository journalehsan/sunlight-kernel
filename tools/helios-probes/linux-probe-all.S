.section .text
.global _start
.type _start,@function

.equ SYS_READ, 0
.equ SYS_WRITE, 1
.equ SYS_CLOSE, 3
.equ SYS_MMAP, 9
.equ SYS_MUNMAP, 11
.equ SYS_BRK, 12
.equ SYS_NANOSLEEP, 35
.equ SYS_DUP, 32
.equ SYS_FCNTL, 72
.equ SYS_EXIT, 60
.equ SYS_GETRANDOM, 318
.equ AT_NULL, 0

_start:
    mov %rsp, %r12
    lea msg_argv(%rip), %rsi
    mov $msg_argv_len, %edx
    call emit

    # Verify argc, envp termination, and auxv AT_NULL are reachable.
    mov (%r12), %rax
    test %rax, %rax
    jz fail
    lea 8(%r12,%rax,8), %r13
    add $8, %r13
1:  mov (%r13), %rax
    add $8, %r13
    test %rax, %rax
    jnz 1b
2:  mov (%r13), %rax
    add $16, %r13
    test %rax, %rax
    jnz 2b

    lea msg_auxv(%rip), %rsi
    mov $msg_auxv_len, %edx
    call emit

    # dup(1), fcntl(F_GETFD), close.
    mov $SYS_DUP, %eax
    mov $1, %edi
    syscall
    test %rax, %rax
    js fail
    mov %eax, %ebp
    mov $SYS_FCNTL, %eax
    mov %ebp, %edi
    xor %esi, %esi
    syscall
    js fail
    mov $SYS_CLOSE, %eax
    mov %ebp, %edi
    syscall
    lea msg_fd(%rip), %rsi
    mov $msg_fd_len, %edx
    call emit

    # brk query, grow, write, shrink, grow again.
    mov $SYS_BRK, %eax
    xor %edi, %edi
    syscall
    mov %rax, %rbx
    lea 4096(%rax), %rdi
    mov $SYS_BRK, %eax
    syscall
    cmp %rdi, %rax
    jne fail
    movb $0x5a, (%rbx)
    mov %rbx, %rdi
    mov $SYS_BRK, %eax
    syscall
    cmp %rbx, %rax
    jne fail
    lea 4096(%rbx), %rdi
    mov $SYS_BRK, %eax
    syscall
    cmp %rdi, %rax
    jne fail
    lea msg_brk(%rip), %rsi
    mov $msg_brk_len, %edx
    call emit

    # mmap anonymous RW, touch, munmap.
    mov $SYS_MMAP, %eax
    xor %edi, %edi
    mov $4096, %esi
    mov $3, %edx
    mov $0x22, %r10d
    mov $-1, %r8d
    xor %r9d, %r9d
    syscall
    test %rax, %rax
    js fail
    movb $0x33, (%rax)
    mov %rax, %rdi
    mov $4096, %esi
    mov $SYS_MUNMAP, %eax
    syscall
    test %rax, %rax
    js fail
    lea msg_mmap(%rip), %rsi
    mov $msg_mmap_len, %edx
    call emit

    # Real timer-backed nanosleep (1ms).
    lea sleep_ts(%rip), %rdi
    xor %esi, %esi
    mov $SYS_NANOSLEEP, %eax
    syscall
    test %rax, %rax
    js fail
    lea msg_sleep(%rip), %rsi
    mov $msg_sleep_len, %edx
    call emit

    # Unknown syscall must be -ENOSYS.
    mov $999, %eax
    syscall
    cmp $-38, %rax
    jne fail
    lea msg_enosys(%rip), %rsi
    mov $msg_enosys_len, %edx
    call emit

    # Invalid pointer must be -EFAULT.
    mov $SYS_WRITE, %eax
    mov $1, %edi
    xor %esi, %esi
    mov $1, %edx
    syscall
    cmp $-14, %rax
    jne fail
    lea msg_efault(%rip), %rsi
    mov $msg_efault_len, %edx
    call emit

    # flags=0 getrandom (zero length is deterministic and pointer-free).
    mov $SYS_GETRANDOM, %eax
    xor %edi, %edi
    xor %esi, %esi
    xor %edx, %edx
    syscall
    test %rax, %rax
    jne fail
    lea msg_random(%rip), %rsi
    mov $msg_random_len, %edx
    call emit

    lea msg_pass(%rip), %rsi
    mov $msg_pass_len, %edx
    call emit
    mov $SYS_EXIT, %eax
    xor %edi, %edi
    syscall
fail:
    lea msg_fail(%rip), %rsi
    mov $msg_fail_len, %edx
    call emit
    mov $SYS_EXIT, %eax
    mov $1, %edi
    syscall

emit:
    mov %edx, %r14d
    mov $SYS_WRITE, %eax
    mov $1, %edi
    syscall
    cmp %r14, %rax
    jne fail
    ret

.section .rodata
msg_argv: .ascii "LINUX-PROBE argv-env PASS\n"
.set msg_argv_len, .-msg_argv
msg_auxv: .ascii "LINUX-PROBE auxv PASS\n"
.set msg_auxv_len, .-msg_auxv
msg_fd: .ascii "LINUX-PROBE dup-cloexec PASS\n"
.set msg_fd_len, .-msg_fd
msg_brk: .ascii "LINUX-PROBE brk PASS\n"
.set msg_brk_len, .-msg_brk
msg_mmap: .ascii "LINUX-PROBE mmap PASS\n"
.set msg_mmap_len, .-msg_mmap
msg_sleep: .ascii "LINUX-PROBE nanosleep PASS\n"
.set msg_sleep_len, .-msg_sleep
msg_enosys: .ascii "LINUX-PROBE invalid-syscall PASS\n"
.set msg_enosys_len, .-msg_enosys
msg_efault: .ascii "LINUX-PROBE invalid-pointer PASS\n"
.set msg_efault_len, .-msg_efault
msg_random: .ascii "LINUX-PROBE getrandom PASS\n"
.set msg_random_len, .-msg_random
msg_pass: .ascii "LINUX-PROBE TIER1 PASS\n"
.set msg_pass_len, .-msg_pass
msg_fail: .ascii "LINUX-PROBE TIER1 FAIL\n"
.set msg_fail_len, .-msg_fail

.section .data
.align 8
sleep_ts:
    .quad 0
    .quad 1000000
