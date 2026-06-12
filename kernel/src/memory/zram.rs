// ZRAM fixed-pool virtual memory compression driver.
// Provides a pre-allocated frame pool for compressed page storage without
// dynamic heap allocation during compression/decompression.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const ZRAM_PAGE_SIZE: usize = 4096;
pub const ZRAM_MAX_PAGES: usize = 128; // 128-slot metadata array
pub const ZRAM_POOL_FRAMES: usize = 128; // 128 frames = 512 KiB physical pool
pub const ZRAM_POOL_SIZE: usize = ZRAM_POOL_FRAMES * ZRAM_PAGE_SIZE;

#[derive(Clone, Copy, Debug)]
pub enum ZramError {
    PageNotFound,
    CompressionFailed,
    DecompressionFailed,
    PoolExhausted,
    NotInitialized,
    InvalidIndex,
}

#[derive(Clone, Copy)]
struct ZramPageMetadata {
    page_index: Option<usize>,
    compressed_size: usize,
    offset: usize,
}

impl ZramPageMetadata {
    const fn empty() -> Self {
        Self {
            page_index: None,
            compressed_size: 0,
            offset: 0,
        }
    }
}

static ZRAM_INITIALIZED: AtomicBool = AtomicBool::new(false);
static ZRAM_NEXT_SLOT: AtomicUsize = AtomicUsize::new(0);

static mut ZRAM_POOL: [u8; ZRAM_POOL_SIZE] = [0; ZRAM_POOL_SIZE];
static mut ZRAM_METADATA: [ZramPageMetadata; ZRAM_MAX_PAGES] = [ZramPageMetadata::empty(); ZRAM_MAX_PAGES];

/// Initialize the ZRAM fixed-pool module.
/// Must be called once during kernel boot after PMM initialization.
pub fn init() {
    unsafe {
        ZRAM_POOL.fill(0);
        for i in 0..ZRAM_MAX_PAGES {
            ZRAM_METADATA[i] = ZramPageMetadata::empty();
        }
    }
    ZRAM_NEXT_SLOT.store(0, Ordering::SeqCst);
    ZRAM_INITIALIZED.store(true, Ordering::SeqCst);
}

/// Compress and store a 4096-byte page into the ZRAM pool.
/// Returns the compressed size on success.
pub fn write_page(page_index: usize, raw_data: &[u8; ZRAM_PAGE_SIZE]) -> Result<usize, ZramError> {
    if !ZRAM_INITIALIZED.load(Ordering::SeqCst) {
        return Err(ZramError::NotInitialized);
    }

    // Compress the page using lz4_flex.
    let compressed = match lz4_flex::compress(raw_data) {
        compressed => compressed,
    };

    if compressed.is_empty() {
        return Err(ZramError::CompressionFailed);
    }

    // Determine which slot to use (round-robin with wrap-around).
    let slot_idx = {
        let mut next = ZRAM_NEXT_SLOT.load(Ordering::SeqCst);
        let slot = next;
        next = (next + 1) % ZRAM_MAX_PAGES;
        ZRAM_NEXT_SLOT.store(next, Ordering::SeqCst);
        slot
    };

    // Calculate slot position in the pool.
    let slot_start = slot_idx * ZRAM_PAGE_SIZE;

    // Ensure compressed data fits in the slot.
    if compressed.len() > ZRAM_PAGE_SIZE {
        return Err(ZramError::PoolExhausted);
    }

    unsafe {
        // Store compressed data in the slot.
        let pool_slice = &mut ZRAM_POOL[slot_start..slot_start + compressed.len()];
        pool_slice.copy_from_slice(&compressed);

        // Update metadata.
        ZRAM_METADATA[slot_idx] = ZramPageMetadata {
            page_index: Some(page_index),
            compressed_size: compressed.len(),
            offset: slot_start,
        };
    }

    Ok(compressed.len())
}

/// Decompress and retrieve a page from the ZRAM pool.
pub fn read_page(page_index: usize, out_buffer: &mut [u8; ZRAM_PAGE_SIZE]) -> Result<(), ZramError> {
    if !ZRAM_INITIALIZED.load(Ordering::SeqCst) {
        return Err(ZramError::NotInitialized);
    }

    // Search for the page_index in metadata.
    let metadata = unsafe {
        let mut found = None;
        for i in 0..ZRAM_MAX_PAGES {
            if let Some(idx) = ZRAM_METADATA[i].page_index {
                if idx == page_index {
                    found = Some(ZRAM_METADATA[i]);
                    break;
                }
            }
        }
        found
    };

    let metadata = metadata.ok_or(ZramError::PageNotFound)?;

    // Decompress from the pool.
    let compressed_data = unsafe {
        let offset = metadata.offset;
        let size = metadata.compressed_size;
        if offset + size > ZRAM_POOL_SIZE {
            return Err(ZramError::PageNotFound);
        }
        &ZRAM_POOL[offset..offset + size]
    };

    match lz4_flex::decompress(compressed_data, ZRAM_PAGE_SIZE) {
        Ok(decompressed) => {
            if decompressed.len() != ZRAM_PAGE_SIZE {
                return Err(ZramError::DecompressionFailed);
            }
            out_buffer.copy_from_slice(&decompressed);
            Ok(())
        }
        Err(_) => Err(ZramError::DecompressionFailed),
    }
}

/// Query ZRAM statistics.
pub fn stats() -> (usize, usize) {
    if !ZRAM_INITIALIZED.load(Ordering::SeqCst) {
        return (0, 0);
    }

    let mut total_compressed = 0usize;
    let mut pages_stored = 0usize;

    unsafe {
        for i in 0..ZRAM_MAX_PAGES {
            if ZRAM_METADATA[i].page_index.is_some() {
                total_compressed += ZRAM_METADATA[i].compressed_size;
                pages_stored += 1;
            }
        }
    }

    (total_compressed, pages_stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_and_read() {
        init();

        let mut page_data = [0u8; ZRAM_PAGE_SIZE];
        for i in 0..ZRAM_PAGE_SIZE {
            page_data[i] = (i as u8).wrapping_mul(7);
        }

        let compressed_size = write_page(1, &page_data).expect("write failed");
        assert!(compressed_size > 0);

        let mut out_buffer = [0u8; ZRAM_PAGE_SIZE];
        read_page(1, &mut out_buffer).expect("read failed");

        assert_eq!(page_data, out_buffer);
    }

    #[test]
    fn test_multiple_pages() {
        init();

        for page_idx in 0..16 {
            let mut page_data = [0u8; ZRAM_PAGE_SIZE];
            for i in 0..ZRAM_PAGE_SIZE {
                page_data[i] = ((i ^ page_idx) as u8).wrapping_mul(13);
            }

            write_page(page_idx, &page_data).expect("write failed");

            let mut out_buffer = [0u8; ZRAM_PAGE_SIZE];
            read_page(page_idx, &mut out_buffer).expect("read failed");

            assert_eq!(page_data, out_buffer);
        }
    }

    #[test]
    fn test_page_not_found() {
        init();

        let mut out_buffer = [0u8; ZRAM_PAGE_SIZE];
        let result = read_page(999, &mut out_buffer);

        assert!(matches!(result, Err(ZramError::PageNotFound)));
    }

    #[test]
    fn test_stats() {
        init();

        let mut page_data = [0u8; ZRAM_PAGE_SIZE];
        for i in 0..10 {
            page_data[i] = i as u8;
        }

        write_page(5, &page_data).expect("write failed");

        let (total, pages) = stats();
        assert_eq!(pages, 1);
        assert!(total > 0);
    }
}
