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
