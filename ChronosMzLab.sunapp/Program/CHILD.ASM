bits 16
org 0x100

start:
    mov dx, hello
    mov ah, 0x09
    int 0x21
    mov ax, 0x4c2a
    int 0x21

hello db 'Hello from the Chronos child process. /from-parent received.$'
