/* Native C ET_EXEC export ABI: 24-byte name, absolute function address. */
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

SECTIONS
{
    . = 0x0000000000400000;

    .text : {
        __sl_text_start = .;
        *(.text .text.*)
        __sl_text_end = .;
    }

    . = ALIGN(4096);

    .rodata : {
        *(.rodata .rodata.*)
        . = ALIGN(8);
        __sl_exports_start = .;
        /* malloc: GLOBAL DEFAULT function, retained by the address reference. */
        BYTE(109); BYTE(97); BYTE(108); BYTE(108); BYTE(111); BYTE(99); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0);
        QUAD(malloc);
        /* free: GLOBAL DEFAULT function, retained by the address reference. */
        BYTE(102); BYTE(114); BYTE(101); BYTE(101); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0);
        QUAD(free);
        /* calloc: GLOBAL DEFAULT function, retained by the address reference. */
        BYTE(99); BYTE(97); BYTE(108); BYTE(108); BYTE(111); BYTE(99); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0);
        QUAD(calloc);
        /* realloc: GLOBAL DEFAULT function, retained by the address reference. */
        BYTE(114); BYTE(101); BYTE(97); BYTE(108); BYTE(108); BYTE(111); BYTE(99); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0); BYTE(0);
        QUAD(realloc);
        __sl_exports_end = .;
    }

    . = ALIGN(4096);

    .data : {
        *(.data .data.*)
    }

    . = ALIGN(4096);

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.comment)
        *(.note)
        *(.note.*)
    }
}
