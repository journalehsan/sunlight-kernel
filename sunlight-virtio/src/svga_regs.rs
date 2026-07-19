//! VMware SVGA II register, capability, FIFO, and command definitions.
//!
//! Source of truth: VMware `svga_reg.h` as published with the Linux `vmwgfx`
//! driver (GPL-2.0 OR MIT dual license). Only the subset required for a
//! linear framebuffer + `SVGA_CMD_UPDATE` path is included.

/// PCI vendor / device for SVGA II.
pub const SVGA_PCI_VENDOR: u16 = 0x15AD;
pub const SVGA_PCI_DEVICE: u16 = 0x0405;

/// I/O port offsets relative to BAR0 I/O base (byte offsets used by `outl`/`inl`).
///
/// Matches Linux `vmw_write` / `vmw_read`:
/// `outl(index, io_start + SVGA_INDEX_PORT); outl(value, io_start + SVGA_VALUE_PORT)`.
pub const SVGA_INDEX_PORT: u16 = 0x0;
pub const SVGA_VALUE_PORT: u16 = 0x1;
pub const SVGA_IRQSTATUS_PORT: u16 = 0x8;

pub const SVGA_MAGIC: u32 = 0x900000;
pub const fn svga_make_id(ver: u32) -> u32 {
    (SVGA_MAGIC << 8) | ver
}

pub const SVGA_ID_2: u32 = svga_make_id(2);
pub const SVGA_ID_1: u32 = svga_make_id(1);
pub const SVGA_ID_0: u32 = svga_make_id(0);
pub const SVGA_ID_INVALID: u32 = 0xFFFF_FFFF;

/// Device registers (`SVGA_REG_*`).
pub const SVGA_REG_ID: u32 = 0;
pub const SVGA_REG_ENABLE: u32 = 1;
pub const SVGA_REG_WIDTH: u32 = 2;
pub const SVGA_REG_HEIGHT: u32 = 3;
pub const SVGA_REG_MAX_WIDTH: u32 = 4;
pub const SVGA_REG_MAX_HEIGHT: u32 = 5;
pub const SVGA_REG_DEPTH: u32 = 6;
pub const SVGA_REG_BITS_PER_PIXEL: u32 = 7;
pub const SVGA_REG_BYTES_PER_LINE: u32 = 12;
pub const SVGA_REG_FB_START: u32 = 13;
pub const SVGA_REG_FB_OFFSET: u32 = 14;
pub const SVGA_REG_VRAM_SIZE: u32 = 15;
pub const SVGA_REG_FB_SIZE: u32 = 16;
pub const SVGA_REG_CAPABILITIES: u32 = 17;
pub const SVGA_REG_MEM_START: u32 = 18;
pub const SVGA_REG_MEM_SIZE: u32 = 19;
pub const SVGA_REG_CONFIG_DONE: u32 = 20;
pub const SVGA_REG_SYNC: u32 = 21;
pub const SVGA_REG_BUSY: u32 = 22;
pub const SVGA_REG_GUEST_ID: u32 = 23;
pub const SVGA_REG_MEM_REGS: u32 = 30;
pub const SVGA_REG_PITCHLOCK: u32 = 32;
pub const SVGA_REG_TRACES: u32 = 45;

/// `SVGA_REG_ENABLE` bits.
pub const SVGA_REG_ENABLE_DISABLE: u32 = 0;
pub const SVGA_REG_ENABLE_ENABLE: u32 = 1 << 0;
pub const SVGA_REG_ENABLE_HIDE: u32 = 1 << 1;

/// Capability bits (`SVGA_CAP_*`).
pub const SVGA_CAP_RECT_COPY: u32 = 0x0000_0002;
pub const SVGA_CAP_CURSOR: u32 = 0x0000_0020;
pub const SVGA_CAP_8BIT_EMULATION: u32 = 0x0000_0100;
pub const SVGA_CAP_ALPHA_CURSOR: u32 = 0x0000_0200;
pub const SVGA_CAP_3D: u32 = 0x0000_4000;
pub const SVGA_CAP_EXTENDED_FIFO: u32 = 0x0000_8000;
pub const SVGA_CAP_MULTIMON: u32 = 0x0001_0000;
pub const SVGA_CAP_PITCHLOCK: u32 = 0x0002_0000;
pub const SVGA_CAP_IRQMASK: u32 = 0x0004_0000;
pub const SVGA_CAP_DISPLAY_TOPOLOGY: u32 = 0x0008_0000;
pub const SVGA_CAP_GMR: u32 = 0x0010_0000;
pub const SVGA_CAP_TRACES: u32 = 0x0020_0000;
pub const SVGA_CAP_GMR2: u32 = 0x0040_0000;
pub const SVGA_CAP_SCREEN_OBJECT_2: u32 = 0x0080_0000;
pub const SVGA_CAP_COMMAND_BUFFERS: u32 = 0x0100_0000;
pub const SVGA_CAP_GBOBJECTS: u32 = 0x0800_0000;

/// FIFO register indices (word offsets into BAR2).
pub const SVGA_FIFO_MIN: u32 = 0;
pub const SVGA_FIFO_MAX: u32 = 1;
pub const SVGA_FIFO_NEXT_CMD: u32 = 2;
pub const SVGA_FIFO_STOP: u32 = 3;
pub const SVGA_FIFO_CAPABILITIES: u32 = 4;
pub const SVGA_FIFO_FLAGS: u32 = 5;
pub const SVGA_FIFO_FENCE: u32 = 6;
pub const SVGA_FIFO_BUSY: u32 = 17;

/// First mandatory extended-FIFO register count (bytes = count * 4).
/// From `svga_reg.h`: `SVGA_FIFO_EXTENDED_MANDATORY_REGS = SVGA_FIFO_3D_CAPS_LAST + 1`.
pub const SVGA_FIFO_3D_CAPS: u32 = 32;
pub const SVGA_FIFO_3D_CAPS_LAST: u32 = 32 + 255;
pub const SVGA_FIFO_EXTENDED_MANDATORY_REGS: u32 = SVGA_FIFO_3D_CAPS_LAST + 1;

/// FIFO commands.
pub const SVGA_CMD_UPDATE: u32 = 1;

/// Generic host bounce for `SVGA_REG_SYNC` (any non-zero reason works; Linux uses 1).
pub const SVGA_SYNC_GENERIC: u32 = 1;

/// Minimum command-area size we require after FIFO metadata (bytes).
pub const SVGA_MIN_COMMAND_BYTES: u32 = 4096;

/// Bounded busy-loop iterations for FIFO space / BUSY wait.
pub const SVGA_FIFO_WAIT_SPINS: u32 = 1_000_000;
