; Independent QEMU/SeaBIOS real-mode INT 16h probe; debug port E9.
bits 16
org 0x7c00
    cli
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov sp, 0x7c00
    sti
    mov al, 'R'
    out 0xe9, al
    mov al, 10
    out 0xe9, al
poll:
    mov ax, 0x115a
    mov bx, 0xabcd
    mov cx, 0x1234
    mov dx, 0x9876
    push word 0x0247
    popf
    int 0x16
    jz poll
    pushf
    call hex
    pop ax
    call hex
    mov ax, bx
    call hex
    mov ax, cx
    call hex
    mov ax, dx
    call hex
    mov ah, 1
    push word 0x0247
    popf
    int 0x16
    call hex
    mov ah, 0x10
    push word 0x0247
    popf
    int 0x16
    pushf
    call hex
    pop ax
    call hex
    mov ah, 2
    push word 0x0247
    popf
    int 0x16
    call hex
    mov al, 10
    out 0xe9, al
    jmp poll
hex:
    pusha
    mov bx, ax
    mov cx, 4
.next:
    rol bx, 4
    mov al, bl
    and al, 15
    add al, '0'
    cmp al, '9'
    jbe .emit
    add al, 'A' - '9' - 1
.emit:
    out 0xe9, al
    loop .next
    mov al, ' '
    out 0xe9, al
    popa
    ret
times 510-($-$$) db 0
dw 0xaa55
