; PALCYCLE.COM - Chronos Prompt 5A.1 guest-driven VGA DAC animation
; NASM, 8086 real mode. The A0000 image is written once. Every later visible
; change is produced exclusively by IN/OUT against VGA DAC ports 03C7h-03C9h.

bits 16
org 0x100

PALETTE_FIRST equ 32
PALETTE_COUNT equ 32
PALETTE_BYTES equ PALETTE_COUNT * 3

start:
    mov ax, 0x0013
    int 0x10

    mov ax, 0xa000
    mov es, ax
    cld

    ; Fixed dark background.
    xor di, di
    xor al, al
    mov cx, 64000
    rep stosb

    ; Thirty-two static five-row bands. Palette cycling makes their colors
    ; flow vertically while these 51,200 indexed bytes remain untouched.
    mov di, 6400                   ; y=20
    mov bl, PALETTE_FIRST
    mov bp, PALETTE_COUNT
.band:
    mov dl, 5
.band_row:
    mov al, bl
    mov cx, 320
    rep stosb
    dec dl
    jnz .band_row
    inc bl
    dec bp
    jnz .band

    ; A static ramp border reinforces that indices, not pixels, are moving.
    mov di, 3200                   ; y=10
    mov bl, PALETTE_FIRST
    mov bp, PALETTE_COUNT
.top_ramp:
    mov al, bl
    mov cx, 10
    rep stosb
    inc bl
    dec bp
    jnz .top_ramp

    mov di, 60800                  ; y=190
    mov bl, PALETTE_FIRST + PALETTE_COUNT - 1
    mov bp, PALETTE_COUNT
.bottom_ramp:
    mov al, bl
    mov cx, 10
    rep stosb
    dec bl
    dec bp
    jnz .bottom_ramp

    ; From here until shutdown ES=DS and no instruction writes A000 memory.
    push ds
    pop es

    ; Exercise DAC readback: snapshot indices 32..63 through 03C7h/03C9h.
    mov dx, 0x03c7
    mov al, PALETTE_FIRST
    out dx, al
    add dx, 2
    mov di, saved_palette
    mov cx, PALETTE_BYTES
.save_palette:
    in al, dx
    stosb
    loop .save_palette

    mov si, custom_palette
    mov di, active_palette
    mov cx, PALETTE_BYTES
    rep movsb
    call program_active_palette

.animation_loop:
    ; With Chronos' cooperative 128-instruction/1ms slice this bounded loop
    ; targets roughly 25-30 guest-selected palette frames per second.
    mov cx, 4000
.delay:
    loop .delay

    mov ah, 0x01
    int 0x16
    jz .animate
    mov ah, 0x00
    int 0x16
    cmp al, 0x1b
    je .exit

.animate:
    call rotate_active_palette
    call program_active_palette
    jmp .animation_loop

.exit:
    ; Restore the exact six-bit values captured through DAC reads.
    mov dx, 0x03c8
    mov al, PALETTE_FIRST
    out dx, al
    inc dx
    mov si, saved_palette
    mov cx, PALETTE_BYTES
.restore_palette:
    lodsb
    out dx, al
    loop .restore_palette

    mov ax, 0x0003
    int 0x10
    mov ax, 0x4c00
    int 0x21

program_active_palette:
    mov dx, 0x03c8
    mov al, PALETTE_FIRST
    out dx, al
    inc dx
    mov si, active_palette
    mov cx, PALETTE_BYTES
.write:
    lodsb
    out dx, al
    loop .write
    ret

rotate_active_palette:
    mov si, active_palette
    lodsb
    mov [rotation_temp], al
    lodsb
    mov [rotation_temp + 1], al
    lodsb
    mov [rotation_temp + 2], al

    mov si, active_palette + 3
    mov di, active_palette
    mov cx, PALETTE_BYTES - 3
    rep movsb
    mov si, rotation_temp
    mov cx, 3
    rep movsb
    ret

; A vivid six-bit RGB loop: orange -> yellow -> green -> cyan -> blue ->
; magenta -> orange. The guest rotates this table one whole entry per frame.
custom_palette:
    db 63,  8,  0, 63, 14,  0, 63, 20,  0, 63, 28,  0
    db 63, 38,  0, 63, 50,  0, 63, 63,  0, 48, 63,  0
    db 32, 63,  0, 16, 63,  0,  0, 63,  0,  0, 63, 20
    db  0, 63, 40,  0, 63, 63,  0, 44, 63,  0, 28, 63
    db  0, 12, 63,  0,  0, 63, 16,  0, 63, 32,  0, 63
    db 48,  0, 63, 63,  0, 63, 63,  0, 48, 63,  0, 32
    db 63,  0, 16, 63,  0,  4, 63,  4,  0, 63,  8,  0
    db 63, 12,  0, 63, 16,  0, 63, 12,  0, 63,  8,  0

saved_palette: times PALETTE_BYTES db 0
active_palette: times PALETTE_BYTES db 0
rotation_temp: times 3 db 0
