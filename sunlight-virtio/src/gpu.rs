use core::sync::atomic::{fence, Ordering};
use super::pci::VirtioGpuPciInfo;

// ---------------------------------------------------------------------------
// VirtIO device status bits (modern)
// ---------------------------------------------------------------------------
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const STATUS_FEATURES_OK: u8 = 8;

// VirtIO GPU feature bit we mask off (we don't want 3D)
const VIRTIO_GPU_F_VIRGL: u32 = 1 << 0;

// ---------------------------------------------------------------------------
// VirtIO PCI common config offsets (§4.1.4.3 of VirtIO spec)
// ---------------------------------------------------------------------------
const COMMON_CFG_DEVICE_FEATURE_SEL: usize = 0x00;
const COMMON_CFG_DEVICE_FEATURE: usize    = 0x04;
const COMMON_CFG_DRIVER_FEATURE_SEL: usize = 0x08;
const COMMON_CFG_DRIVER_FEATURE: usize    = 0x0C;
const COMMON_CFG_DEVICE_STATUS: usize     = 0x14;
const COMMON_CFG_QUEUE_SELECT: usize      = 0x16;
const COMMON_CFG_QUEUE_SIZE: usize        = 0x18;
const COMMON_CFG_QUEUE_ENABLE: usize      = 0x1C;
const COMMON_CFG_QUEUE_NOTIFY_OFF: usize  = 0x1E;
const COMMON_CFG_QUEUE_DESC: usize        = 0x20;
const COMMON_CFG_QUEUE_DRIVER: usize      = 0x28;
const COMMON_CFG_QUEUE_DEVICE: usize      = 0x30;

// ---------------------------------------------------------------------------
// Virtqueue descriptor flags
// ---------------------------------------------------------------------------
const DESC_F_NEXT: u16  = 1;
const DESC_F_WRITE: u16 = 2;

// Queue indices
const QUEUE_CONTROLQ: u16 = 0;
const QUEUE_CURSORQ: u16  = 1;

/// Queue size: 64 entries. Sufficient for serial GPU commands with polling.
const QUEUE_SIZE: u16 = 64;

// ---------------------------------------------------------------------------
// VirtIO GPU command type codes (§5.7.6)
// ---------------------------------------------------------------------------
const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32      = 0x0100;
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32    = 0x0101;
const VIRTIO_GPU_CMD_SET_SCANOUT: u32           = 0x0103;
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32        = 0x0104;
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32   = 0x0105;
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const VIRTIO_GPU_CMD_UPDATE_CURSOR: u32         = 0x0300;
const VIRTIO_GPU_CMD_MOVE_CURSOR: u32           = 0x0301;

const VIRTIO_GPU_RESP_OK_NODATA: u32            = 0x1100;
const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32      = 0x1101;

// Pixel formats (§5.7.6.8)
/// XRGB 32-bit — matches the existing display server back_buffer format.
pub const VIRTIO_GPU_FORMAT_X8R8G8B8_UNORM: u32 = 4;
/// BGRA 32-bit — cursor resource (supports alpha for transparency).
pub const VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM: u32 = 1;

/// Resource ID for the scanout (back_buffer).
pub const SCANOUT_RESOURCE_ID: u32 = 1;
/// Resource ID for the hardware cursor.
pub const CURSOR_RESOURCE_ID: u32  = 2;
/// Scanout index.
pub const SCANOUT_ID: u32          = 0;

/// Hardware cursor dimensions.
pub const CURSOR_W: u32 = 64;
pub const CURSOR_H: u32 = 64;

// ---------------------------------------------------------------------------
// GPU command/response structs
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtioGpuCtrlHdr {
    pub ctrl_type: u32,
    pub flags:     u32,
    pub fence_id:  u64,
    pub ctx_id:    u32,
    pub padding:   u32,
}

impl VirtioGpuCtrlHdr {
    pub fn cmd(ctrl_type: u32) -> Self {
        Self { ctrl_type, flags: 0, fence_id: 0, ctx_id: 0, padding: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtioGpuDisplayOne {
    pub r_x: u32, pub r_y: u32, pub r_w: u32, pub r_h: u32,
    pub enabled: u32,
    pub flags:   u32,
}

#[repr(C)]
pub struct VirtioGpuRespDisplayInfo {
    pub hdr:    VirtioGpuCtrlHdr,
    pub pmodes: [VirtioGpuDisplayOne; 16],
}

#[repr(C)]
pub struct VirtioGpuResourceCreate2d {
    pub hdr:         VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub format:      u32,
    pub width:       u32,
    pub height:      u32,
}

#[repr(C)]
pub struct VirtioGpuResourceAttachBacking {
    pub hdr:         VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub nr_entries:  u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtioGpuMemEntry {
    pub addr:    u64,
    pub length:  u32,
    pub padding: u32,
}

#[repr(C)]
pub struct VirtioGpuSetScanout {
    pub hdr:         VirtioGpuCtrlHdr,
    pub r_x: u32, pub r_y: u32, pub r_w: u32, pub r_h: u32,
    pub scanout_id:  u32,
    pub resource_id: u32,
}

#[repr(C)]
pub struct VirtioGpuTransferToHost2d {
    pub hdr:         VirtioGpuCtrlHdr,
    pub r_x: u32, pub r_y: u32, pub r_w: u32, pub r_h: u32,
    pub offset:      u64,
    pub resource_id: u32,
    pub padding:     u32,
}

#[repr(C)]
pub struct VirtioGpuResourceFlush {
    pub hdr:         VirtioGpuCtrlHdr,
    pub r_x: u32, pub r_y: u32, pub r_w: u32, pub r_h: u32,
    pub resource_id: u32,
    pub padding:     u32,
}

/// Used for both UPDATE_CURSOR and MOVE_CURSOR (same struct layout).
#[repr(C)]
pub struct VirtioGpuUpdateCursor {
    pub hdr:         VirtioGpuCtrlHdr,
    pub scanout_id:  u32,
    pub pos_x:       u32,
    pub pos_y:       u32,
    pub padding0:    u32,
    pub resource_id: u32,
    pub hot_x:       u32,
    pub hot_y:       u32,
    pub padding1:    u32,
}

// ---------------------------------------------------------------------------
// Virtqueue descriptor (same memory layout as blk/net)
// ---------------------------------------------------------------------------
#[repr(C)]
struct VirtqDesc {
    addr:  u64,
    len:   u32,
    flags: u16,
    next:  u16,
}

/// State for one split virtqueue.
struct Virtq {
    queue_size:    u16,
    desc_virt:     u64,
    avail_virt:    u64,
    used_virt:     u64,
    avail_idx:     u16,
    last_used_idx: u16,
    /// Per-queue notify offset (read from common_cfg after queue select).
    notify_off:    u16,
}

// ---------------------------------------------------------------------------
// VirtIO GPU device driver
// ---------------------------------------------------------------------------

pub struct VirtioGpu {
    /// HHDM-mapped pointer to the VirtIO common config MMIO region.
    common_cfg:         *mut u8,
    /// HHDM-mapped base of the queue notification MMIO region.
    notify_base:        u64,
    notify_multiplier:  u32,

    controlq: Virtq,
    cursorq:  Virtq,

    /// 1-page command/response scratch buffer (physical and virtual).
    cmd_phys: u64,
    cmd_virt: u64,

    /// 4-page scatter-gather buffer for RESOURCE_ATTACH_BACKING entries.
    sg_phys: u64,
    sg_virt: u64,

    /// 4-page cursor resource backing (64×64×4 = 16384 bytes).
    cursor_phys: u64,
    cursor_virt: u64,

    /// Display dimensions detected at init.
    pub width:  u32,
    pub height: u32,
}

// SAFETY: VirtioGpu holds raw pointers into kernel-owned MMIO and physical frames.
// Access is serialised through the GPU_DEVICE spin::Mutex.
unsafe impl Send for VirtioGpu {}

// ---------------------------------------------------------------------------
// MMIO volatile helpers
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn wr8(base: *mut u8, off: usize, val: u8) {
    base.add(off).write_volatile(val);
}
#[inline(always)]
unsafe fn rd8(base: *mut u8, off: usize) -> u8 {
    base.add(off).read_volatile()
}
#[inline(always)]
unsafe fn wr16(base: *mut u8, off: usize, val: u16) {
    (base.add(off) as *mut u16).write_volatile(val);
}
#[inline(always)]
unsafe fn rd16(base: *mut u8, off: usize) -> u16 {
    (base.add(off) as *mut u16).read_volatile()
}
#[inline(always)]
unsafe fn wr32(base: *mut u8, off: usize, val: u32) {
    (base.add(off) as *mut u32).write_volatile(val);
}
#[inline(always)]
unsafe fn rd32(base: *mut u8, off: usize) -> u32 {
    (base.add(off) as *mut u32).read_volatile()
}
#[inline(always)]
unsafe fn wr64(base: *mut u8, off: usize, val: u64) {
    (base.add(off) as *mut u64).write_volatile(val);
}

impl VirtioGpu {
    /// Initialize VirtIO GPU from PCI discovery info.
    ///
    /// All physical/virtual pairs must be valid kernel-allocated frames; caller must be ring-0.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn init(
        info:         &VirtioGpuPciInfo,
        hhdm:         u64,
        ctrl_q_phys:  u64,
        ctrl_q_virt:  u64,
        cur_q_phys:   u64,
        cur_q_virt:   u64,
        cmd_phys:     u64,
        cmd_virt:     u64,
        sg_phys:      u64,
        sg_virt:      u64,
        cursor_phys:  u64,
        cursor_virt:  u64,
    ) -> Option<Self> {
        let common_cfg = (hhdm + info.common_cfg_phys + info.common_cfg_off as u64) as *mut u8;
        let notify_base = hhdm + info.notify_phys + info.notify_off as u64;

        // 1. Reset
        wr8(common_cfg, COMMON_CFG_DEVICE_STATUS, 0);
        fence(Ordering::SeqCst);

        // 2. ACKNOWLEDGE + DRIVER
        wr8(common_cfg, COMMON_CFG_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
        fence(Ordering::SeqCst);

        // 3. Feature negotiation — page 0 only; disable VIRGL (3D).
        wr32(common_cfg, COMMON_CFG_DEVICE_FEATURE_SEL, 0);
        fence(Ordering::SeqCst);
        let dev_feats = rd32(common_cfg, COMMON_CFG_DEVICE_FEATURE);
        wr32(common_cfg, COMMON_CFG_DRIVER_FEATURE_SEL, 0);
        wr32(common_cfg, COMMON_CFG_DRIVER_FEATURE, dev_feats & !VIRTIO_GPU_F_VIRGL);
        fence(Ordering::SeqCst);

        // 4. FEATURES_OK
        wr8(common_cfg, COMMON_CFG_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK);
        fence(Ordering::SeqCst);
        if rd8(common_cfg, COMMON_CFG_DEVICE_STATUS) & STATUS_FEATURES_OK == 0 {
            return None;
        }

        // 5. Setup queues
        let controlq = Self::setup_queue(common_cfg, QUEUE_CONTROLQ, ctrl_q_phys, ctrl_q_virt)?;
        let cursorq  = Self::setup_queue(common_cfg, QUEUE_CURSORQ,  cur_q_phys,  cur_q_virt)?;

        // 6. DRIVER_OK
        wr8(common_cfg, COMMON_CFG_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK);
        fence(Ordering::SeqCst);

        // Zero scratch and cursor backing
        (cmd_virt as *mut u8).write_bytes(0, 4096);
        (cursor_virt as *mut u8).write_bytes(0, 4 * 4096);

        Some(VirtioGpu {
            common_cfg,
            notify_base,
            notify_multiplier: info.notify_off_multiplier,
            controlq,
            cursorq,
            cmd_phys,
            cmd_virt,
            sg_phys,
            sg_virt,
            cursor_phys,
            cursor_virt,
            width: 0,
            height: 0,
        })
    }

    unsafe fn setup_queue(
        common_cfg: *mut u8,
        queue_idx:  u16,
        q_phys:     u64,
        q_virt:     u64,
    ) -> Option<Virtq> {
        wr16(common_cfg, COMMON_CFG_QUEUE_SELECT, queue_idx);
        fence(Ordering::SeqCst);

        let max_size = rd16(common_cfg, COMMON_CFG_QUEUE_SIZE);
        if max_size == 0 { return None; }
        let qsize = max_size.min(QUEUE_SIZE);

        wr16(common_cfg, COMMON_CFG_QUEUE_SIZE, qsize);

        // Layout: [desc][avail][padding][used] within the 2-page region.
        let desc_off: u64  = 0;
        let avail_off: u64 = (qsize as u64) * 16;
        let avail_end: u64 = avail_off + 6 + (qsize as u64) * 2;
        let used_off: u64  = (avail_end + 4095) & !4095;

        (q_virt as *mut u8).write_bytes(0, 8192);

        wr64(common_cfg, COMMON_CFG_QUEUE_DESC,   q_phys + desc_off);
        wr64(common_cfg, COMMON_CFG_QUEUE_DRIVER, q_phys + avail_off);
        wr64(common_cfg, COMMON_CFG_QUEUE_DEVICE, q_phys + used_off);
        wr16(common_cfg, COMMON_CFG_QUEUE_ENABLE, 1);
        fence(Ordering::SeqCst);

        let notify_off = rd16(common_cfg, COMMON_CFG_QUEUE_NOTIFY_OFF);

        Some(Virtq {
            queue_size: qsize,
            desc_virt:  q_virt + desc_off,
            avail_virt: q_virt + avail_off,
            used_virt:  q_virt + used_off,
            avail_idx:     0,
            last_used_idx: 0,
            notify_off,
        })
    }

    // -------------------------------------------------------------------------
    // High-level GPU operations (all use the command scratch buffer)
    // -------------------------------------------------------------------------

    /// Probe scanout 0 dimensions. Returns (width, height) or None on failure.
    pub unsafe fn get_display_info(&mut self) -> Option<(u32, u32)> {
        let cmd_v = self.cmd_virt as *mut VirtioGpuCtrlHdr;
        let rsp_v = (self.cmd_virt + 512) as *mut VirtioGpuRespDisplayInfo;
        let rsp_type_v = (self.cmd_virt + 512) as *const u32;

        *cmd_v = VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_GET_DISPLAY_INFO);
        (rsp_v as *mut u8).write_bytes(0, core::mem::size_of::<VirtioGpuRespDisplayInfo>());
        fence(Ordering::SeqCst);

        let ok = self.ctrl2(
            self.cmd_phys, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            self.cmd_phys + 512, core::mem::size_of::<VirtioGpuRespDisplayInfo>() as u32,
            rsp_type_v,
        );
        if !ok { return None; }

        let rsp = &*rsp_v;
        let w = rsp.pmodes[0].r_w;
        let h = rsp.pmodes[0].r_h;
        if w == 0 || h == 0 { None } else { Some((w, h)) }
    }

    /// Create a 2D resource.
    pub unsafe fn resource_create_2d(
        &mut self, resource_id: u32, format: u32, w: u32, h: u32,
    ) -> bool {
        let cmd_v = self.cmd_virt as *mut VirtioGpuResourceCreate2d;
        let rsp_v = (self.cmd_virt + 512) as *const u32;
        *cmd_v = VirtioGpuResourceCreate2d {
            hdr: VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
            resource_id, format, width: w, height: h,
        };
        fence(Ordering::SeqCst);
        self.ctrl2(
            self.cmd_phys, core::mem::size_of::<VirtioGpuResourceCreate2d>() as u32,
            self.cmd_phys + 512, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            rsp_v,
        )
    }

    /// Attach backing memory to a resource (scatter-gather list via sg buffer).
    pub unsafe fn resource_attach_backing(
        &mut self,
        resource_id: u32,
        entries: &[VirtioGpuMemEntry],
    ) -> bool {
        // Command header in cmd buffer
        let cmd_v = self.cmd_virt as *mut VirtioGpuResourceAttachBacking;
        *cmd_v = VirtioGpuResourceAttachBacking {
            hdr: VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
            resource_id,
            nr_entries: entries.len() as u32,
        };
        let cmd_phys = self.cmd_phys;
        let cmd_len  = core::mem::size_of::<VirtioGpuResourceAttachBacking>() as u32;

        // Copy entries to sg buffer
        let max_n = (4 * 4096) / core::mem::size_of::<VirtioGpuMemEntry>();
        let n = entries.len().min(max_n);
        let sg_v = self.sg_virt as *mut VirtioGpuMemEntry;
        for i in 0..n {
            sg_v.add(i).write_volatile(entries[i]);
        }
        let entries_phys = self.sg_phys;
        let entries_len  = (n * core::mem::size_of::<VirtioGpuMemEntry>()) as u32;

        let resp_phys    = self.cmd_phys + 512;
        let resp_len     = core::mem::size_of::<VirtioGpuCtrlHdr>() as u32;
        (self.cmd_virt as *mut u8).add(512).write_bytes(0, resp_len as usize);

        let rsp_type_v = (self.cmd_virt + 512) as *const u32;
        fence(Ordering::SeqCst);

        self.ctrl3(
            cmd_phys, cmd_len,
            entries_phys, entries_len,
            resp_phys, resp_len,
            rsp_type_v,
        )
    }

    /// Wire a resource to a scanout.
    pub unsafe fn set_scanout(
        &mut self, scanout_id: u32, resource_id: u32, w: u32, h: u32,
    ) -> bool {
        let cmd_v = self.cmd_virt as *mut VirtioGpuSetScanout;
        let rsp_v = (self.cmd_virt + 512) as *const u32;
        *cmd_v = VirtioGpuSetScanout {
            hdr: VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_SET_SCANOUT),
            r_x: 0, r_y: 0, r_w: w, r_h: h,
            scanout_id, resource_id,
        };
        fence(Ordering::SeqCst);
        self.ctrl2(
            self.cmd_phys, core::mem::size_of::<VirtioGpuSetScanout>() as u32,
            self.cmd_phys + 512, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            rsp_v,
        )
    }

    /// TRANSFER_TO_HOST_2D for the given dirty rect.
    /// `stride` is the width of the full resource in pixels.
    pub unsafe fn transfer_to_host_2d(
        &mut self, resource_id: u32, x: u32, y: u32, w: u32, h: u32, stride: u32,
    ) -> bool {
        let offset = (y as u64) * (stride as u64 * 4) + (x as u64 * 4);
        let cmd_v  = self.cmd_virt as *mut VirtioGpuTransferToHost2d;
        let rsp_v  = (self.cmd_virt + 512) as *const u32;
        *cmd_v = VirtioGpuTransferToHost2d {
            hdr: VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
            r_x: x, r_y: y, r_w: w, r_h: h,
            offset, resource_id, padding: 0,
        };
        fence(Ordering::SeqCst);
        self.ctrl2(
            self.cmd_phys, core::mem::size_of::<VirtioGpuTransferToHost2d>() as u32,
            self.cmd_phys + 512, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            rsp_v,
        )
    }

    /// RESOURCE_FLUSH for the given dirty rect.
    pub unsafe fn resource_flush(
        &mut self, resource_id: u32, x: u32, y: u32, w: u32, h: u32,
    ) -> bool {
        let cmd_v = self.cmd_virt as *mut VirtioGpuResourceFlush;
        let rsp_v = (self.cmd_virt + 512) as *const u32;
        *cmd_v = VirtioGpuResourceFlush {
            hdr: VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
            r_x: x, r_y: y, r_w: w, r_h: h,
            resource_id, padding: 0,
        };
        fence(Ordering::SeqCst);
        self.ctrl2(
            self.cmd_phys, core::mem::size_of::<VirtioGpuResourceFlush>() as u32,
            self.cmd_phys + 512, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            rsp_v,
        )
    }

    /// UPDATE_CURSOR — upload a new cursor image and set its position.
    pub unsafe fn update_cursor(
        &mut self,
        scanout_id: u32, resource_id: u32,
        x: u32, y: u32,
        hot_x: u32, hot_y: u32,
    ) -> bool {
        let cmd_v = self.cmd_virt as *mut VirtioGpuUpdateCursor;
        let rsp_v = (self.cmd_virt + 512) as *const u32;
        *cmd_v = VirtioGpuUpdateCursor {
            hdr: VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_UPDATE_CURSOR),
            scanout_id, pos_x: x, pos_y: y, padding0: 0,
            resource_id, hot_x, hot_y, padding1: 0,
        };
        fence(Ordering::SeqCst);
        self.cursor2(
            self.cmd_phys, core::mem::size_of::<VirtioGpuUpdateCursor>() as u32,
            self.cmd_phys + 512, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            rsp_v,
        )
    }

    /// MOVE_CURSOR — update position without changing the cursor image.
    pub unsafe fn move_cursor(&mut self, scanout_id: u32, x: u32, y: u32) -> bool {
        let cmd_v = self.cmd_virt as *mut VirtioGpuUpdateCursor;
        let rsp_v = (self.cmd_virt + 512) as *const u32;
        *cmd_v = VirtioGpuUpdateCursor {
            hdr: VirtioGpuCtrlHdr::cmd(VIRTIO_GPU_CMD_MOVE_CURSOR),
            scanout_id, pos_x: x, pos_y: y, padding0: 0,
            resource_id: 0, hot_x: 0, hot_y: 0, padding1: 0,
        };
        fence(Ordering::SeqCst);
        self.cursor2(
            self.cmd_phys, core::mem::size_of::<VirtioGpuUpdateCursor>() as u32,
            self.cmd_phys + 512, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            rsp_v,
        )
    }

    /// Virtual address of the cursor backing pixels (64×64 BGRA, kernel-owned).
    pub fn cursor_pixels_virt(&self) -> u64 { self.cursor_virt }

    /// Physical scatter-gather entries for the 4 cursor backing pages.
    pub fn cursor_pages_phys(&self) -> [VirtioGpuMemEntry; 4] {
        [
            VirtioGpuMemEntry { addr: self.cursor_phys,            length: 4096, padding: 0 },
            VirtioGpuMemEntry { addr: self.cursor_phys + 4096,     length: 4096, padding: 0 },
            VirtioGpuMemEntry { addr: self.cursor_phys + 4096 * 2, length: 4096, padding: 0 },
            VirtioGpuMemEntry { addr: self.cursor_phys + 4096 * 3, length: 4096, padding: 0 },
        ]
    }

    // -------------------------------------------------------------------------
    // Internal queue helpers
    // -------------------------------------------------------------------------

    /// Send a 2-descriptor chain on the control queue.
    /// `rsp_type_v`: virtual *const u32 pointing at the response type field (for polling).
    unsafe fn ctrl2(
        &mut self,
        cmd_phys: u64, cmd_len: u32,
        rsp_phys: u64, rsp_len: u32,
        rsp_type_v: *const u32,
    ) -> bool {
        let q    = &mut self.controlq;
        let nb   = self.notify_base;
        let mult = self.notify_multiplier;
        Self::submit2(q, QUEUE_CONTROLQ, cmd_phys, cmd_len, rsp_phys, rsp_len, nb, mult);
        Self::poll(q, rsp_type_v)
    }

    /// 3-descriptor chain on the control queue (ATTACH_BACKING).
    unsafe fn ctrl3(
        &mut self,
        cmd_phys:     u64, cmd_len:     u32,
        extra_phys:   u64, extra_len:   u32,
        rsp_phys:     u64, rsp_len:     u32,
        rsp_type_v:   *const u32,
    ) -> bool {
        let q    = &mut self.controlq;
        let nb   = self.notify_base;
        let mult = self.notify_multiplier;
        Self::submit3(q, QUEUE_CONTROLQ, cmd_phys, cmd_len, extra_phys, extra_len, rsp_phys, rsp_len, nb, mult);
        Self::poll(q, rsp_type_v)
    }

    /// 2-descriptor chain on the cursor queue.
    unsafe fn cursor2(
        &mut self,
        cmd_phys: u64, cmd_len: u32,
        rsp_phys: u64, rsp_len: u32,
        rsp_type_v: *const u32,
    ) -> bool {
        let q    = &mut self.cursorq;
        let nb   = self.notify_base;
        let mult = self.notify_multiplier;
        Self::submit2(q, QUEUE_CURSORQ, cmd_phys, cmd_len, rsp_phys, rsp_len, nb, mult);
        Self::poll(q, rsp_type_v)
    }

    /// Build and submit a 2-descriptor chain to a queue and notify the device.
    unsafe fn submit2(
        q:       &mut Virtq,
        q_idx:   u16,
        d0_phys: u64, d0_len: u32,
        d1_phys: u64, d1_len: u32,
        notify_base: u64, notify_mult: u32,
    ) {
        let descs = q.desc_virt as *mut VirtqDesc;
        *descs.add(0) = VirtqDesc { addr: d0_phys, len: d0_len, flags: DESC_F_NEXT,  next: 1 };
        *descs.add(1) = VirtqDesc { addr: d1_phys, len: d1_len, flags: DESC_F_WRITE, next: 0 };

        let slot = (q.avail_idx as usize) % (q.queue_size as usize);
        ((q.avail_virt + 4) as *mut u16).add(slot).write_volatile(0);
        fence(Ordering::SeqCst);
        let new_idx = q.avail_idx.wrapping_add(1);
        ((q.avail_virt + 2) as *mut u16).write_volatile(new_idx);
        q.avail_idx = new_idx;
        fence(Ordering::SeqCst);

        let notify_addr = (notify_base + (q.notify_off as u64) * (notify_mult as u64)) as *mut u16;
        notify_addr.write_volatile(q_idx);
    }

    /// Build and submit a 3-descriptor chain.
    unsafe fn submit3(
        q:       &mut Virtq,
        q_idx:   u16,
        d0_phys: u64, d0_len: u32,
        d1_phys: u64, d1_len: u32,
        d2_phys: u64, d2_len: u32,
        notify_base: u64, notify_mult: u32,
    ) {
        let descs = q.desc_virt as *mut VirtqDesc;
        *descs.add(0) = VirtqDesc { addr: d0_phys, len: d0_len, flags: DESC_F_NEXT,  next: 1 };
        *descs.add(1) = VirtqDesc { addr: d1_phys, len: d1_len, flags: DESC_F_NEXT,  next: 2 };
        *descs.add(2) = VirtqDesc { addr: d2_phys, len: d2_len, flags: DESC_F_WRITE, next: 0 };

        let slot = (q.avail_idx as usize) % (q.queue_size as usize);
        ((q.avail_virt + 4) as *mut u16).add(slot).write_volatile(0);
        fence(Ordering::SeqCst);
        let new_idx = q.avail_idx.wrapping_add(1);
        ((q.avail_virt + 2) as *mut u16).write_volatile(new_idx);
        q.avail_idx = new_idx;
        fence(Ordering::SeqCst);

        let notify_addr = (notify_base + (q.notify_off as u64) * (notify_mult as u64)) as *mut u16;
        notify_addr.write_volatile(q_idx);
    }

    /// Spin-poll the used ring for one completion entry.
    unsafe fn poll(q: &mut Virtq, rsp_type_v: *const u32) -> bool {
        let used_idx_ptr = (q.used_virt + 2) as *const u16;
        let mut limit = 10_000_000u32;
        loop {
            fence(Ordering::SeqCst);
            if used_idx_ptr.read_volatile() != q.last_used_idx {
                break;
            }
            limit -= 1;
            if limit == 0 { return false; }
            core::hint::spin_loop();
        }
        q.last_used_idx = q.last_used_idx.wrapping_add(1);
        fence(Ordering::SeqCst);
        let t = rsp_type_v.read_volatile();
        t == VIRTIO_GPU_RESP_OK_NODATA || t == VIRTIO_GPU_RESP_OK_DISPLAY_INFO
    }
}
