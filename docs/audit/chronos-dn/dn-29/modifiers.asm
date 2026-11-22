bits 16
org 100h

    mov ax, 3
    int 10h
    mov ax, 40h
    mov es, ax
    mov byte [es:17h], 0d0h
    mov ax, 0b800h
    mov ds, ax
    mov di, 22*160
    mov si, digits
    mov cx, 16
.legend:
    mov al, [cs:si]
    inc si
    call character
    loop .legend

sample:
    xor ax, ax
    mov al, [es:17h]
    mov [cs:values], ax
    mov ax, 02ffh
    mov bx, 0247h
    push bx
    popf
    int 16h
    mov [cs:values+2], ax
    pushf
    pop ax
    mov [cs:values+4], ax
    mov ax, 12ffh
    push bx
    popf
    int 16h
    mov [cs:values+6], ax
    pushf
    pop ax
    mov [cs:values+8], ax

    mov di, 23*160
    mov si, heading
.label:
    mov al, [cs:si]
    inc si
    test al, al
    jz .words
    call character
    jmp .label
.words:
    mov di, 24*160
    mov si, values
.word:
    mov dx, [cs:si]
    mov cx, 4
.digit:
    rol dx, 1
    rol dx, 1
    rol dx, 1
    rol dx, 1
    mov al, dl
    and al, 15
    add al, '0'
    cmp al, '9'
    jbe .store
    add al, 'A'-'9'-1
.store:
    call character
    loop .digit
    mov al, ' '
    call character
    add si, 2
    cmp si, values+10
    jb .word
    int 28h
    jmp sample

character:
    mov ah, 0fh
    mov [di], ax
    add di, 2
    ret

heading db 'BDA  AX02 FL02 AX12 FL12', 0
digits db '0123456789ABCDEF'
values times 5 dw 0
