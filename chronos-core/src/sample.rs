/// A valid DOS `.COM` program:
///
/// ```asm
/// mov dx, message
/// mov ah, 09h
/// int 21h
/// mov ax, 4c00h
/// int 21h
/// message db 'Hello from Chronos!$'
/// ```
pub const HELLO_CHRONOS_COM: &[u8] = &[
    0xba, 0x0c, 0x01, 0xb4, 0x09, 0xcd, 0x21, 0xb8, 0x00, 0x4c, 0xcd, 0x21, b'H', b'e', b'l', b'l',
    b'o', b' ', b'f', b'r', b'o', b'm', b' ', b'C', b'h', b'r', b'o', b'n', b'o', b's', b'!', b'$',
];

/// Interactive guest built from `chronos-core/guests/chronos-interactive.asm`.
///
/// It draws a colored title directly through `ES:DI`/`STOSW`, then reads keys
/// through BIOS `INT 16h`, echoing them through BIOS teletype until Escape.
pub const CHRONOS_INTERACTIVE_COM: &[u8] = &[
    0xb8, 0x03, 0x00, 0xcd, 0x10, // mov ax,0003 / int 10
    0xb8, 0x00, 0xb8, 0x8e, 0xc0, // mov ax,b800 / mov es,ax
    0x31, 0xff, // xor di,di
    0xbe, 0x4a, 0x01, 0xb9, 0x1b, 0x00, // mov si,title / mov cx,27
    0xac, 0xb4, 0x1f, 0xab, 0xe2, 0xfa, // lodsb / mov ah,1f / stosw / loop
    0xb7, 0x00, 0xb6, 0x02, 0xb2, 0x00, 0xb4, 0x02, 0xcd, 0x10, // cursor row 2
    0xba, 0x65, 0x01, 0xb4, 0x09, 0xcd, 0x21, // mov dx,message / AH=09
    0xb4, 0x00, 0xcd, 0x16, // wait: int 16
    0x3c, 0x1b, 0x74, 0x14, // cmp al,esc / je exit
    0x3c, 0x0d, 0x75, 0x0a, // cmp al,enter / jne printable
    0xb4, 0x0e, 0xcd, 0x10, 0xb0, 0x0a, 0xcd, 0x10, 0xeb, 0xea, // CR LF / wait
    0xb4, 0x0e, 0xcd, 0x10, 0xeb, 0xe4, // printable / wait
    0xb8, 0x00, 0x4c, 0xcd, 0x21, // exit
    b'C', b'h', b'r', b'o', b'n', b'o', b's', b' ', b'I', b'n', b't', b'e', b'r', b'a', b'c', b't',
    b'i', b'v', b'e', b' ', b'C', b'o', b'n', b's', b'o', b'l', b'e', b'T', b'y', b'p', b'e', b' ',
    b'i', b'n', b's', b'i', b'd', b'e', b' ', b't', b'h', b'e', b' ', b'D', b'O', b'S', b' ', b'g',
    b'u', b'e', b's', b't', b'.', b'\r', b'\n', b'E', b'n', b't', b'e', b'r', b' ', b's', b't',
    b'a', b'r', b't', b's', b' ', b'a', b' ', b'n', b'e', b'w', b' ', b'l', b'i', b'n', b'e', b'.',
    b'\r', b'\n', b'B', b'a', b'c', b'k', b's', b'p', b'a', b'c', b'e', b' ', b'd', b'e', b'l',
    b'e', b't', b'e', b's', b'.', b'\r', b'\n', b'E', b's', b'c', b' ', b'e', b'x', b'i', b't',
    b's', b'.', b'\r', b'\n', b'$',
];
