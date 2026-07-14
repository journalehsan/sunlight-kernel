; SUNMINE.EXE - Sunlight Mines (9x9, 10 mines) for Chronos
; Text-mode 03h / direct B8000. Full game logic in 8086 assembly.
; Build: nasm -f bin sunmine.asm -o SUNMINE.EXE

%macro push_all 0
    push ax; push bx; push cx; push dx
    push si; push di; push bp; push ds
%endmacro

%macro pop_all 0
    pop ds; pop bp; pop di; pop si
    pop dx; pop cx; pop bx; pop ax
%endmacro

; =============================================================================
;  MEMORY LAYOUT
;  board: 81 bytes. Per cell: bit7=mine bit6=revealed bit5=flagged bits3-0=nbrs
;  queue: 256-word pairs for flood fill (x then y)
; =============================================================================

bits 16
org 0

    db 'MZ'
    dw (the_end - $$) % 512
    dw ((the_end - $$) + 511) / 512
    dw 0, 2, 0x20, 0x20
    dw ((the_end - code_start) + 15) / 16
    dw 0x0200, 0
    dw entry - code_start
    dw 0, 0x001c, 0, 0, 0

code_start:
    dw 0                    ; relocation

entry:
    push cs
    pop ds

    mov ax, 0x0003
    int 0x10
    mov ah, 1
    mov cx, 0x2607
    int 0x10

    xor ax, ax
    int 0x33
    mov ax, 1
    int 0x33
    mov ax, 7
    xor cx, cx
    mov dx, 79
    int 0x33
    mov ax, 8
    xor cx, cx
    mov dx, 24
    int 0x33
    mov ax, 4
    mov cx, 40
    mov dx, 13
    int 0x33

    call init_game
    call redraw_all
    xor ax, ax
    mov [tick_cnt], ax
    mov [ts_last], ax
    dec word [ts_last]
    mov [prev_won], ax
    mov [prev_lost], ax

; ---------------------------------------------------------------------------
;  MAIN LOOP
; ---------------------------------------------------------------------------
main_loop:
    inc word [tick_cnt]
    cmp word [tick_cnt], 8
    jb .no_yield
    mov word [tick_cnt], 0
    int 0x28
.no_yield:

    mov ax, 3
    int 0x33
    mov [mouse_x], cx
    mov [mouse_y], dx
    mov [buttons], bx

    mov ax, [buttons]
    mov bx, [old_buttons]
    mov [old_buttons], ax

    xor bp, bp                  ; dirty flag

    test ax, 1
    jz .no_l
    test bx, 1
    jnz .no_l
    call do_left_click
.no_l:
    test ax, 2
    jz .no_r
    test bx, 2
    jnz .no_r
    call do_right_click
.no_r:

    mov ah, 1
    int 0x16
    jz .no_key
    mov ah, 0
    int 0x16
    cmp al, 27
    je near do_exit
    or al, 0x20
    cmp al, 'r'
    je do_restart
    cmp al, 'n'
    je do_restart
.no_key:

    cmp word [post_redraw], 0
    je .no_post
    call draw_board
    call draw_status
    call maybe_banner
    mov word [post_redraw], 0
.no_post:

    call timer_tick

    cmp word [running], 0
    je .no_timer
    mov ax, [elapsed]
    cmp ax, [ts_last]
    je .no_timer
    mov [ts_last], ax
    call draw_status
.no_timer:

    mov bx, [winner]
    cmp bx, [prev_won]
    je .chk_lost
    mov [prev_won], bx
    test bx, bx
    jz .chk_lost
    call draw_board
    call draw_status
    mov si, str_win
    mov byte [banner_row], 15
    call draw_centered
.chk_lost:
    mov bx, [loser]
    cmp bx, [prev_lost]
    je .loop_tail
    mov [prev_lost], bx
    test bx, bx
    jz .loop_tail
    call draw_status
    mov si, str_lose
    mov byte [banner_row], 15
    call draw_centered

.loop_tail:
    jmp main_loop

do_restart:
    call init_game
    call redraw_all
    mov word [ts_last], 0xFFFF
    mov word [prev_won], 0
    mov word [prev_lost], 0
    jmp main_loop

do_exit:
    mov ax, 2
    int 0x33
    mov ax, 0x0003
    int 0x10
    mov ax, 0x4c00
    int 0x21

; ---------------------------------------------------------------------------
;  RANDOM NUMBER GENERATOR  (16-bit LCG)
; ---------------------------------------------------------------------------
rand:
    push bx
    push dx
    mov ax, [rng_seed]
    mov bx, 25173
    mul bx
    add ax, 13849
    mov [rng_seed], ax
    pop dx
    pop bx
    ret

; ---------------------------------------------------------------------------
;  BOARD INDEX:  si = x (0..8), di = y (0..8)  ->  bx = y*9 + x
; ---------------------------------------------------------------------------
cell_index:
    push ax
    push dx
    mov al, byte [tmp_y]
    mov dl, 9
    mul dl
    mov dl, byte [tmp_x]
    xor dh, dh
    add ax, dx
    mov bx, ax
    pop dx
    pop ax
    ret
tmp_x dw 0
tmp_y dw 0

; ---------------------------------------------------------------------------
;  GAME LOGIC
; ---------------------------------------------------------------------------
init_game:
    push ax
    push cx
    push di
    mov byte [mines_placed], 0
    mov word [winner], 0
    mov word [loser], 0
    mov word [first_click], 1
    mov word [left_mines], 10
    mov word [elapsed], 0
    mov word [running], 0
    mov word [rng_seed], 0x4321
    mov di, board_data
    mov cx, 81
    xor al, al
    rep stosb
    pop di
    pop cx
    pop ax
    ret

; place mines, avoiding (safe_x, safe_y) and its 3x3 neighborhood
place_mines:
    push_all
    cmp byte [mines_placed], 1
    je .pm_exit
    mov byte [mines_placed], 1
    mov word [running], 1
    mov word [first_click], 0
    mov ax, [safe_x]
    mov [tmp_sx], ax
    mov ax, [safe_y]
    mov [tmp_sy], ax

    mov bp, 10                 ; mines to place
.pm_try:
    call rand
    xor dx, dx
    mov cx, 9
    div cx                     ; dx = x
    mov si, dx
    call rand
    xor dx, dx
    mov cx, 9
    div cx                     ; dx = y
    mov di, dx

    ; exclude first-click 3x3 zone
    mov ax, [tmp_sx]
    sub ax, si
    jns .ab1
    neg ax
.ab1:
    cmp ax, 2
    jbe .pm_try
    mov ax, [tmp_sy]
    sub ax, di
    jns .ab2
    neg ax
.ab2:
    cmp ax, 2
    jbe .pm_try

    mov [tmp_x], si
    mov [tmp_y], di
    call cell_index
    test byte [board_data + bx], 0x80
    jnz .pm_try
    or byte [board_data + bx], 0x80
    dec bp
    jnz .pm_try

    call compute_neighbors
.pm_exit:
    pop_all
    ret

tmp_sx dw 0
tmp_sy dw 0

compute_neighbors:
    push_all
    xor di, di
.cn_y:
    xor si, si
.cn_x:
    mov ax, di
    mov dl, 9
    mul dl
    add ax, si
    mov bx, ax
    test byte [board_data + bx], 0x80
    jnz .cn_skip

    xor bp, bp
    ; dir0: (si-1,di-1)
    cmp si,0;je .cd1; cmp di,0;je .cd1
    push bx; mov ax,di;dec ax;mov dl,9;mul dl;add ax,si;dec ax;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd1a;inc bp
.cd1a: pop bx
.cd1:
    ; dir1: (si,di-1)
    cmp di,0;je .cd2
    push bx; mov ax,di;dec ax;mov dl,9;mul dl;add ax,si;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd2a;inc bp
.cd2a: pop bx
.cd2:
    ; dir2: (si+1,di-1)
    cmp si,8;je .cd3; cmp di,0;je .cd3
    push bx; mov ax,di;dec ax;mov dl,9;mul dl;add ax,si;inc ax;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd3a;inc bp
.cd3a: pop bx
.cd3:
    ; dir3: (si-1,di)
    cmp si,0;je .cd4
    push bx; mov ax,di;mov dl,9;mul dl;add ax,si;dec ax;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd4a;inc bp
.cd4a: pop bx
.cd4:
    ; dir4: (si+1,di)
    cmp si,8;je .cd5
    push bx; mov ax,di;mov dl,9;mul dl;add ax,si;inc ax;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd5a;inc bp
.cd5a: pop bx
.cd5:
    ; dir5: (si-1,di+1)
    cmp si,0;je .cd6; cmp di,8;je .cd6
    push bx; mov ax,di;inc ax;mov dl,9;mul dl;add ax,si;dec ax;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd6a;inc bp
.cd6a: pop bx
.cd6:
    ; dir6: (si,di+1)
    cmp di,8;je .cd7
    push bx; mov ax,di;inc ax;mov dl,9;mul dl;add ax,si;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd7a;inc bp
.cd7a: pop bx
.cd7:
    ; dir7: (si+1,di+1)
    cmp si,8;je .cd_st; cmp di,8;je .cd_st
    push bx; mov ax,di;inc ax;mov dl,9;mul dl;add ax,si;inc ax;mov bx,ax
    test byte [board_data+bx],0x80;jz .cd8a;inc bp
.cd8a: pop bx
.cd_st:
    mov al, [board_data + bx]
    and al, 0xF0
    cmp bp, 9
    jbe .cn_st
    xor bp, bp
.cn_st:
    push bx
    mov bx, bp
    or al, bl
    pop bx
    mov [board_data + bx], al
    mov [board_data + bx], al

.cn_skip:
    inc si
    cmp si, 9
    jb .cn_x
    inc di
    cmp di, 9
    jb .cn_y
    pop_all
    ret

; reveal cell at (si, di)
reveal_cell:
    push_all
    mov [tmp_x], si
    mov [tmp_y], di
    call cell_index
    mov al, [board_data + bx]
    test al, 0x40
    jnz .rc_done
    test al, 0x20
    jnz .rc_done
    test al, 0x80
    jnz .rc_mine

    or byte [board_data + bx], 0x40
    mov al, [board_data + bx]
    and al, 0x0F
    cmp al, 0
    jne .rc_done
    call flood_fill
    jmp .rc_done

.rc_mine:
    or byte [board_data + bx], 0x40
    mov word [loser], 1
    mov word [running], 0
    mov word [post_redraw], 1
    ; reveal all mines
    mov cx, 81
    mov bx, board_data
.rm_loop:
    test byte [bx], 0x80
    jz .rm_skip
    or byte [bx], 0x40
.rm_skip:
    inc bx
    loop .rm_loop

.rc_done:
    pop_all
    ret

flood_fill:
    push_all
    mov word [q_head], 0
    mov word [q_tail], 0
    ; enqueue initial (si, di) - store as X at offset, Y at offset+2
    mov bx, [q_tail]
    shl bx, 2              ; *4
    add bx, flood_queue
    mov word [bx], si      ; x
    mov word [bx+2], di    ; y
    inc word [q_tail]

.ff_loop:
    mov ax, [q_head]
    cmp ax, [q_tail]
    jae .ff_done

    mov bx, [q_head]
    shl bx, 2              ; *4
    add bx, flood_queue
    mov si, word [bx]      ; x
    mov di, word [bx+2]    ; y
    inc word [q_head]

    mov [tmp_x], si
    mov [tmp_y], di
    call cell_index
    test byte [board_data + bx], 0x20
    jnz .ff_loop
    or byte [board_data + bx], 0x40
    mov al, [board_data + bx]
    and al, 0x0F
    cmp al, 0
    jne .ff_loop

    ; expand 8 directions
    xor bp, bp
.ff_dir:
    mov ax, si
    mov dx, di
    cmp bp, 0; jne .fd1; dec ax; dec dx; jmp .fd_chk
.fd1: cmp bp, 1; jne .fd2;          dec dx; jmp .fd_chk
.fd2: cmp bp, 2; jne .fd3; inc ax; dec dx; jmp .fd_chk
.fd3: cmp bp, 3; jne .fd4; dec ax;          jmp .fd_chk
.fd4: cmp bp, 4; jne .fd5; inc ax;          jmp .fd_chk
.fd5: cmp bp, 5; jne .fd6; dec ax; inc dx; jmp .fd_chk
.fd6: cmp bp, 6; jne .fd7;          inc dx; jmp .fd_chk
.fd7:             inc ax; inc dx
.fd_chk:
    cmp ax, 9; jae .fd_next
    cmp dx, 9; jae .fd_next
    cmp ax, 0; jb .fd_next
    cmp dx, 0; jb .fd_next

    push si; push di
    mov si, ax; mov di, dx
    mov [tmp_x], si
    mov [tmp_y], di
    call cell_index
    mov al, [board_data + bx]
    test al, 0x40; jnz .fd_pop
    test al, 0x20; jnz .fd_pop
    and al, 0x0F; cmp al, 0; jne .fd_pop

    cmp word [q_tail], 254; jae .fd_pop
    mov bx, [q_tail]
    shl bx, 2              ; *4
    add bx, flood_queue
    mov word [bx], si      ; x
    mov word [bx+2], di    ; y
    inc word [q_tail]

.fd_pop:
    pop di; pop si
.fd_next:
    inc bp; cmp bp, 8; jb .ff_dir
    jmp .ff_loop
.ff_done:
    pop_all
    ret

; toggle flag
flag_toggle:
    push_all
    mov [tmp_x], si
    mov [tmp_y], di
    call cell_index
    mov al, [board_data + bx]
    test al, 0x40
    jnz .ft_out
    test al, 0x20
    jnz .ft_unflag
    or byte [board_data + bx], 0x20
    dec word [left_mines]
    jmp .ft_out
.ft_unflag:
    and byte [board_data + bx], 0xDF
    inc word [left_mines]
.ft_out:
    pop_all
    ret

; check if player won
do_check_win:
    push bx
    push cx
    mov cx, 81
    mov bx, board_data
.cw_loop:
    mov al, [bx]
    test al, 0x80
    jnz .cw_next
    test al, 0x40
    jnz .cw_next
    pop cx
    pop bx
    ret
.cw_next:
    inc bx
    loop .cw_loop
    mov word [winner], 1
    mov word [running], 0
    pop cx
    pop bx
    ret

; ---------------------------------------------------------------------------
;  INPUT HANDLING
; ---------------------------------------------------------------------------
; convert mouse to cell coords in (si, di); carry=out_of_bounds
mouse2cell:
    push ax
    push bx
    push dx
    mov ax, [mouse_x]
    cmp ax, 31
    jb .m2c_out
    sub ax, 31
    xor dx, dx
    mov bx, 2
    div bx
    mov si, ax
    mov ax, [mouse_y]
    cmp ax, 3
    jb .m2c_out
    sub ax, 3
    mov di, ax
    cmp si, 9
    jae .m2c_out
    cmp di, 9
    jae .m2c_out
    clc
    pop dx
    pop bx
    pop ax
    ret
.m2c_out:
    stc
    pop dx
    pop bx
    pop ax
    ret

do_left_click:
    push si
    push di
    call mouse2cell
    jc .dlc_exit
    cmp word [first_click], 1
    jne .dlc_reveal
    mov [safe_x], si
    mov [safe_y], di
    call place_mines
.dlc_reveal:
    call reveal_cell
    call do_check_win
    mov word [post_redraw], 1
.dlc_exit:
    pop di
    pop si
    ret
safe_x dw 0
safe_y dw 0

do_right_click:
    push si
    push di
    call mouse2cell
    jc .drc_exit
    call flag_toggle
    call draw_one_cell_at_sidi
    call draw_status
.drc_exit:
    pop di
    pop si
    ret

; ---------------------------------------------------------------------------
;  DRAWING
; ---------------------------------------------------------------------------
; write one B8000 cell:  ch=row  cl=col  al=char  ah=attr
b8000_put:
    push es
    push bx
    push di
    push ax
    mov bx, 0xB800
    mov es, bx
    movzx di, ch
    mov al, 160
    mul di
    mov di, ax
    movzx ax, cl
    shl ax, 1
    add di, ax
    pop ax
    stosw
    pop di
    pop bx
    pop es
    ret

; number -> attribute color
num_color:
    cmp al, 1; je  .n1
    cmp al, 2; je  .n2
    cmp al, 3; je  .n3
    cmp al, 4; je  .n4
    cmp al, 5; je  .n5
    cmp al, 6; je  .n6
    cmp al, 7; je  .n7
    cmp al, 8; je  .n8
    mov al, 0x07; ret
.n1: mov al, 0x09; ret
.n2: mov al, 0x0A; ret
.n3: mov al, 0x0C; ret
.n4: mov al, 0x01; ret
.n5: mov al, 0x05; ret
.n6: mov al, 0x03; ret
.n7: mov al, 0x07; ret
.n8: mov al, 0x0E; ret

; draw one cell at (si, di) screen position
draw_one_cell_at_sidi:
    push_all
    movzx ax, di
    add al, 3                  ; BOARD_ROW
    mov ch, al
    movzx ax, si
    shl al, 1                  ; * CELL_W
    add al, 31                 ; BOARD_COL
    mov cl, al

    mov [tmp_x], si
    mov [tmp_y], di
    call cell_index
    mov bl, [board_data + bx]
    mov bh, bl

    test bl, 0x40
    jnz .v_revealed

    ; HIDDEN
    test bl, 0x20
    jnz .v_flagged

    cmp word [loser], 0
    je .v_plain_hidden
    test bl, 0x80
    jz .v_plain_hidden
    mov al, '*'
    mov ah, 0x40
    call b8000_put
    inc cl
    mov al, ' '
    mov ah, 0x40
    call b8000_put
    jmp .v_done

.v_plain_hidden:
    mov al, 0xDB
    mov ah, 0x70
    call b8000_put
    inc cl
    call b8000_put
    jmp .v_done

.v_flagged:
    cmp word [loser], 0
    je .v_flag_ok
    test bl, 0x80
    jnz .v_flag_ok
    mov al, 'X'
    mov ah, 0x4F
    call b8000_put
    inc cl
    mov al, 0xDB
    mov ah, 0x4F
    call b8000_put
    jmp .v_done
.v_flag_ok:
    mov al, 'F'
    mov ah, 0x4F
    call b8000_put
    inc cl
    mov al, 0xDB
    mov ah, 0x70
    call b8000_put
    jmp .v_done

.v_revealed:
    test bl, 0x80
    jnz .v_mine
    mov al, bl
    and al, 0x0F
    cmp al, 0
    jne .v_number
    mov al, ' '
    mov ah, 0x07
    call b8000_put
    inc cl
    call b8000_put
    jmp .v_done
.v_number:
    mov al, ' '
    mov ah, 0x07
    call b8000_put
    inc cl
    mov al, bl
    and al, 0x0F
    push ax
    call num_color
    mov ah, al
    pop ax
    add al, '0'
    call b8000_put
    jmp .v_done
.v_mine:
    mov al, '*'
    mov ah, 0x07
    call b8000_put
    inc cl
    mov al, ' '
    mov ah, 0x07
    call b8000_put
.v_done:
    pop_all
    ret

draw_board:
    push_all
    xor di, di
.db_y:
    xor si, si
.db_x:
    call draw_one_cell_at_sidi
    inc si
    cmp si, 9
    jb .db_x
    inc di
    cmp di, 9
    jb .db_y
    pop_all
    ret

; draw zero-terminated string at (bh=row, bl=col), ah=attr, si->string
draw_zstring:
    push es
    push di
    push ax          ; save attr (ah) + junk(al)
    mov ax, 0xB800
    mov es, ax
    movzx di, bh
    push ax
    mov al, 160
    mul di
    mov di, ax
    pop ax
    movzx dx, bl
    shl dx, 1
    add di, dx
    pop ax           ; restore attr: ah=attr, al=junk
    mov dl, ah       ; move attr to dl for stosb/xchg
.dz_loop:
    lodsb
    test al, al
    jz .dz_done
    stosb
    xchg al, dl
    stosb
    xchg al, dl
    jmp .dz_loop
.dz_done:
    pop di
    pop es
    ret

draw_title:
    mov si, title_str
    mov bh, 0
    mov bl, 29
    mov ah, 0x1E
    call draw_zstring
    ret

draw_hints:
    mov si, hint_str
    mov bh, 13
    mov bl, 8
    mov ah, 0x07
    call draw_zstring
    ret

; draw centered text at banner_row
draw_centered:
    push_all
    push si
    xor cx, cx
    push si
.dc_len:
    cmp byte [si], 0
    je .dc_len_done
    inc cx
    inc si
    jmp .dc_len
.dc_len_done:
    pop si
    mov bl, 80
    sub bl, cl
    shr bl, 1
    mov bh, [banner_row]
    call draw_zstring
    pop si
    pop_all
    ret

; 2-digit value in ax at row 1, col bl
write_2digits:
    push ax; push bx; push di; push es
    push ax
    mov ax, 0xB800; mov es, ax
    movzx di, bl; shl di, 1; add di, 160
    pop ax
    push bx
    mov bl, 10; div bl
    add al, '0'; mov ah, 0x07; stosw
    add ah, '0'; mov al, ah; stosw
    pop bx
    pop es; pop di; pop bx; pop ax; ret

; 3-digit value in ax at row 1, col bl
write_3digits:
    push ax; push bx; push di; push es
    mov dx, 0xB800; mov es, dx
    movzx di, bl; shl di, 1; add di, 160
    mov bl, 100; div bl
    add al, '0'; mov ah, 0x07; stosw
    mov al, ah; xor ah, ah
    mov bl, 10; div bl
    add al, '0'; mov ah, 0x07; stosw
    add ah, '0'; mov al, ah; stosw
    pop es; pop di; pop bx; pop ax; ret

draw_status:
    push_all
    ; clear row 1
    push es
    mov ax, 0xB800
    mov es, ax
    mov di, 160
    mov cx, 80
    mov ax, 0x0720
    rep stosw
    pop es

    ; "MINES"
    mov si, mines_label
    mov bh, 1; mov bl, 4; mov ah, 0x0F
    call draw_zstring

    mov ax, [left_mines]
    mov bl, 10
    call write_2digits

    ; "TIME"
    mov si, time_label
    mov bh, 1; mov bl, 26; mov ah, 0x0F
    call draw_zstring

    mov ax, [elapsed]
    cmp ax, 999; jbe .ds_tok
    mov ax, 999
.ds_tok:
    mov bl, 31
    call write_3digits

    ; "BEST"
    mov si, best_label
    mov bh, 1; mov bl, 50; mov ah, 0x0F
    call draw_zstring

    mov ax, [best_time]
    test ax, ax; jnz .ds_bval
    ; write "---"
    push es
    mov ax, 0xB800; mov es, ax
    mov di, 160 + 55*2
    mov ax, 0x072D; stosw; stosw; stosw
    pop es
    jmp .ds_done
.ds_bval:
    mov bl, 55
    call write_3digits
.ds_done:
    pop_all; ret

clear_screen:
    push es; push ax; push cx; push di
    mov ax, 0xB800; mov es, ax
    xor di, di
    mov cx, 80*25
    mov ax, 0x0720
    rep stosw
    pop di; pop cx; pop ax; pop es; ret

maybe_banner:
    cmp word [winner], 1
    jne .mb_cmp
    mov si, str_win
    mov byte [banner_row], 15
    call draw_centered
    ret
.mb_cmp:
    cmp word [loser], 1
    jne .mb_done
    mov si, str_lose
    mov byte [banner_row], 15
    call draw_centered
.mb_done:
    ret

redraw_all:
    call clear_screen
    call draw_title
    call draw_status
    call draw_board
    call draw_hints
    call maybe_banner
    ret

timer_tick:
    push ax; push cx; push dx
    mov ah, 0x2C; int 0x21
    cmp word [running], 0; je .tickx
    movzx ax, dh
    cmp ax, 999; jbe .tickok; mov ax, 999
.tickok:
    mov [elapsed], ax
.tickx:
    pop dx; pop cx; pop ax; ret

; ---------------------------------------------------------------------------
;  DATA & BSS
; ---------------------------------------------------------------------------
title_str:   db 'SUNLIGHT MINES', 0
hint_str:    db 'Left=Reveal  Right=Flag  R=Restart  Esc=Exit', 0
str_win:     db 'FIELD CLEARED', 0
str_lose:    db 'MINE TRIGGERED', 0
mines_label: db 'MINES', 0
time_label:  db 'TIME', 0
best_label:  db 'BEST', 0

banner_row:  db 15

rng_seed:    dw 0x4321
mouse_x:     dw 0
mouse_y:     dw 0
buttons:     dw 0
old_buttons: dw 0
mines_placed: db 0
winner:      dw 0
loser:       dw 0
first_click: dw 1
left_mines:  dw 10
elapsed:     dw 0
running:     dw 0
tick_cnt:    dw 0
post_redraw: dw 0
ts_last:     dw 0xFFFF
prev_won:    dw 0
prev_lost:   dw 0
best_time:   dw 0

q_head:      dw 0
q_tail:      dw 0
flood_queue: times 512 db 0
board_data:  times 81 db 0

the_end:
