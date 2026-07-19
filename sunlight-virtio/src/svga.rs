//! Minimal VMware SVGA II 2D display driver (legacy framebuffer + UPDATE).
//!
//! Programming model validated against VMware `svga_reg.h` / Linux `vmwgfx`
//! and the SerenityOS VMWare SVGA II adapter. Scope is deliberately limited to:
//! PCI resource discovery, version negotiation, FIFO init, mode adoption, and
//! `SVGA_CMD_UPDATE` presentation.

use crate::pci::{
    inl, outl, PciIoBarInfo, PciMemoryBarInfo, VmwareSvgaPciInfo, VmwareSvgaProbeError,
};
use crate::svga_regs::*;

/// Lifecycle stages reported in serial diagnostics and Device Manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SvgaStage {
    NotProbed = 0,
    PciMatched = 1,
    ResourcesMapped = 2,
    VersionNegotiated = 3,
    CapabilitiesRead = 4,
    FifoInitialized = 5,
    DisplayUsable = 6,
    Active = 7,
}

impl SvgaStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotProbed => "not-probed",
            Self::PciMatched => "pci-matched",
            Self::ResourcesMapped => "resources-mapped",
            Self::VersionNegotiated => "version-negotiated",
            Self::CapabilitiesRead => "capabilities-read",
            Self::FifoInitialized => "fifo-initialized",
            Self::DisplayUsable => "display-usable",
            Self::Active => "active",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvgaError {
    Probe(VmwareSvgaProbeError),
    VersionUnsupported { readback: u32 },
    InvalidGeometry,
    InvalidPitch,
    FbExtentOverflow,
    FbOutsideBar,
    FifoTooSmall { mem_size: u32, min_bytes: u32 },
    FifoInvariant,
    FifoTimeout,
    ModeRejected,
    NotReady,
    RectInvalid,
}

impl SvgaError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Probe(_) => "probe-failed",
            Self::VersionUnsupported { .. } => "version-unsupported",
            Self::InvalidGeometry => "invalid-geometry",
            Self::InvalidPitch => "invalid-pitch",
            Self::FbExtentOverflow => "fb-extent-overflow",
            Self::FbOutsideBar => "fb-outside-bar",
            Self::FifoTooSmall { .. } => "fifo-too-small",
            Self::FifoInvariant => "fifo-invariant",
            Self::FifoTimeout => "fifo-timeout",
            Self::ModeRejected => "mode-rejected",
            Self::NotReady => "not-ready",
            Self::RectInvalid => "rect-invalid",
        }
    }

    pub const fn code(self) -> u64 {
        match self {
            Self::Probe(_) => 1,
            Self::VersionUnsupported { .. } => 2,
            Self::InvalidGeometry => 3,
            Self::InvalidPitch => 4,
            Self::FbExtentOverflow => 5,
            Self::FbOutsideBar => 6,
            Self::FifoTooSmall { .. } => 7,
            Self::FifoInvariant => 8,
            Self::FifoTimeout => 9,
            Self::ModeRejected => 10,
            Self::NotReady => 11,
            Self::RectInvalid => 12,
        }
    }
}

/// Snapshot of probe-time device state (no mode change).
#[derive(Clone, Copy, Debug)]
pub struct SvgaProbeInfo {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub revision: u8,
    pub io_bar: PciIoBarInfo,
    pub fb_bar: PciMemoryBarInfo,
    pub fifo_bar: PciMemoryBarInfo,
    pub version_id: u32,
    pub capabilities: u32,
    pub vram_size: u32,
    pub fb_size: u32,
    pub fb_offset: u32,
    pub fifo_size: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub enabled: u32,
    pub config_done: u32,
}

/// Runtime counters for bounded diagnostics (not per-frame logs).
#[derive(Clone, Copy, Debug, Default)]
pub struct SvgaCounters {
    pub probe_attempts: u64,
    pub probe_failures: u64,
    pub activations: u64,
    pub fallback_activations: u64,
    pub full_updates: u64,
    pub rectangular_updates: u64,
    pub fifo_commands: u64,
    pub fifo_wraps: u64,
    pub fifo_waits: u64,
    pub fifo_timeouts: u64,
    pub invalid_damage_rects: u64,
    pub present_failures: u64,
}

/// Live SVGA II device after successful activation.
pub struct VmwareSvga {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub io_base: u16,
    pub fb_bar: PciMemoryBarInfo,
    pub fifo_bar: PciMemoryBarInfo,
    /// HHDM virtual address of mapped FIFO (BAR2).
    pub fifo_virt: u64,
    /// Usable FIFO size from `SVGA_REG_MEM_SIZE` (≤ BAR2 size).
    pub fifo_size: u32,
    pub version_id: u32,
    pub capabilities: u32,
    pub vram_size: u32,
    pub fb_size: u32,
    pub fb_offset: u32,
    /// Guest-physical start of the visible framebuffer (`fb_bar.phys + fb_offset`).
    pub fb_phys: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub stage: SvgaStage,
    pub counters: SvgaCounters,
    /// True when the boot Limine framebuffer physical base lies inside our FB.
    pub boot_fb_in_vram: bool,
}

// SAFETY: register/FIFO access is serialised by the kernel's SVGA_DEVICE mutex.
unsafe impl Send for VmwareSvga {}

impl VmwareSvga {
    /// Index/value register write via BAR0 I/O ports.
    ///
    /// SAFETY: `io_base` must be the enabled SVGA I/O BAR; caller is ring 0.
    #[inline]
    pub unsafe fn reg_write(io_base: u16, index: u32, value: u32) {
        outl(io_base.wrapping_add(SVGA_INDEX_PORT), index);
        outl(io_base.wrapping_add(SVGA_VALUE_PORT), value);
    }

    /// Index/value register read via BAR0 I/O ports.
    ///
    /// SAFETY: as for [`reg_write`].
    #[inline]
    pub unsafe fn reg_read(io_base: u16, index: u32) -> u32 {
        outl(io_base.wrapping_add(SVGA_INDEX_PORT), index);
        inl(io_base.wrapping_add(SVGA_VALUE_PORT))
    }

    #[inline]
    unsafe fn write_reg(&self, index: u32, value: u32) {
        Self::reg_write(self.io_base, index, value);
    }

    #[inline]
    unsafe fn read_reg(&self, index: u32) -> u32 {
        Self::reg_read(self.io_base, index)
    }

    /// Negotiate the highest supported classic SVGA ID (2 → 1 → 0).
    ///
    /// SAFETY: device I/O BAR must be enabled.
    pub unsafe fn negotiate_version(io_base: u16) -> Result<u32, SvgaError> {
        for id in [SVGA_ID_2, SVGA_ID_1, SVGA_ID_0] {
            Self::reg_write(io_base, SVGA_REG_ID, id);
            let readback = Self::reg_read(io_base, SVGA_REG_ID);
            if readback == id {
                return Ok(id);
            }
        }
        let readback = Self::reg_read(io_base, SVGA_REG_ID);
        Err(SvgaError::VersionUnsupported { readback })
    }

    /// Probe-only: read device state without enabling or touching the FIFO.
    ///
    /// SAFETY: PCI info BARs must be valid; I/O and memory decoding enabled.
    pub unsafe fn probe_device(pci: &VmwareSvgaPciInfo) -> Result<SvgaProbeInfo, SvgaError> {
        let version_id = Self::negotiate_version(pci.io_bar.port)?;
        let capabilities = Self::reg_read(pci.io_bar.port, SVGA_REG_CAPABILITIES);
        let vram_size = Self::reg_read(pci.io_bar.port, SVGA_REG_VRAM_SIZE);
        let fb_size = Self::reg_read(pci.io_bar.port, SVGA_REG_FB_SIZE);
        let fb_offset = Self::reg_read(pci.io_bar.port, SVGA_REG_FB_OFFSET);
        let fifo_size = Self::reg_read(pci.io_bar.port, SVGA_REG_MEM_SIZE);
        let max_width = Self::reg_read(pci.io_bar.port, SVGA_REG_MAX_WIDTH);
        let max_height = Self::reg_read(pci.io_bar.port, SVGA_REG_MAX_HEIGHT);
        let width = Self::reg_read(pci.io_bar.port, SVGA_REG_WIDTH);
        let height = Self::reg_read(pci.io_bar.port, SVGA_REG_HEIGHT);
        let pitch = Self::reg_read(pci.io_bar.port, SVGA_REG_BYTES_PER_LINE);
        let bpp = Self::reg_read(pci.io_bar.port, SVGA_REG_BITS_PER_PIXEL);
        let enabled = Self::reg_read(pci.io_bar.port, SVGA_REG_ENABLE);
        let config_done = Self::reg_read(pci.io_bar.port, SVGA_REG_CONFIG_DONE);

        Ok(SvgaProbeInfo {
            bus: pci.bus,
            slot: pci.slot,
            func: pci.func,
            revision: pci.revision,
            io_bar: pci.io_bar,
            fb_bar: pci.fb_bar,
            fifo_bar: pci.fifo_bar,
            version_id,
            capabilities,
            vram_size,
            fb_size,
            fb_offset,
            fifo_size,
            max_width,
            max_height,
            width,
            height,
            pitch,
            bpp,
            enabled,
            config_done,
        })
    }

    /// Validate probe snapshot against BAR sizes and arithmetic invariants.
    pub fn validate_probe(info: &SvgaProbeInfo) -> Result<(), SvgaError> {
        if info.vram_size == 0 || (info.vram_size as u64) > info.fb_bar.size {
            return Err(SvgaError::FbOutsideBar);
        }
        if info.fifo_size == 0 || (info.fifo_size as u64) > info.fifo_bar.size {
            return Err(SvgaError::FifoTooSmall {
                mem_size: info.fifo_size,
                min_bytes: SVGA_MIN_COMMAND_BYTES,
            });
        }
        if info.fb_offset as u64 >= info.fb_bar.size {
            return Err(SvgaError::FbOutsideBar);
        }
        if info.fb_size == 0 {
            return Err(SvgaError::InvalidGeometry);
        }
        let fb_end = (info.fb_offset as u64)
            .checked_add(info.fb_size as u64)
            .ok_or(SvgaError::FbExtentOverflow)?;
        if fb_end > info.fb_bar.size {
            return Err(SvgaError::FbOutsideBar);
        }
        Ok(())
    }

    /// Activate the device for legacy 2D presentation.
    ///
    /// Prefers the current device/boot resolution. Does not change mode when
    /// the hardware already reports a usable 32-bpp mode that fits VRAM.
    ///
    /// `fifo_virt` is the HHDM virtual base of BAR2. `prefer_w`/`prefer_h` are
    /// the boot framebuffer dimensions (0 = ignore). `boot_fb_phys` is the
    /// Limine framebuffer physical base for identity checks.
    ///
    /// SAFETY: FIFO mapping must cover at least `REG_MEM_SIZE` bytes of BAR2;
    /// I/O BAR must be enabled.
    pub unsafe fn activate(
        pci: &VmwareSvgaPciInfo,
        fifo_virt: u64,
        prefer_w: u32,
        prefer_h: u32,
        boot_fb_phys: Option<u64>,
    ) -> Result<Self, SvgaError> {
        let counters = SvgaCounters {
            probe_attempts: 1,
            ..SvgaCounters::default()
        };

        let probe = Self::probe_device(pci)?;
        Self::validate_probe(&probe)?;

        let mut dev = Self {
            bus: pci.bus,
            slot: pci.slot,
            func: pci.func,
            io_base: pci.io_bar.port,
            fb_bar: pci.fb_bar,
            fifo_bar: pci.fifo_bar,
            fifo_virt,
            fifo_size: probe.fifo_size,
            version_id: probe.version_id,
            capabilities: probe.capabilities,
            vram_size: probe.vram_size,
            fb_size: probe.fb_size,
            fb_offset: probe.fb_offset,
            fb_phys: pci
                .fb_bar
                .phys
                .checked_add(probe.fb_offset as u64)
                .ok_or(SvgaError::FbExtentOverflow)?,
            width: probe.width,
            height: probe.height,
            pitch: probe.pitch,
            bpp: probe.bpp,
            stage: SvgaStage::CapabilitiesRead,
            counters,
            boot_fb_in_vram: false,
        };

        if let Some(phys) = boot_fb_phys {
            dev.boot_fb_in_vram = dev.fb_contains_phys(phys, 1);
        }

        dev.init_fifo()?;
        dev.stage = SvgaStage::FifoInitialized;

        // Prefer an already-programmed usable mode (firmware / boot FB).
        let want_w = if prefer_w != 0 {
            prefer_w
        } else {
            probe.width
        };
        let want_h = if prefer_h != 0 {
            prefer_h
        } else {
            probe.height
        };

        let mode_ok = probe.width >= 320
            && probe.height >= 200
            && probe.bpp == 32
            && probe.pitch >= probe.width.saturating_mul(4)
            && (probe.enabled & SVGA_REG_ENABLE_ENABLE) != 0;

        if mode_ok && (prefer_w == 0 || prefer_h == 0 || (probe.width == want_w && probe.height == want_h))
        {
            // Keep firmware mode; ensure enable + config_done.
            if (probe.enabled & SVGA_REG_ENABLE_ENABLE) == 0 {
                dev.write_reg(SVGA_REG_ENABLE, SVGA_REG_ENABLE_ENABLE);
            }
            if probe.config_done == 0 {
                dev.write_reg(SVGA_REG_CONFIG_DONE, 1);
            }
            dev.refresh_mode_regs()?;
        } else {
            dev.set_mode(want_w, want_h, 32)?;
        }

        // Optional traces: help hosts that track FB dirtiness without FIFO.
        if (dev.capabilities & SVGA_CAP_TRACES) != 0 {
            dev.write_reg(SVGA_REG_TRACES, 1);
        }

        dev.stage = SvgaStage::DisplayUsable;
        dev.counters.activations = 1;
        dev.stage = SvgaStage::Active;
        Ok(dev)
    }

    fn fb_contains_phys(&self, phys: u64, len: u64) -> bool {
        let Some(end) = phys.checked_add(len) else {
            return false;
        };
        let Some(fb_end) = self.fb_phys.checked_add(self.fb_size as u64) else {
            return false;
        };
        phys >= self.fb_phys && end <= fb_end
    }

    /// Initialize FIFO MIN/MAX/NEXT/STOP and set CONFIG_DONE.
    ///
    /// SAFETY: FIFO mapping is live and sized for `fifo_size`.
    unsafe fn init_fifo(&mut self) -> Result<(), SvgaError> {
        let mem_regs = if (self.capabilities & SVGA_CAP_EXTENDED_FIFO) != 0 {
            let n = self.read_reg(SVGA_REG_MEM_REGS);
            if n < 4 {
                4
            } else {
                n
            }
        } else {
            4
        };

        // Byte offset of the first command slot. Match Linux: at least one
        // page when extended FIFO is large enough; never exceed half the FIFO.
        let mut min_bytes = mem_regs.saturating_mul(4);
        if (self.capabilities & SVGA_CAP_EXTENDED_FIFO) != 0 && min_bytes < 4096 {
            min_bytes = 4096;
        }
        if min_bytes < 16 {
            min_bytes = 16;
        }

        let max_bytes = self.fifo_size;
        if max_bytes <= min_bytes || max_bytes - min_bytes < SVGA_MIN_COMMAND_BYTES {
            return Err(SvgaError::FifoTooSmall {
                mem_size: max_bytes,
                min_bytes,
            });
        }

        // Zero only the FIFO header region we own; do not clear command history
        // beyond that (host may not care, but avoid large memset on VRAM-like mem).
        let header_words = (min_bytes / 4) as usize;
        for i in 0..header_words.min(512) {
            self.fifo_store(i as u32, 0);
        }

        self.fifo_store(SVGA_FIFO_MIN, min_bytes);
        self.fifo_store(SVGA_FIFO_MAX, max_bytes);
        // Publish MIN/MAX before NEXT/STOP (host samples after CONFIG_DONE).
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.fifo_store(SVGA_FIFO_NEXT_CMD, min_bytes);
        self.fifo_store(SVGA_FIFO_STOP, min_bytes);
        if (self.capabilities & SVGA_CAP_EXTENDED_FIFO) != 0 {
            // FIFO_BUSY index exists when extended; clear it.
            self.fifo_store(SVGA_FIFO_BUSY, 0);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        self.write_reg(SVGA_REG_CONFIG_DONE, 1);

        let min_r = self.fifo_load(SVGA_FIFO_MIN);
        let max_r = self.fifo_load(SVGA_FIFO_MAX);
        if min_r >= max_r || max_r > self.fifo_size {
            return Err(SvgaError::FifoInvariant);
        }
        Ok(())
    }

    /// Program width/height/bpp and re-read pitch. Leaves the device enabled.
    ///
    /// SAFETY: register I/O only.
    pub unsafe fn set_mode(&mut self, width: u32, height: u32, bpp: u32) -> Result<(), SvgaError> {
        let max_w = self.read_reg(SVGA_REG_MAX_WIDTH);
        let max_h = self.read_reg(SVGA_REG_MAX_HEIGHT);
        if width < 320 || height < 200 || width > max_w || height > max_h || bpp != 32 {
            return Err(SvgaError::ModeRejected);
        }

        // Disable before mode registers (avoids host seeing a half-written mode).
        self.write_reg(SVGA_REG_ENABLE, SVGA_REG_ENABLE_DISABLE);
        self.write_reg(SVGA_REG_WIDTH, width);
        self.write_reg(SVGA_REG_HEIGHT, height);
        self.write_reg(SVGA_REG_BITS_PER_PIXEL, bpp);
        self.write_reg(SVGA_REG_ENABLE, SVGA_REG_ENABLE_ENABLE);
        self.write_reg(SVGA_REG_CONFIG_DONE, 1);

        self.refresh_mode_regs()?;
        if self.width != width || self.height != height || self.bpp != 32 {
            return Err(SvgaError::ModeRejected);
        }
        Ok(())
    }

    unsafe fn refresh_mode_regs(&mut self) -> Result<(), SvgaError> {
        self.width = self.read_reg(SVGA_REG_WIDTH);
        self.height = self.read_reg(SVGA_REG_HEIGHT);
        self.pitch = self.read_reg(SVGA_REG_BYTES_PER_LINE);
        self.bpp = self.read_reg(SVGA_REG_BITS_PER_PIXEL);
        self.fb_offset = self.read_reg(SVGA_REG_FB_OFFSET);
        self.fb_size = self.read_reg(SVGA_REG_FB_SIZE);
        self.fb_phys = self
            .fb_bar
            .phys
            .checked_add(self.fb_offset as u64)
            .ok_or(SvgaError::FbExtentOverflow)?;

        if self.width == 0 || self.height == 0 || self.bpp != 32 {
            return Err(SvgaError::InvalidGeometry);
        }
        if self.pitch < self.width.saturating_mul(4) {
            return Err(SvgaError::InvalidPitch);
        }
        let need = (self.pitch as u64)
            .checked_mul(self.height as u64)
            .ok_or(SvgaError::FbExtentOverflow)?;
        let avail = (self.fb_size as u64).min(
            self.fb_bar
                .size
                .saturating_sub(self.fb_offset as u64),
        );
        if need > avail {
            return Err(SvgaError::FbExtentOverflow);
        }
        Ok(())
    }

    #[inline]
    unsafe fn fifo_ptr(&self, word_index: u32) -> *mut u32 {
        (self.fifo_virt as *mut u32).add(word_index as usize)
    }

    #[inline]
    unsafe fn fifo_store(&self, word_index: u32, value: u32) {
        core::ptr::write_volatile(self.fifo_ptr(word_index), value);
    }

    #[inline]
    unsafe fn fifo_load(&self, word_index: u32) -> u32 {
        core::ptr::read_volatile(self.fifo_ptr(word_index))
    }

    /// Bytes of free space in the FIFO command ring (Linux-compatible estimate).
    unsafe fn fifo_free_bytes(&self) -> u32 {
        let min = self.fifo_load(SVGA_FIFO_MIN);
        let max = self.fifo_load(SVGA_FIFO_MAX);
        let next = self.fifo_load(SVGA_FIFO_NEXT_CMD);
        let stop = self.fifo_load(SVGA_FIFO_STOP);
        if min >= max || next < min || next >= max || stop < min || stop >= max {
            return 0;
        }
        // When next == stop the FIFO is empty; free = max - min.
        // Linux: (max - next) + (stop - min)
        (max - next).wrapping_add(stop - min)
    }

    unsafe fn fifo_wait_space(&mut self, bytes: u32) -> Result<(), SvgaError> {
        // Need free > bytes (leave at least one gap so empty/full stay distinct).
        let need = bytes.saturating_add(4);
        for _ in 0..SVGA_FIFO_WAIT_SPINS {
            if self.fifo_free_bytes() > need {
                return Ok(());
            }
            self.counters.fifo_waits = self.counters.fifo_waits.saturating_add(1);
            // Bounce the host to make progress on STOP.
            self.write_reg(SVGA_REG_SYNC, SVGA_SYNC_GENERIC);
            core::hint::spin_loop();
        }
        self.counters.fifo_timeouts = self.counters.fifo_timeouts.saturating_add(1);
        Err(SvgaError::FifoTimeout)
    }

    /// Write `words` into the FIFO with wrap, then bounce the host.
    ///
    /// SAFETY: FIFO mapping live; words length is a multiple of the command format.
    pub unsafe fn fifo_write(&mut self, words: &[u32]) -> Result<(), SvgaError> {
        if words.is_empty() {
            return Ok(());
        }
        let bytes = (words.len() as u32).checked_mul(4).ok_or(SvgaError::FifoInvariant)?;
        self.fifo_wait_space(bytes)?;

        let min = self.fifo_load(SVGA_FIFO_MIN);
        let max = self.fifo_load(SVGA_FIFO_MAX);
        let mut next = self.fifo_load(SVGA_FIFO_NEXT_CMD);
        if min >= max || next < min || next >= max {
            return Err(SvgaError::FifoInvariant);
        }

        for &word in words {
            // Store as a 32-bit word at byte offset `next`.
            let word_index = next / 4;
            self.fifo_store(word_index, word);
            next = next.wrapping_add(4);
            if next >= max {
                next = min;
                self.counters.fifo_wraps = self.counters.fifo_wraps.saturating_add(1);
            }
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.fifo_store(SVGA_FIFO_NEXT_CMD, next);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.write_reg(SVGA_REG_SYNC, SVGA_SYNC_GENERIC);
        self.counters.fifo_commands = self.counters.fifo_commands.saturating_add(1);
        Ok(())
    }

    /// Submit `SVGA_CMD_UPDATE` for a rectangle in device pixel coordinates.
    pub unsafe fn update_rect(&mut self, x: u32, y: u32, w: u32, h: u32) -> Result<(), SvgaError> {
        if self.stage != SvgaStage::Active && self.stage != SvgaStage::DisplayUsable {
            return Err(SvgaError::NotReady);
        }
        if w == 0 || h == 0 {
            self.counters.invalid_damage_rects =
                self.counters.invalid_damage_rects.saturating_add(1);
            return Err(SvgaError::RectInvalid);
        }
        if x >= self.width || y >= self.height {
            self.counters.invalid_damage_rects =
                self.counters.invalid_damage_rects.saturating_add(1);
            return Err(SvgaError::RectInvalid);
        }
        let max_w = self.width - x;
        let max_h = self.height - y;
        let w = w.min(max_w);
        let h = h.min(max_h);
        if w == 0 || h == 0 {
            self.counters.invalid_damage_rects =
                self.counters.invalid_damage_rects.saturating_add(1);
            return Err(SvgaError::RectInvalid);
        }

        // Guest FB stores must be globally visible before the host processes UPDATE.
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        let cmd = [SVGA_CMD_UPDATE, x, y, w, h];
        match self.fifo_write(&cmd) {
            Ok(()) => {
                if x == 0 && y == 0 && w == self.width && h == self.height {
                    self.counters.full_updates = self.counters.full_updates.saturating_add(1);
                } else {
                    self.counters.rectangular_updates =
                        self.counters.rectangular_updates.saturating_add(1);
                }
                Ok(())
            }
            Err(e) => {
                self.counters.present_failures = self.counters.present_failures.saturating_add(1);
                Err(e)
            }
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.stage, SvgaStage::Active | SvgaStage::DisplayUsable)
            && self.width > 0
            && self.height > 0
            && self.bpp == 32
            && self.pitch >= self.width.saturating_mul(4)
    }
}
