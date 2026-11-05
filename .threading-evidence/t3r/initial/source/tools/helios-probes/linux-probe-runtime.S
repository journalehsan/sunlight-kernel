.section .text
.global _start
.type _start,@function

.equ SYS_READ, 0
.equ SYS_WRITE, 1
.equ SYS_OPEN, 2
.equ SYS_CLOSE, 3
.equ SYS_STAT, 4
.equ SYS_FSTAT, 5
.equ SYS_LSEEK, 8
.equ SYS_PREAD64, 17
.equ SYS_PWRITE64, 18
.equ SYS_ACCESS, 21
.equ SYS_DUP3, 292
.equ SYS_GETPID, 39
.equ SYS_EXIT, 60
.equ SYS_UNAME, 63
.equ SYS_MKDIR, 83
.equ SYS_UNLINK, 87
.equ SYS_READLINK, 89
.equ SYS_GETTIMEOFDAY, 96
.equ SYS_GETUID, 102
.equ SYS_GETGID, 104
.equ SYS_GETEUID, 107
.equ SYS_GETEGID, 108
.equ SYS_GETPPID, 110
.equ SYS_GETDENTS64, 217
.equ SYS_CLOCK_GETTIME, 228
.equ SYS_NANOSLEEP, 35
.equ SYS_OPENAT, 257
.equ SYS_MKDIRAT, 258
.equ SYS_NEWFSTATAT, 262
.equ SYS_FACCESSAT, 269
.equ SYS_FCNTL, 72
.equ O_RDONLY, 0
.equ O_RDWR, 2
.equ O_CREAT, 0x40
.equ O_TRUNC, 0x200
.equ O_DIRECTORY, 0x10000
.equ O_CLOEXEC, 0x80000
.equ AT_FDCWD, -100
.equ F_GETFD, 1
.equ FD_CLOEXEC, 1
.equ SEEK_SET, 0
.equ SEEK_CUR, 1

_start:
    # uname
    lea utsbuf(%rip), %rdi
    mov $SYS_UNAME, %eax
    syscall
    test %rax, %rax
    jnz fail
    cmpb $'S', utsbuf(%rip)
    jne fail
    mov $SYS_UNAME, %eax
    xor %edi, %edi
    syscall
    cmp $-14, %rax
    jne fail
    lea msg_uname(%rip), %rsi
    mov $msg_uname_len, %edx
    call emit

    # identity
    mov $SYS_GETUID, %eax
    syscall
    mov %rax, %r12
    mov $SYS_GETEUID, %eax
    syscall
    cmp %r12, %rax
    jne fail
    mov $SYS_GETGID, %eax
    syscall
    mov %rax, %r13
    mov $SYS_GETEGID, %eax
    syscall
    cmp %r13, %rax
    jne fail
    mov $SYS_GETPPID, %eax
    syscall
    test %rax, %rax
    js fail
    lea msg_ids(%rip), %rsi
    mov $msg_ids_len, %edx
    call emit

    # gettimeofday + clocks
    lea timeval(%rip), %rdi
    xor %esi, %esi
    mov $SYS_GETTIMEOFDAY, %eax
    syscall
    test %rax, %rax
    jnz fail
    mov $SYS_CLOCK_GETTIME, %eax
    xor %edi, %edi
    lea timespec(%rip), %rsi
    syscall
    test %rax, %rax
    jnz fail
    mov $SYS_CLOCK_GETTIME, %eax
    mov $1, %edi
    lea timespec(%rip), %rsi
    syscall
    test %rax, %rax
    jnz fail
    mov $SYS_CLOCK_GETTIME, %eax
    mov $99, %edi
    lea timespec(%rip), %rsi
    syscall
    cmp $-22, %rax
    jne fail
    lea zero_ts(%rip), %rdi
    xor %esi, %esi
    mov $SYS_NANOSLEEP, %eax
    syscall
    test %rax, %rax
    jnz fail
    lea msg_time(%rip), %rsi
    mov $msg_time_len, %edx
    call emit

    # mkdir / mkdirat
    lea dirpath(%rip), %rdi
    mov $0x1ed, %esi
    mov $SYS_MKDIR, %eax
    syscall
    test %rax, %rax
    jz 1f
    cmp $-17, %rax
    jne fail
1:
    lea dirpath(%rip), %rdi
    mov $0x1ed, %esi
    mov $SYS_MKDIR, %eax
    syscall
    cmp $-17, %rax
    jne fail
    lea msg_mkdir(%rip), %rsi
    mov $msg_mkdir_len, %edx
    call emit

    # getdents64
    lea dirpath(%rip), %rdi
    mov $O_RDONLY, %esi
    or $O_DIRECTORY, %esi
    xor %edx, %edx
    mov $SYS_OPEN, %eax
    syscall
    test %rax, %rax
    js fail
    mov %eax, %ebx
    lea dentbuf(%rip), %rsi
    mov $512, %edx
    mov %ebx, %edi
    mov $SYS_GETDENTS64, %eax
    syscall
    test %rax, %rax
    jle fail
    mov %eax, %r12d
    lea dentbuf(%rip), %r13
    xor %r14d, %r14d
2:
    cmp %r12d, %r14d
    jae fail
    lea 19(%r13), %rdi
    lea dotname(%rip), %rsi
    call cstr_eq
    jc found_dir
    movzwl 16(%r13), %eax
    test %eax, %eax
    jz fail
    add %rax, %r13
    add %eax, %r14d
    jmp 2b
found_dir:
    mov %ebx, %edi
    xor %esi, %esi
    mov $SEEK_SET, %edx
    mov $SYS_LSEEK, %eax
    syscall
    test %rax, %rax
    jnz fail
    mov %ebx, %edi
    lea tinybuf(%rip), %rsi
    mov $8, %edx
    mov $SYS_GETDENTS64, %eax
    syscall
    cmp $-22, %rax
    jne fail
    mov %ebx, %edi
    lea dentbuf(%rip), %rsi
    mov $512, %edx
    mov $SYS_GETDENTS64, %eax
    syscall
    test %rax, %rax
    jle fail
    mov %ebx, %edi
    lea dentbuf(%rip), %rsi
    mov $512, %edx
    mov $SYS_GETDENTS64, %eax
    syscall
    test %rax, %rax
    jnz fail
    mov %ebx, %edi
    mov $SYS_CLOSE, %eax
    syscall
    lea msg_dents(%rip), %rsi
    mov $msg_dents_len, %edx
    call emit

    # pread/pwrite
    lea filepath(%rip), %rdi
    mov $O_RDWR, %esi
    or $O_CREAT, %esi
    or $O_TRUNC, %esi
    mov $0x1a4, %edx
    mov $SYS_OPEN, %eax
    syscall
    test %rax, %rax
    js fail
    mov %eax, %ebx
    mov %ebx, %edi
    lea payload(%rip), %rsi
    mov $8, %edx
    mov $SYS_WRITE, %eax
    syscall
    cmp $8, %rax
    jne fail
    mov %ebx, %edi
    xor %esi, %esi
    mov $SEEK_SET, %edx
    mov $SYS_LSEEK, %eax
    syscall
    test %rax, %rax
    jnz fail
    mov %ebx, %edi
    lea preadbuf(%rip), %rsi
    mov $4, %edx
    xor %r10, %r10
    add $4, %r10
    mov $SYS_PREAD64, %eax
    syscall
    cmp $4, %rax
    jne fail
    mov %ebx, %edi
    xor %esi, %esi
    mov $SEEK_CUR, %edx
    mov $SYS_LSEEK, %eax
    syscall
    test %rax, %rax
    jnz fail
    mov %ebx, %edi
    lea pwritepay(%rip), %rsi
    mov $2, %edx
    xor %r10, %r10
    add $1, %r10
    mov $SYS_PWRITE64, %eax
    syscall
    cmp $2, %rax
    jne fail
    mov %ebx, %edi
    xor %esi, %esi
    mov $SEEK_CUR, %edx
    mov $SYS_LSEEK, %eax
    syscall
    test %rax, %rax
    jnz fail
    xor %edi, %edi
    lea preadbuf(%rip), %rsi
    mov $1, %edx
    xor %r10, %r10
    mov $SYS_PREAD64, %eax
    syscall
    cmp $-29, %rax
    jne fail
    mov %ebx, %edi
    mov $SYS_CLOSE, %eax
    syscall
    lea msg_pread(%rip), %rsi
    mov $msg_pread_len, %edx
    call emit

    # access
    lea filepath(%rip), %rdi
    xor %esi, %esi
    mov $SYS_ACCESS, %eax
    syscall
    test %rax, %rax
    jnz fail
    lea missing(%rip), %rdi
    xor %esi, %esi
    mov $SYS_ACCESS, %eax
    syscall
    cmp $-2, %rax
    jne fail
    mov $AT_FDCWD, %edi
    lea filepath(%rip), %rsi
    xor %edx, %edx
    mov $0x4, %r10d
    mov $SYS_FACCESSAT, %eax
    syscall
    cmp $-22, %rax
    jne fail
    lea msg_access(%rip), %rsi
    mov $msg_access_len, %edx
    call emit

    # open flags
    lea filepath(%rip), %rdi
    mov $O_RDONLY, %esi
    or $O_DIRECTORY, %esi
    xor %edx, %edx
    mov $SYS_OPEN, %eax
    syscall
    cmp $-20, %rax
    jne fail
    lea filepath(%rip), %rdi
    mov $0x40000, %esi
    xor %edx, %edx
    mov $SYS_OPEN, %eax
    syscall
    cmp $-22, %rax
    jne fail
    lea filepath(%rip), %rdi
    mov $O_RDONLY, %esi
    or $O_CLOEXEC, %esi
    xor %edx, %edx
    mov $SYS_OPEN, %eax
    syscall
    test %rax, %rax
    js fail
    mov %eax, %ebx
    mov %ebx, %edi
    mov $F_GETFD, %esi
    xor %edx, %edx
    mov $SYS_FCNTL, %eax
    syscall
    cmp $FD_CLOEXEC, %rax
    jne fail
    mov %ebx, %edi
    mov $SYS_CLOSE, %eax
    syscall
    lea msg_open(%rip), %rsi
    mov $msg_open_len, %edx
    call emit

    # dup3
    mov $1, %edi
    mov $1, %esi
    xor %edx, %edx
    mov $SYS_DUP3, %eax
    syscall
    cmp $-22, %rax
    jne fail
    mov $1, %edi
    mov $8, %esi
    mov $O_CLOEXEC, %edx
    mov $SYS_DUP3, %eax
    syscall
    test %rax, %rax
    js fail
    mov %eax, %ebx
    mov %ebx, %edi
    mov $F_GETFD, %esi
    xor %edx, %edx
    mov $SYS_FCNTL, %eax
    syscall
    cmp $FD_CLOEXEC, %rax
    jne fail
    mov %ebx, %edi
    mov $SYS_CLOSE, %eax
    syscall
    lea msg_dup3(%rip), %rsi
    mov $msg_dup3_len, %edx
    call emit

    # stat metadata
    mov $AT_FDCWD, %edi
    lea dirpath(%rip), %rsi
    lea statbuf(%rip), %rdx
    xor %r10, %r10
    mov $SYS_NEWFSTATAT, %eax
    syscall
    test %rax, %rax
    jnz fail
    movl statbuf+24(%rip), %eax
    and $0xf000, %eax
    cmp $0x4000, %eax
    jne fail
    lea msg_stat(%rip), %rsi
    mov $msg_stat_len, %edx
    call emit

    # readlink: existing non-link is EINVAL; missing is ENOENT
    lea filepath(%rip), %rdi
    lea tinybuf(%rip), %rsi
    mov $8, %edx
    mov $SYS_READLINK, %eax
    syscall
    cmp $-22, %rax
    jne fail
    lea missing(%rip), %rdi
    lea tinybuf(%rip), %rsi
    mov $8, %edx
    mov $SYS_READLINK, %eax
    syscall
    cmp $-2, %rax
    jne fail
    lea msg_readlink(%rip), %rsi
    mov $msg_readlink_len, %edx
    call emit

    # dirfd: open /tmp as directory and openat the probe dir
    lea tmppath(%rip), %rdi
    mov $O_RDONLY, %esi
    or $O_DIRECTORY, %esi
    xor %edx, %edx
    mov $SYS_OPEN, %eax
    syscall
    test %rax, %rax
    js fail
    mov %eax, %ebx
    mov %ebx, %edi
    lea relname(%rip), %rsi
    mov $O_RDONLY, %edx
    or $O_DIRECTORY, %edx
    xor %r10, %r10
    mov $SYS_OPENAT, %eax
    syscall
    test %rax, %rax
    js fail
    mov %eax, %r12d
    mov %r12d, %edi
    mov $SYS_CLOSE, %eax
    syscall
    mov $99, %edi
    lea relname(%rip), %rsi
    xor %edx, %edx
    xor %r10, %r10
    mov $SYS_OPENAT, %eax
    syscall
    cmp $-9, %rax
    jne fail
    mov %ebx, %edi
    mov $SYS_CLOSE, %eax
    syscall
    lea msg_dirfd(%rip), %rsi
    mov $msg_dirfd_len, %edx
    call emit

    lea filepath(%rip), %rdi
    mov $SYS_UNLINK, %eax
    syscall

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
    mov %edx, %r15d
    mov $SYS_WRITE, %eax
    mov $1, %edi
    syscall
    cmp %r15, %rax
    jne fail
    ret

# CF=1 if equal
cstr_eq:
    push %rax
4:
    movb (%rdi), %al
    movb (%rsi), %ah
    cmp %al, %ah
    jne 5f
    test %al, %al
    jz 6f
    inc %rdi
    inc %rsi
    jmp 4b
5:
    pop %rax
    clc
    ret
6:
    pop %rax
    stc
    ret

.section .rodata
msg_uname: .ascii "LINUX-PROBE uname PASS\n"
.set msg_uname_len, .-msg_uname
msg_ids: .ascii "LINUX-PROBE ids PASS\n"
.set msg_ids_len, .-msg_ids
msg_time: .ascii "LINUX-PROBE gettimeofday PASS\n"
.set msg_time_len, .-msg_time
msg_mkdir: .ascii "LINUX-PROBE mkdir PASS\n"
.set msg_mkdir_len, .-msg_mkdir
msg_dents: .ascii "LINUX-PROBE getdents64 PASS\n"
.set msg_dents_len, .-msg_dents
msg_pread: .ascii "LINUX-PROBE pread-pwrite PASS\n"
.set msg_pread_len, .-msg_pread
msg_access: .ascii "LINUX-PROBE access PASS\n"
.set msg_access_len, .-msg_access
msg_open: .ascii "LINUX-PROBE open-flags PASS\n"
.set msg_open_len, .-msg_open
msg_dup3: .ascii "LINUX-PROBE dup3 PASS\n"
.set msg_dup3_len, .-msg_dup3
msg_stat: .ascii "LINUX-PROBE stat-metadata PASS\n"
.set msg_stat_len, .-msg_stat
msg_readlink: .ascii "LINUX-PROBE readlink PASS\n"
.set msg_readlink_len, .-msg_readlink
msg_dirfd: .ascii "LINUX-PROBE dirfd PASS\n"
.set msg_dirfd_len, .-msg_dirfd
msg_pass: .ascii "LINUX-PROBE RUNTIME PASS\n"
.set msg_pass_len, .-msg_pass
msg_fail: .ascii "LINUX-PROBE RUNTIME FAIL\n"
.set msg_fail_len, .-msg_fail
dirpath: .asciz "/tmp/helios-rt-dir"
tmppath: .asciz "/tmp"
filepath: .asciz "/tmp/helios-rt-file"
missing: .asciz "/tmp/helios-rt-missing"
relname: .asciz "helios-rt-dir"
dotname: .asciz "."
expect_name: .asciz "helios-rt-dir"
payload: .ascii "ABCDEFGH"
pwritepay: .ascii "XY"

.section .bss
.align 8
utsbuf: .space 390
timeval: .space 16
timespec: .space 16
zero_ts: .space 16
dentbuf: .space 512
tinybuf: .space 16
preadbuf: .space 8
statbuf: .space 144
