//! Owned screen-sized pixels, outside the shell's small general-purpose heap.
//! The worker recycles the retired mapping on the next wallpaper preparation.

#[cfg(not(target_os = "none"))]
extern crate alloc;

pub(crate) struct Wallpaper {
    pub width: u32,
    pub height: u32,
    #[cfg(target_os = "none")]
    ptr: core::ptr::NonNull<u32>,
    #[cfg(not(target_os = "none"))]
    storage: alloc::vec::Vec<u32>,
    len: usize,
}

impl Wallpaper {
    fn new(width: u32, height: u32) -> Option<Self> {
        let len = (width as usize).checked_mul(height as usize)?;
        let bytes = len.checked_mul(core::mem::size_of::<u32>())?;
        if width == 0 || height == 0 || bytes > isize::MAX as usize {
            return None;
        }
        #[cfg(target_os = "none")]
        let ptr = {
            use sunlight_libc::mman::*;
            // Private anonymous mappings are zero initialized and page aligned.
            let address = mmap(
                core::ptr::null_mut(),
                bytes,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .ok()?;
            core::ptr::NonNull::new(address.cast::<u32>())?
        };
        #[cfg(not(target_os = "none"))]
        let storage = {
            let mut storage = alloc::vec::Vec::new();
            storage.try_reserve_exact(len).ok()?;
            storage.resize(len, 0);
            storage
        };
        Some(Self {
            width,
            height,
            len,
            #[cfg(target_os = "none")]
            ptr,
            #[cfg(not(target_os = "none"))]
            storage,
        })
    }

    pub fn pixels(&self) -> &[u32] {
        #[cfg(target_os = "none")]
        // The mapping is owned by self and remains live until Drop.
        unsafe {
            core::slice::from_raw_parts(self.ptr.as_ptr(), self.len)
        }
        #[cfg(not(target_os = "none"))]
        &self.storage[..self.len]
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        #[cfg(target_os = "none")]
        // Exclusive ownership of the retired/new buffer is required to fill it.
        unsafe {
            core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len)
        }
        #[cfg(not(target_os = "none"))]
        &mut self.storage[..self.len]
    }

    /// Preserve the existing nearest-neighbor cover/stretch and opaque alpha.
    /// A retired buffer is reusable only after the UI has transferred ownership.
    pub fn prepare(
        source: &[u32],
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
        spare: Option<Self>,
    ) -> Option<Self> {
        let source_len = (source_width as usize).checked_mul(source_height as usize)?;
        if source_width == 0 || source_height == 0 || source.len() < source_len {
            return None;
        }
        let mut output = match spare {
            Some(buffer) if buffer.width == width && buffer.height == height => buffer,
            other => {
                // Drop an obsolete-size spare before allocating its replacement.
                drop(other);
                Self::new(width, height)?
            }
        };
        for (y, row) in output
            .pixels_mut()
            .chunks_exact_mut(width as usize)
            .enumerate()
        {
            let sy = y * source_height as usize / height as usize;
            for (x, pixel) in row.iter_mut().enumerate() {
                let sx = x * source_width as usize / width as usize;
                *pixel = source[sy * source_width as usize + sx] | 0xff00_0000;
            }
        }
        Some(output)
    }
}

#[cfg(target_os = "none")]
impl Drop for Wallpaper {
    fn drop(&mut self) {
        // Normal retirement and resize cleanup happen on the worker that mapped
        // the buffer. Never wrap mmap memory in a Vec (wrong deallocator).
        if sunlight_libc::mman::munmap(self.ptr.as_ptr().cast(), self.len * 4).is_err() {
            sunlight_ipc::debug_log("[VORTEX] wallpaper unmap failed\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_resolution_prepares_opaque_pixels() {
        let image = Wallpaper::prepare(
            &[0x123456, 0xabcdef, 0xff0000, 0x00ff00],
            2,
            2,
            1898,
            1157,
            None,
        )
        .unwrap();
        assert_eq!(image.pixels().len() * 4, 8_783_944);
        for (i, pixel) in image.pixels().iter().enumerate() {
            let x = i % 1898;
            let y = i / 1898;
            let expected =
                [0x123456, 0xabcdef, 0xff0000, 0x00ff00][(y * 2 / 1157) * 2 + x * 2 / 1898];
            assert_eq!(*pixel, expected | 0xff000000);
        }
    }

    #[test]
    fn repeated_swaps_reuse_two_buffers_and_keep_visible_pixels_unchanged() {
        let mut visible = Wallpaper::prepare(&[1], 1, 1, 1898, 1157, None).unwrap();
        let mut spare = None;
        let mut addresses = alloc::vec![visible.pixels().as_ptr()];
        for value in 2..22 {
            let next = Wallpaper::prepare(&[value], 1, 1, 1898, 1157, spare.take()).unwrap();
            assert!(visible
                .pixels()
                .iter()
                .all(|p| *p == 0xff000000 | (value - 1)));
            let address = next.pixels().as_ptr();
            if !addresses.contains(&address) {
                addresses.push(address);
            }
            spare = Some(core::mem::replace(&mut visible, next));
        }
        assert_eq!(addresses.len(), 2);
        assert!(visible.pixels().iter().all(|p| *p == 0xff000015));
    }

    #[test]
    fn resize_and_invalid_sizes_are_checked() {
        let old = Wallpaper::prepare(&[1], 1, 1, 8, 8, None).unwrap();
        let resized = Wallpaper::prepare(&[2], 1, 1, 17, 9, Some(old)).unwrap();
        assert_eq!(resized.pixels().len(), 153);
        assert!(Wallpaper::prepare(&[], 1, 1, 8, 8, None).is_none());
        assert!(Wallpaper::prepare(&[1], 1, 1, 0, 8, None).is_none());
        assert!(Wallpaper::prepare(&[1], 1, 1, u32::MAX, u32::MAX, None).is_none());
    }
}
