bits 16
org 0x100

start:
    mov dx, title
    mov ah, 0x09
    int 0x21

    mov dx, readme_path
    xor al, al
    mov ah, 0x3d
    int 0x21
    jc exit
    mov bx, ax
    mov dx, buffer
    mov cx, 192
    mov ah, 0x3f
    int 0x21
    mov cx, ax
    mov bx, 1
    mov dx, buffer
    mov ah, 0x40
    int 0x21

    mov dx, runs_dir
    mov ah, 0x39
    int 0x21
    mov dx, runs_path
    xor cx, cx
    mov ah, 0x3c
    int 0x21
    jc menu
    mov bx, ax
    mov dx, run_message
    mov cx, run_message_end-run_message
    mov ah, 0x40
    int 0x21
    mov ah, 0x3e
    int 0x21

menu:
    mov dx, menu_text
    mov ah, 0x09
    int 0x21
wait_key:
    xor ah, ah
    int 0x16
    cmp al, 27
    je exit
    cmp al, 'L'
    je list
    cmp al, 'l'
    je list
    cmp al, 'W'
    je write_doc
    cmp al, 'w'
    je write_doc
    jmp wait_key

list:
    mov dx, documents_pattern
    xor cx, cx
    mov ah, 0x4e
    int 0x21
    jc menu
    mov dx, found_prefix
    mov ah, 0x09
    int 0x21
    mov dx, 0x009e
    mov ah, 0x09
    int 0x21
    jmp menu

write_doc:
    mov dx, document_path
    xor cx, cx
    mov ah, 0x3c
    int 0x21
    jc menu
    mov bx, ax
    mov dx, document_text
    mov cx, document_text_end-document_text
    mov ah, 0x40
    int 0x21
    mov ah, 0x3e
    int 0x21
    mov dx, saved_text
    mov ah, 0x09
    int 0x21
    jmp menu

exit:
    mov ax, 0x4c00
    int 0x21

title db 'Chronos File Lab', 13, 10, '$'
readme_path db 'C:\README.TXT', 0
runs_dir db 'C:\STATE', 0
runs_path db 'C:\STATE\RUNS.DAT', 0
documents_pattern db 'D:\*.*', 0
document_path db 'D:\CHRONOS.TXT', 0
run_message db '1', 13, 10
run_message_end:
document_text db 'Written by the real Chronos DOS guest.', 13, 10
document_text_end:
menu_text db 'L list D:  W write D:\CHRONOS.TXT  Esc exit', 13, 10, '$'
found_prefix db 'Found: $'
saved_text db 'Saved D:\CHRONOS.TXT', 13, 10, '$'
buffer times 256 db 0
