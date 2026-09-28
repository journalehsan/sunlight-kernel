/* Linux x86-64 open/openat O_LARGEFILE pathname and descriptor probe. */
.section .text
.global _start
_start:
    mov $2, %eax                  /* open existing file */
    lea existing(%rip), %rdi
    mov $0x88000, %esi           /* O_RDONLY | O_LARGEFILE | O_CLOEXEC */
    xor %edx, %edx
    syscall
    test %rax, %rax
    js fail
    mov %eax, fd(%rip)

    mov $72, %eax                 /* F_GETFD returns FD_CLOEXEC */
    mov fd(%rip), %edi
    mov $1, %esi
    syscall
    cmp $1, %rax
    jne fail
    mov $72, %eax                 /* F_GETFL exposes O_LARGEFILE, not CLOEXEC */
    mov fd(%rip), %edi
    mov $3, %esi
    syscall
    test $0x8000, %eax
    jz fail
    test $0x80000, %eax
    jnz fail

    mov $0, %eax
    mov fd(%rip), %edi
    lea contents(%rip), %rsi
    mov $5, %edx
    syscall
    cmp $5, %rax
    jne fail
    lea contents(%rip), %rsi
    lea prefix(%rip), %rdi
    mov $5, %ecx
compare:
    movb (%rsi), %al
    cmpb (%rdi), %al
    jne fail
    inc %rsi
    inc %rdi
    loop compare
    mov $3, %eax
    mov fd(%rip), %edi
    syscall
    test %rax, %rax
    jne fail

    mov $257, %eax               /* openat absolute nonexistent pathname */
    mov $-100, %edi
    lea missing(%rip), %rsi
    mov $0x88000, %edx
    xor %r10d, %r10d
    syscall
    cmp $-2, %rax                /* ENOENT, not EINVAL */
    jne fail

    mov $2, %eax                 /* unsupported O_DIRECT is still rejected */
    lea existing(%rip), %rdi
    mov $0xc000, %esi            /* O_LARGEFILE | O_DIRECT */
    xor %edx, %edx
    syscall
    cmp $-22, %rax
    jne fail
    mov $2, %eax
    lea existing(%rip), %rdi
    mov $0x40000000, %esi        /* unknown bit */
    xor %edx, %edx
    syscall
    cmp $-22, %rax
    jne fail

    mov $1, %eax
    mov $1, %edi
    lea pass(%rip), %rsi
    mov $PASS_LEN, %edx
    syscall
    mov $60, %eax
    xor %edi, %edi
    syscall
fail:
    mov $1, %eax
    mov $1, %edi
    lea failure(%rip), %rsi
    mov $FAIL_LEN, %edx
    syscall
    mov $60, %eax
    mov $1, %edi
    syscall
.section .rodata
existing: .asciz "/etc/passwd"
missing: .asciz "/definitely/not/present"
prefix: .ascii "root:"
pass: .ascii "OPEN_LARGEFILE PASS\n"
.set PASS_LEN, . - pass
failure: .ascii "OPEN_LARGEFILE FAIL\n"
.set FAIL_LEN, . - failure
.section .bss
.align 8
fd: .skip 4
contents: .skip 8
