.text
.globl sig01_spin
.type sig01_spin,@function
sig01_spin:
    mov %rsp, spin_rsp(%rip)
    movabs $0x123456789abcdef0, %rcx
    movabs $0xfedcba9876543210, %r11
    movq %rcx, %xmm15
    mov %rcx, -16(%rsp)
    xor %eax, %eax
    stc
    std
    movl $1, ready(%rip)
.globl spin_begin
spin_begin:
    pause
    mov caught(%rip), %eax
    jmp *spin_targets(,%rax,8)  # Poll without modifying any flags.
.globl spin_end
spin_end:
    pushfq
    pop %rax
    cld
    mov %rax, resumed_flags(%rip)
    mov %rcx, resumed_rcx(%rip)
    mov %r11, resumed_r11(%rip)
    movq %xmm15, resumed_xmm(%rip)
    mov %rsp, resumed_rsp(%rip)
    mov -16(%rsp), %rax         # independent red-zone canary is below pushfq
    mov %rax, resumed_redzone(%rip)
    ret
.size sig01_spin, .-sig01_spin
.section .rodata
.p2align 3
spin_targets:
    .quad spin_begin, spin_end
.section .note.GNU-stack,"",@progbits
