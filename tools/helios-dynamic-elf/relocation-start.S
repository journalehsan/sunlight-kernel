.text
.globl _start
_start:
    cmpq $1, (%rsp)
    jne failed
    mov pointer(%rip), %rsi
    lea message(%rip), %rax
    cmp %rax, %rsi
    jne failed
    mov $1, %eax
    mov $1, %edi
    mov $(message_end-message), %edx
    syscall
    cmp $(message_end-message), %rax
    jne failed
    xor %edi, %edi
    jmp finish
failed:
    mov $1, %edi
finish:
    mov $60, %eax
    syscall
    ud2
.section .rodata
message:
    .ascii "[HELIOS-RELOC] application entry verified relocated pointer\n"
message_end:
.data
.balign 8
pointer:
    .quad message
.section .note.GNU-stack,"",@progbits
