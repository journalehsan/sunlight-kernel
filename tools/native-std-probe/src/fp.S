.section .text
.global _start
_start:
    and rsp, -16
    call native_probe_main
    ud2

.global fp_sentinel
fp_sentinel:
    sub rsp, 72
    stmxcsr [rsp + 64]
    fnstcw [rsp + 68]
    mov r9, rdi
    mov [rsp], r9
    mov eax, r9d
    and eax, 3
    shl eax, 13
    or eax, 0x1f80
    mov [rsp + 8], eax
    ldmxcsr [rsp + 8]
    mov eax, r9d
    and eax, 3
    shl eax, 10
    or eax, 0x037f
    mov [rsp + 12], ax
    fldcw [rsp + 12]
    fild qword ptr [rsp]
    movq xmm0, r9
    punpcklqdq xmm0, xmm0
    mov rax, r9
    not rax
    movq xmm15, rax
    punpcklqdq xmm15, xmm15
    mov r8d, 64
1:
    /* Busy interval plus yield: test timer preemption and syscall dispatch. */
    mov edi, 100000
2:
    pause
    dec edi
    jnz 2b
    mov eax, 21
    syscall
    movaps [rsp + 32], xmm0
    movaps [rsp + 48], xmm15
    cmp [rsp + 32], r9
    jne 3f
    cmp [rsp + 40], r9
    jne 3f
    mov rax, r9
    not rax
    cmp [rsp + 48], rax
    jne 3f
    cmp [rsp + 56], rax
    jne 3f
    stmxcsr [rsp + 16]
    mov eax, [rsp + 16]
    cmp eax, [rsp + 8]
    jne 3f
    fnstcw [rsp + 20]
    mov ax, [rsp + 20]
    cmp ax, [rsp + 12]
    jne 3f
    fistp qword ptr [rsp + 24]
    fild qword ptr [rsp + 24]
    cmp [rsp + 24], r9
    jne 3f
    dec r8d
    jnz 1b
    fstp st(0)
    xor eax, eax
    ldmxcsr [rsp + 64]
    fldcw [rsp + 68]
    add rsp, 72
    ret
3:
    fninit
    mov eax, 1
    ldmxcsr [rsp + 64]
    fldcw [rsp + 68]
    add rsp, 72
    ret
