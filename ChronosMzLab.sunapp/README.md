# Chronos MZ Process Lab

Rebuild the guest assets on a development host with NASM:

```sh
nasm -f bin Program/CHILD.ASM -o Program/CHILD.COM
nasm -f bin Program/MZLAB.ASM -o Program/MZLAB.EXE
```

`MZLAB.EXE` has a valid MZ header, a non-zero relocation count, initialized
stack metadata, reads `MESSAGE.TXT`, exercises DOS allocation/resize, calls
`INT 21h/AH=4Bh` for `CHILD.COM`, receives return code 42, and writes
`C:\MZLAB.LOG` to the bundle-private persistent C: overlay.
