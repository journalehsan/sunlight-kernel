/* Linux x86-64 userspace proof of shared pipe open-description status. */
.section .text
.global _start
_start:
    mov $293, %eax                /* pipe2(0) */
    lea pair(%rip), %rdi
    xor %esi, %esi
    syscall
    test %rax, %rax
    jne fail
    mov $32, %eax                 /* dup(read end) */
    mov pair(%rip), %edi
    syscall
    test %rax, %rax
    js fail
    mov %eax, duplicate(%rip)

    mov $16, %eax                 /* FIONBIO on original */
    mov pair(%rip), %edi
    mov $0x5421, %esi
    lea one(%rip), %rdx
    syscall
    test %rax, %rax
    jne fail
    mov $72, %eax                 /* F_GETFL on duplicate */
    mov duplicate(%rip), %edi
    mov $3, %esi
    syscall
    test $0x800, %eax
    jz fail
    mov $0, %eax                  /* empty pipe, live writer -> EAGAIN */
    mov duplicate(%rip), %edi
    lea received(%rip), %rsi
    mov $1, %edx
    syscall
    cmp $-11, %rax
    jne fail
    mov $1, %eax                  /* write/read byte */
    mov pair+4(%rip), %edi
    lea payload(%rip), %rsi
    mov $1, %edx
    syscall
    cmp $1, %rax
    jne fail
    mov $0, %eax
    mov duplicate(%rip), %edi
    lea received(%rip), %rsi
    mov $1, %edx
    syscall
    cmp $1, %rax
    jne fail
    cmpb $'Q', received(%rip)
    jne fail

    mov $72, %eax                 /* F_SETFL clears on duplicate */
    mov duplicate(%rip), %edi
    mov $4, %esi
    xor %edx, %edx
    syscall
    test %rax, %rax
    jne fail
    mov $72, %eax
    mov pair(%rip), %edi
    mov $3, %esi
    syscall
    test $0x800, %eax
    jnz fail
    mov $16, %eax                 /* FIONBIO sets again on duplicate */
    mov duplicate(%rip), %edi
    mov $0x5421, %esi
    lea one(%rip), %rdx
    syscall
    test %rax, %rax
    jne fail
    mov $16, %eax                 /* and clears on original */
    mov pair(%rip), %edi
    lea zero(%rip), %rdx
    syscall
    test %rax, %rax
    jne fail
    mov $72, %eax
    mov duplicate(%rip), %edi
    mov $3, %esi
    syscall
    test $0x800, %eax
    jnz fail

    mov $3, %eax                  /* close writer -> EOF */
    mov pair+4(%rip), %edi
    syscall
    test %rax, %rax
    jne fail
    mov $0, %eax
    mov duplicate(%rip), %edi
    lea received(%rip), %rsi
    mov $1, %edx
    syscall
    test %rax, %rax
    jne fail

    mov $293, %eax               /* pipe2(O_NONBLOCK) uses same status */
    lea second(%rip), %rdi
    mov $0x800, %esi
    syscall
    test %rax, %rax
    jne fail
    mov $72, %eax
    mov second(%rip), %edi
    mov $3, %esi
    syscall
    test $0x800, %eax
    jz fail
    mov $1, %eax                  /* fill pipe then EAGAIN on full write */
    mov second+4(%rip), %edi
    lea fill(%rip), %rsi
    mov $4096, %edx
    syscall
    cmp $4096, %rax
    jne fail
    mov $1, %eax
    mov second+4(%rip), %edi
    lea payload(%rip), %rsi
    mov $1, %edx
    syscall
    cmp $-11, %rax
    jne fail

    mov $1, %eax
    mov $1, %edi
    lea pass(%rip), %rsi
    mov $IO_PIPE_PASS_LEN, %edx
    syscall
    mov $60, %eax
    xor %edi, %edi
    syscall
fail:
    mov $1, %eax
    mov $1, %edi
    lea failure(%rip), %rsi
    mov $IO_PIPE_FAIL_LEN, %edx
    syscall
    mov $60, %eax
    mov $1, %edi
    syscall
.section .rodata
pass: .ascii "IO_PIPE PASS\n"
.set IO_PIPE_PASS_LEN, . - pass
failure: .ascii "IO_PIPE FAIL\n"
.set IO_PIPE_FAIL_LEN, . - failure
payload: .byte 'Q'
.section .data
one: .long 1
zero: .long 0
pair: .space 8
second: .space 8
duplicate: .long 0
received: .byte 0
fill: .fill 4096, 1, 0x41
