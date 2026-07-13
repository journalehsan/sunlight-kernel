; SUNPAINT.COM - Chronos Prompt 5B polling DOS mouse paint demonstration
; NASM, 8086 real mode. Chronos supplies only INT 33h state and presentation;
; every toolbar, brush stroke, erase, and clear below is a guest A000 write.

bits 16
org 0x100

CANVAS_Y       equ 16
CANVAS_OFFSET  equ CANVAS_Y * 320
CANVAS_BYTES   equ (200 - CANVAS_Y) * 320
BACKGROUND     equ 0

start:
    xor ax, ax
    int 0x33
    or ax, ax
    jnz mouse_available
    jmp no_mouse

mouse_available:

    mov ax, 0x0013
    int 0x10
    mov ax, 0xa000
    mov es, ax
    cld

    ; One INT 33h coordinate per visible Mode 13h pixel. The driver reset
    ; default is deliberately 0..639 horizontally for DOS compatibility.
    mov ax, 0x0007
    xor cx, cx
    mov dx, 319
    int 0x33
    mov ax, 0x0008
    xor cx, cx
    mov dx, 199
    int 0x33
    mov ax, 0x0004
    mov cx, 160
    mov dx, 100
    int 0x33

    call draw_initial_ui
    mov ax, 0x0001
    int 0x33

    mov word [last_x], 160
    mov word [last_y], 100
    mov word [last_buttons], 0

poll:
    mov ax, 0x0003
    int 0x33
    mov [cur_buttons], bx
    mov [cur_x], cx
    mov [cur_y], dx

    mov ah, 0x01
    int 0x16
    jnz keyboard

    mov ax, [cur_x]
    cmp ax, [last_x]
    jne mouse_changed
    mov ax, [cur_y]
    cmp ax, [last_y]
    jne mouse_changed
    mov ax, [cur_buttons]
    cmp ax, [last_buttons]
    jne mouse_changed

    ; Generic DOS idle hint: Chronos yields this execution slice and returns
    ; to its native event loop without changing guest registers.
    int 0x28
    jmp poll

keyboard:
    mov ah, 0x00
    int 0x16
    cmp al, 0x1b
    jne .not_escape
    jmp exit_ok
.not_escape:
    cmp al, 'C'
    je clear_key
    cmp al, 'c'
    je clear_key
    cmp al, '1'
    jb poll
    cmp al, '8'
    ja poll
    sub al, '1'
    xor ah, ah
    mov si, ax
    mov al, [swatch_colors + si]
    mov [brush_color], al
    call draw_indicator
    jmp poll

clear_key:
    call clear_canvas
    call draw_indicator
    jmp poll

mouse_changed:
    test word [cur_buttons], 1
    jz check_right
    cmp word [cur_y], CANVAS_Y
    jb select_swatch
    mov al, [brush_color]
    mov [draw_color], al
    test word [last_buttons], 1
    jz plot_current
    cmp word [last_y], CANVAS_Y
    jb plot_current
    call draw_line
    jmp save_mouse

select_swatch:
    mov ax, [cur_x]
    xor dx, dx
    mov bx, 40
    div bx
    cmp ax, 7
    jbe .valid
    mov ax, 7
.valid:
    mov si, ax
    mov al, [swatch_colors + si]
    mov [brush_color], al
    call draw_indicator
    jmp save_mouse

check_right:
    test word [cur_buttons], 2
    jz save_mouse
    cmp word [cur_y], CANVAS_Y
    jb save_mouse
    mov byte [draw_color], BACKGROUND
    test word [last_buttons], 2
    jz plot_current
    cmp word [last_y], CANVAS_Y
    jb plot_current
    call draw_line
    jmp save_mouse

plot_current:
    mov ax, [cur_x]
    mov [line_x], ax
    mov ax, [cur_y]
    mov [line_y], ax
    call plot_line_point

save_mouse:
    mov ax, [cur_x]
    mov [last_x], ax
    mov ax, [cur_y]
    mov [last_y], ax
    mov ax, [cur_buttons]
    mov [last_buttons], ax
    jmp poll

exit_ok:
    mov ax, 0x0002
    int 0x33
    mov ax, 0x0003
    int 0x10
    mov ax, 0x4c00
    int 0x21

no_mouse:
    mov ax, 0x0003
    int 0x10
    mov dx, mouse_error
    mov ah, 0x09
    int 0x21
    mov ax, 0x4c01
    int 0x21

draw_initial_ui:
    xor di, di
    xor al, al
    mov cx, 64000
    rep stosb

    xor bx, bx
    mov si, swatch_colors
    mov word [swatches_left], 8
.swatch:
    lodsb
    mov di, bx
    mov bp, 16
.row:
    mov cx, 40
    rep stosb
    add di, 280
    dec bp
    jnz .row
    add bx, 40
    dec word [swatches_left]
    jnz .swatch
    call draw_indicator
    ret

clear_canvas:
    mov di, CANVAS_OFFSET
    mov al, BACKGROUND
    mov cx, CANVAS_BYTES
    rep stosb
    ret

; Bottom-right 12x12 current-color indicator with a white top/left edge.
draw_indicator:
    mov di, 188 * 320 + 308
    mov al, [brush_color]
    mov bp, 12
.row:
    mov cx, 12
    rep stosb
    add di, 308
    dec bp
    jnz .row
    mov di, 188 * 320 + 308
    mov al, 15
    mov cx, 12
    rep stosb
    mov di, 188 * 320 + 308
    mov cx, 12
.side:
    mov [es:di], al
    add di, 320
    loop .side
    ret

; Guest-side Bresenham line from last_x,last_y to cur_x,cur_y.
draw_line:
    mov ax, [last_x]
    mov [line_x], ax
    mov ax, [last_y]
    mov [line_y], ax

    mov ax, [cur_x]
    sub ax, [last_x]
    jns .dx_positive
    neg ax
    mov word [step_x], -1
    jmp .dx_done
.dx_positive:
    mov word [step_x], 1
.dx_done:
    mov [delta_x], ax

    mov ax, [cur_y]
    sub ax, [last_y]
    jns .dy_positive
    neg ax
    mov word [step_y], -1
    jmp .dy_done
.dy_positive:
    mov word [step_y], 1
.dy_done:
    neg ax
    mov [delta_y], ax
    add ax, [delta_x]
    mov [line_error], ax

.loop:
    call plot_line_point
    mov ax, [line_x]
    cmp ax, [cur_x]
    jne .advance
    mov ax, [line_y]
    cmp ax, [cur_y]
    je .done
.advance:
    mov ax, [line_error]
    add ax, ax
    mov [twice_error], ax
    cmp ax, [delta_y]
    jl .skip_x
    mov ax, [delta_y]
    add [line_error], ax
    mov ax, [step_x]
    add [line_x], ax
.skip_x:
    mov ax, [twice_error]
    cmp ax, [delta_x]
    jg .loop
    mov ax, [delta_x]
    add [line_error], ax
    mov ax, [step_y]
    add [line_y], ax
    jmp .loop
.done:
    ret

plot_line_point:
    mov ax, [line_y]
    cmp ax, CANVAS_Y
    jb .done
    cmp ax, 199
    ja .done
    mov bx, 320
    mul bx
    add ax, [line_x]
    mov di, ax
    mov al, [draw_color]
    mov [es:di], al
.done:
    ret

swatch_colors: db 4, 6, 10, 11, 12, 13, 14, 15
brush_color: db 12
draw_color: db 12
swatches_left: dw 0
cur_x: dw 0
cur_y: dw 0
cur_buttons: dw 0
last_x: dw 0
last_y: dw 0
last_buttons: dw 0
line_x: dw 0
line_y: dw 0
delta_x: dw 0
delta_y: dw 0
step_x: dw 0
step_y: dw 0
line_error: dw 0
twice_error: dw 0
mouse_error: db 'SUNPAINT: mouse unavailable', 13, 10, '$'
