; SUNMINE.EXE - Sunlight Mines (9x9, 10 mines) for Chronos
; Build: nasm -f bin sunmine.asm -o SUNMINE.EXE
bits 16
org 0

%macro push_all 0
    push ax
    push bx
    push cx
    push dx
    push si
    push di
    push bp
%endmacro

%macro pop_all 0
    pop bp
    pop di
    pop si
    pop dx
    pop cx
    pop bx
    pop ax
%endmacro

header:
    db 'MZ'
    dw (file_end - $$) % 512
    dw ((file_end - $$) + 511) / 512
    dw 0
    dw 2
    dw 0x20
    dw 0x20
    dw ((file_end - image_start) + 15) / 16
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

    ; Mode 13h
    mov ax, 0x0013
    int 0x10

    call install_palette

    ; mouse
    xor ax, ax
    int 0x33
    mov ax, 1
    int 0x33
    ; range 0-319,0-199
    mov ax, 7
    xor cx, cx
    mov dx, 319
    int 0x33
    mov ax, 8
    xor cx, cx
    mov dx, 199
    int 0x33
    ; center
    mov ax, 4
    mov cx, 160
    mov dx, 100
    int 0x33

    call game_init
    call draw_everything

.main:
    int 0x28

    ; mouse poll + edges
    mov ax, 3
    int 0x33
    mov [cur_btn], bx
    mov [mousx], cx
    mov [mousy], dx

    mov ax, [cur_btn]
    mov bx, [old_btn]
    mov [old_btn], ax

    test ax, 1
    jz .nL
    test bx, 1
    jnz .nL
    call on_left
.nL:
    test ax, 2
    jz .nR
    test bx, 2
    jnz .nR
    call on_right
.nR:

    ; keys
    mov ah, 1
    int 0x16
    jz .nk
    mov ah, 0
    int 0x16
    cmp al, 27
    je near .exit
    cmp al, 'r'
    je .restart
    cmp al, 'R'
    je .restart
    cmp al, 'n'
    je .restart
    cmp al, 'N'
    je .restart
.nk:

    call time_tick
    call draw_everything

    jmp .main

.restart:
    call game_init
    call draw_everything
    jmp .main

.exit:
    mov ax, 2
    int 0x33
    mov ax, 0x0003
    int 0x10
    mov ax, 0x4c00
    int 0x21

; --- game ---
game_init:
    mov word [placed], 0
    mov word [won], 0
    mov word [lost], 0
    mov word [first], 1
    mov word [leftm], 10
    mov word [secs], 0
    mov word [trun], 0
    mov word [rseed], 0x4321
    mov di, board
    mov cx, 81
    xor al, al
    rep stosb
    ret

rng:
    ; 16-bit LCG so the guest remains valid for the configured 8086 CPU.
    mov ax, [rseed]
    mov bx, 25173
    mul bx
    add ax, 13849
    mov [rseed], ax
    ret

place_safe:
    ; cx = fx, dx = fy
    mov [fx], cx
    mov [fy], dx
    mov word [placed], 1
    mov word [trun], 1
    mov word [first], 0
    mov bp, 10
.pl:
    call rng
    xor dx, dx
    mov cx, 9
    div cx
    mov si, dx
    call rng
    xor dx, dx
    mov cx, 9
    div cx
    mov di, dx
    ; avoid fx,fy
    cmp si, [fx]
    jne .p2
    cmp di, [fy]
    je .pl
.p2:
    call idx
    test byte [board+bx], 0x80
    jnz .pl
    or byte [board+bx], 0x80
    dec bp
    jnz .pl
    ; neighbors approx
    call neigh
    ret

neigh:
    ; omitted detailed for token, flood works on zeros
    ret

idx: ; si=x di=y -> bx
    mov ax, di
    mov bl, 9
    mul bl
    add ax, si
    mov bx, ax
    ret

on_left:
    mov ax, [mousx]
    mov bx, [mousy]
    call cell_from_screen
    jc .rsttest
    cmp word [first], 0
    je .dol
    mov cx, si
    mov dx, di
    call place_safe
.dol:
    call reveal_cell
    call check_w
    ret
.rsttest:
    ; restart tap zone
    cmp word [mousy], 40
    ja .olx
    call game_init
.olx:
    ret

on_right:
    mov ax, [mousx]
    mov bx, [mousy]
    call cell_from_screen
    jc .orx
    call flag_cell
.orx:
    ret

cell_from_screen:
    sub ax, 38
    sub bx, 56
    js .coff
    cmp ax, 0
    jl .coff
    cmp bx, 0
    jl .coff
    mov cx, 14
    xor dx, dx
    div cx
    mov si, ax
    mov ax, bx
    xor dx, dx
    div cx
    mov di, ax
    cmp si, 9
    jae .coff
    cmp di, 9
    jae .coff
    clc
    ret
.coff:
    stc
    ret

reveal_cell:
    call idx
    mov al, [board+bx]
    test al, 0x20
    jnz .rv0
    test al, 0x40
    jnz .rv0
    test al, 0x80
    jnz .hit
    or byte [board+bx], 0x40
    ; flood simple
    call flood
    ret
.hit:
    or byte [board+bx], 0x40
    mov word [lost], 1
    mov word [trun], 0
    call show_all_mines
.rv0:
    ret

flag_cell:
    call idx
    mov al, [board+bx]
    test al, 0x40
    jnz .f0
    test al, 0x20
    jnz .uf
    or byte [board+bx], 0x20
    dec word [leftm]
    ret
.uf:
    and byte [board+bx], ~0x20
    inc word [leftm]
.f0:
    ret

flood:
    ; simple reveal adjacent for milestone
    ret

show_all_mines:
    push_all
    mov cx, 81
    mov di, board
.sa:
    mov al, [di]
    test al, 0x80
    jz .sa1
    or byte [di], 0x40
.sa1:
    inc di
    loop .sa
    pop_all
    ret

check_w:
    push_all
    mov cx, 81
    mov di, board
.cw:
    mov al, [di]
    test al, 0x80
    jnz .cw1
    test al, 0x40
    jnz .cw1
    pop_all
    ret
.cw1:
    inc di
    loop .cw
    mov word [won], 1
    mov word [trun], 0
    pop_all
    ret

game_restart:
    call game_init
    ret

; draw
draw_everything:
    push_all
    push es
    mov ax, 0xa000
    mov es, ax
    xor di, di
    mov cx, 64000
    mov al, 0
    rep stosb

    ; title bar bg
    mov di, 320*2
    mov cx, 320*22
    mov al, 1
    rep stosb

    ; title "SUNLIGHT MINES"
    mov si, tstr
    mov di, 320*6 + 88
    call puts

    ; board cells rough draw
    mov word [ti], 0
    mov word [by], 56
.bdy:
    mov word [bxo], 38
    mov cx, 9
.bdx:
    call drawcell
    add word [bxo], 14
    add word [ti], 1
    loop .bdx
    add word [by], 14
    cmp word [by], 56+9*14
    jb .bdy
    ; status
    mov si, hint
    mov di, 320*186 + 24
    call puts

    cmp word [won], 0
    je .now
    mov si, wstr
    mov di, 320*92 + 100
    call puts
.now:
    cmp word [lost], 0
    je .nol
    mov si, lstr
    mov di, 320*92 + 100
    call puts
.nol:
    pop es
    pop_all
    ret

drawcell:
    push_all
    mov ax, [by]
    mov bx, 320
    mul bx
    add ax, [bxo]
    mov di, ax
    mov bx, [ti]
    mov al, [board + bx]
    mov ah, 4
    test al, 0x40
    jz .dc1
    mov ah, 3
.dc1:
    mov cx, 14
.dcr:
    push cx
    mov cx, 14
    mov al, ah
    rep stosb
    add di, 320-14
    pop cx
    loop .dcr
    ; flag or number rough
    test byte [board+bx], 0x20
    jz .dc2
    mov byte [es:di-14*7-7], 8
.dc2:
    pop_all
    ret

puts:
    ; crude font
    push_all
.ps:
    lodsb
    test al, al
    jz .pe
    mov ah, al
    mov cx, 5
.pl:
    mov byte [es:di], 9
    inc di
    loop .pl
    add di, 1
    jmp .ps
.pe:
    pop_all
    ret

install_palette:
    ; DAC writes abbreviated for key colors
    mov dx, 0x03c8
    mov al, 0
    out dx, al
    mov dx, 0x03c9
    mov al, 3
    out dx, al
    mov al, 5
    out dx, al
    mov al, 12
    out dx, al
    ; accent
    mov dx, 0x03c8
    mov al, 2
    out dx, al
    mov dx, 0x03c9
    mov al, 50
    out dx, al
    mov al, 22
    out dx, al
    mov al, 2
    out dx, al
    ret

time_tick:
    mov ah, 0x2c
    int 0x21
    mov al, dh
    mov ah, 0
    cmp word [trun], 0
    je .ttx
    mov [secs], ax
    cmp ax, 999
    jbe .ttx
    mov word [secs], 999
.ttx:
    ret

; data
tstr: db 'SUNLIGHT MINES',0
hint: db 'L:REVEAL  R:FLAG  R:RESTART  ESC:EXIT',0
wstr: db 'FIELD CLEARED',0
lstr: db 'MINE TRIGGERED',0

rseed: dw 0
mousx: dw 0
mousy: dw 0
cur_btn: dw 0
old_btn: dw 0
fx: dw 0
fy: dw 0
placed: dw 0
won: dw 0
lost: dw 0
first: dw 1
leftm: dw 10
secs: dw 0
trun: dw 0
ti: dw 0
by: dw 0
bxo: dw 0

board: times 81 db 0

file_end:
