OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

SECTIONS
{
    . = 0x0000000000400000;

    .text : {
        *(.text .text.*)
    }

    . = ALIGN(4096);

    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(4096);

    /* Static TLS image consumed by sunlight-libc's thread bootstrap. */
    .tdata : {
        __tls_tdata_start = .;
        *(.tdata .tdata.*)
        __tls_tdata_end = .;
    }

    .tbss : {
        __tls_tbss_start = .;
        *(.tbss .tbss.*)
        __tls_tbss_end = .;
    }

    __tls_align = MAX(ALIGNOF(.tdata), ALIGNOF(.tbss));
    __tls_size = __tls_tbss_end - __tls_tdata_start;

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
