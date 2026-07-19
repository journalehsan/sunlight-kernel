/// Display backend — selects among Limine framebuffer, VirtIO GPU, and VMware SVGA II.
///
/// The `back_buffer: Vec<u32>` in CompositorState is always the canonical pixel store.
/// - `Limine`: memcpy to the mapped FB on present.
/// - `VirtioGpu`: back_buffer pages are pinned as scanout resource backing;
///   present calls `gpu_flush()` (TRANSFER_TO_HOST_2D + RESOURCE_FLUSH).
/// - `VmwareSvga`: memcpy to the mapped boot/SVGA FB (same path as Limine when the
///   boot FB lies in VRAM), then `svga_update()` issues `SVGA_CMD_UPDATE`.
#[derive(Clone, Copy)]
pub enum DisplayBackend {
    /// Limine physical framebuffer mapped via syscall 118. Final fallback.
    Limine { fb: *mut u32, pitch_words: usize },
    /// VirtIO GPU scanout driven via kernel proxy syscalls (119-124).
    VirtioGpu { width: u32, height: u32 },
    /// VMware SVGA II legacy framebuffer + FIFO UPDATE (syscalls 127-128).
    VmwareSvga { fb: *mut u32, pitch_words: usize, width: u32, height: u32 },
}

// SAFETY: The raw pointer is only ever used by the single-threaded compositor.
unsafe impl Send for DisplayBackend {}
