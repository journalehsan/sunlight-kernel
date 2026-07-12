bits 16
org 0x100

    mov ax, 0x0003
    int 0x10
    mov ax, 0xb800
    mov es, ax
    xor di, di
    mov si, title
    mov cx, title_end - title
draw_title:
    lodsb
    mov ah, 0x1f
    stosw
    loop draw_title

    mov dx, instructions
    mov ah, 0x09
    int 0x21

read_key:
    mov ah, 0x00
    int 0x16
    cmp al, 0x1b
    je exit
    cmp al, 13
    jne print
    mov ah, 0x0e
    int 0x10
    mov al, 10
    int 0x10
    jmp read_key

print:
    mov ah, 0x0e
    int 0x10
    jmp read_key

exit:
    mov ax, 0x4c00
    int 0x21

title db 'Chronos Interactive Console'
title_end:

instructions db 'Type inside the DOS guest.', 13, 10
             db 'Enter starts a new line.', 13, 10
             db 'Backspace deletes.', 13, 10
             db 'Esc exits.', 13, 10, '$'
