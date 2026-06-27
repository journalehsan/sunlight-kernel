/// Display backend — selects between the Limine physical framebuffer and VirtIO GPU.
///
/// The `back_buffer: Vec<u32>` in CompositorState is always the canonical pixel store.
/// With `Limine`, we memcpy it to the mapped FB on present.
/// With `VirtioGpu`, the back_buffer pages are pinned as the scanout resource backing;
/// present calls `gpu_flush()` (kernel issues TRANSFER_TO_HOST_2D + RESOURCE_FLUSH).
pub enum DisplayBackend {
    /// Limine physical framebuffer mapped via syscall 118. Fallback when no VirtIO GPU.
    Limine {
        fb: *mut u32,
        pitch_words: usize,
    },
    /// VirtIO GPU scanout driven via kernel proxy syscalls (119-124).
    VirtioGpu {
        width:  u32,
        height: u32,
    },
}

// SAFETY: The raw pointer is only ever used by the single-threaded compositor.
unsafe impl Send for DisplayBackend {}
