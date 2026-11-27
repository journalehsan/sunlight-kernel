.intel_syntax noprefix
.global _start
.section .text
_start:
    test rsp, 15
    jne fail
    cmp qword ptr [rsp], 2
    jne fail
    cmp qword ptr [rsp+24], 0
    jne fail
    mov rsi, [rsp+8]
    cmp byte ptr [rsi], '/'
    jne fail
    mov rsi, [rsp+16]
    movzx r14d, byte ptr [rsi]
    cmp byte ptr [rsi+1], 0
    jne fail
    lea rsi, [rsp+32]
env_next:
    lodsq
    test rax, rax
    jne env_next
    xor r13d, r13d
aux_next:
    lodsq
    test rax, rax
    je aux_end
    mov rdx, [rsi]
    add rsi, 8
    cmp eax, 25
    jne aux_next
    mov r13, rdx
    jmp aux_next
aux_end:
    movabs r15, 0x00007ffffffff000
    cmp r13, rsp
    jb fail
    lea rax, [r15-16]
    cmp r13, rax
    ja fail
    mov rax, [r13]
    or rax, [r13+8]
    jz fail
    mov r12, rsp
    mov ax, cs
    and eax, 3
    cmp eax, 3
    jne fail
    cmp r14b, 'b'
    je pass
    cmp r14b, 's'
    je sparse
    cmp r14b, 'd'
    jne fail
    lea rsp, [r15-16384]
    mov ebx, 5
grow:
    sub rsp, 4096
    mov ecx, 512
    mov rdi, rsp
    xor eax, eax
    repe scasq
    jne fail
    mov [rsp], rbx
    inc ebx
    cmp ebx, 513
    jne grow
    mov ebx, 512
verify:
    cmp [rsp], rbx
    jne fail
    add rsp, 4096
    dec ebx
    cmp ebx, 4
    jne verify
    jmp pass
sparse:
    lea rsp, [r15-2097152]
    mov ecx, 512
    mov rdi, rsp
    xor eax, eax
    repe scasq
    jne fail
    mov qword ptr [rsp], 123
    cmp qword ptr [rsp], 123
    jne fail
    jmp pass
pass:
    mov rsp, r12
    mov eax, 99
    lea rdi, [rip+marker]
    mov esi, marker_end-marker
    syscall
    xor edi, edi
    jmp exit
fail:
    mov edi, 1
exit:
    mov eax, 20
    syscall
    ud2
.section .rodata
marker: .ascii "[STACK-2] Ring-3 checks passed\n"
marker_end:
.section .note.GNU-stack,"",@progbits
