; Build with: nasm -f bin MZLAB.ASM -o MZLAB.EXE
; This is a real MZ image with a 32-byte header and one relocation entry.
bits 16
org 0

header:
    db 'MZ'
    dw file_end - $$
    dw 1
    dw 1
    dw 2
    dw 0x20
    dw 0xffff
    dw 0
    dw 0x0200
    dw 0
    dw start - image_start
    dw 0
    dw 0x001c
    dw 0
    dw 0, 0

image_start:
relocation_word dw 0
start:
    push cs
    pop ds
    push ds
    pop es

    mov dx, title
    mov ah, 0x09
    int 0x21

    mov dx, message_path
    mov ax, 0x3d00
    int 0x21
    jc skip_message
    mov bx, ax
    mov dx, buffer
    mov cx, 96
    mov ah, 0x3f
    int 0x21
    mov cx, ax
    mov bx, 1
    mov ah, 0x40
    int 0x21
skip_message:
    mov bx, 16
    mov ah, 0x48
    int 0x21
    jc finish
    mov [allocation], ax
    mov es, ax
    mov bx, 32
    mov ah, 0x4a
    int 0x21

    mov ax, cs
    mov [exec_block + 4], ax
    push cs
    pop es
    mov dx, child_path
    mov bx, exec_block
    mov ax, 0x4b00
    int 0x21
    jc free_block

    mov ah, 0x4d
    int 0x21
    cmp al, 42
    jne free_block
    mov dx, child_ok
    mov ah, 0x09
    int 0x21

free_block:
    mov es, [allocation]
    mov ah, 0x49
    int 0x21
finish:
    mov dx, log_path
    xor cx, cx
    mov ah, 0x3c
    int 0x21
    jc exit
    mov bx, ax
    mov dx, log_text
    mov cx, log_text_end - log_text
    mov ah, 0x40
    int 0x21
exit:
    mov ax, 0x4c00
    int 0x21

title db 'Chronos MZ Process Lab',13,10,'$'
message_path db 'C:\MESSAGE.TXT',0
child_path db 'C:\CHILD.COM',0
log_path db 'C:\MZLAB.LOG',0
child_ok db 'Child exit code: 42',13,10,'$'
log_text db 'Chronos MZ Process Lab completed child EXEC.',13,10
log_text_end:
allocation dw 0
exec_block:
    dw 0
    dw command_tail
    dw 0
    dw 0
    dw 0, 0, 0, 0
command_tail db 13,' /from-parent',13
buffer times 96 db 0
file_end:
