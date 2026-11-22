; FreeDOS reference only: request an ordinary DOS EXEC of DN.PRG.
; No interrupt hooks, launcher state, arguments, or injected guest keys.
bits 16
org 100h
    mov ax,cs
    mov ss,ax
    mov sp,stack_top
    mov ds,ax
    mov es,ax
    mov [params+4],ax
    mov [params+8],ax
    mov [params+12],ax
    mov bx,(stack_top-$$+100h+15)/16
    mov ah,4ah
    int 21h
    jc fail
    mov dx,program
    mov bx,params
    mov ax,4b00h
    int 21h
    jc fail
    mov ah,4dh
    int 21h
    mov ah,4ch
    int 21h
fail:
    mov ax,4cffh
    int 21h
program db 'DN.PRG',0
params dw 0,tail,0,fcb1,0,fcb2,0
tail db 0,13
fcb1 times 37 db 0
fcb2 times 37 db 0
times 512 db 0
stack_top equ $
