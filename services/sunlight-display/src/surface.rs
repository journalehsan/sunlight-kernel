//! Checked layout and readable-range validation for XRGB8888 SHM surfaces.

pub const BYTES_PER_PIXEL: u32 = 4;
pub const MAX_SURFACE_DIM: u32 = 8192;
pub const MAX_SURFACE_BYTES: usize =
    MAX_SURFACE_DIM as usize * MAX_SURFACE_DIM as usize * BYTES_PER_PIXEL as usize;
pub const SHM_PAGE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceLayoutError {
    ZeroDimension,
    DimensionTooLarge,
    ArithmeticOverflow,
    StrideTooSmall,
    StrideNotPixelAligned,
    AllocationTooSmall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadableRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceLayout {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    pub stride_pixels: usize,
    pub min_row_bytes: usize,
    pub required_bytes: usize,
    pub surface_len_bytes: usize,
}

impl SurfaceLayout {
    /// Validate one attached XRGB8888 allocation, including its complete row span.
    pub fn validate(
        width: u32,
        height: u32,
        stride_bytes: usize,
        surface_len_bytes: usize,
    ) -> Result<Self, SurfaceLayoutError> {
        if width == 0 || height == 0 {
            return Err(SurfaceLayoutError::ZeroDimension);
        }
        if width > MAX_SURFACE_DIM || height > MAX_SURFACE_DIM {
            return Err(SurfaceLayoutError::DimensionTooLarge);
        }
        let min_row_bytes = (width as usize)
            .checked_mul(BYTES_PER_PIXEL as usize)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        if stride_bytes < min_row_bytes {
            return Err(SurfaceLayoutError::StrideTooSmall);
        }
        if stride_bytes % BYTES_PER_PIXEL as usize != 0 {
            return Err(SurfaceLayoutError::StrideNotPixelAligned);
        }
        let required_bytes = stride_bytes
            .checked_mul(height as usize)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        if required_bytes > MAX_SURFACE_BYTES || surface_len_bytes < required_bytes {
            return Err(SurfaceLayoutError::AllocationTooSmall);
        }
        let stride_pixels = stride_bytes
            .checked_div(BYTES_PER_PIXEL as usize)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        Ok(Self {
            width,
            height,
            stride_bytes,
            stride_pixels,
            min_row_bytes,
            required_bytes,
            surface_len_bytes,
        })
    }

    /// Compute the allocation used by CREATE_WINDOW after validating dimensions.
    pub fn for_new_surface(width: u32, height: u32) -> Result<Self, SurfaceLayoutError> {
        if width == 0 || height == 0 {
            return Err(SurfaceLayoutError::ZeroDimension);
        }
        if width > MAX_SURFACE_DIM || height > MAX_SURFACE_DIM {
            return Err(SurfaceLayoutError::DimensionTooLarge);
        }
        let min_row_bytes = (width as usize)
            .checked_mul(BYTES_PER_PIXEL as usize)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        let required_bytes = min_row_bytes
            .checked_mul(height as usize)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        let allocation_len = required_bytes.max(SHM_PAGE_BYTES);
        Self::validate(width, height, min_row_bytes, allocation_len)
    }

    /// Clip a source rectangle and prove every row in it is readable.
    pub fn readable_rect(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<ReadableRect, SurfaceLayoutError> {
        if x >= self.width || y >= self.height || width == 0 || height == 0 {
            return Ok(ReadableRect {
                x,
                y,
                width: 0,
                height: 0,
            });
        }
        let end_x = x
            .checked_add(width)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        let end_y = y
            .checked_add(height)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        let rect = ReadableRect {
            x,
            y,
            width: end_x.min(self.width) - x,
            height: end_y.min(self.height) - y,
        };
        let row_start = (rect.y as usize)
            .checked_mul(self.stride_bytes)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        let x_bytes = (rect.x as usize)
            .checked_mul(BYTES_PER_PIXEL as usize)
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        let row_end = (rect.width as usize)
            .checked_mul(BYTES_PER_PIXEL as usize)
            .and_then(|n| x_bytes.checked_add(n))
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        let last_row = (rect.height as usize - 1)
            .checked_mul(self.stride_bytes)
            .and_then(|n| row_start.checked_add(n))
            .and_then(|n| n.checked_add(row_end))
            .ok_or(SurfaceLayoutError::ArithmeticOverflow)?;
        if last_row > self.surface_len_bytes {
            return Err(SurfaceLayoutError::AllocationTooSmall);
        }
        Ok(rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dimensions_stride_and_allocation() {
        let created = SurfaceLayout::for_new_surface(1, 1).unwrap();
        assert_eq!(created.surface_len_bytes, SHM_PAGE_BYTES);
        let exact = SurfaceLayout::validate(2, 3, 8, 24).unwrap();
        assert_eq!(exact.required_bytes, 24);
        assert_eq!(
            SurfaceLayout::validate(1, 1, 4, 4).unwrap().min_row_bytes,
            4
        );
        assert_eq!(
            SurfaceLayout::validate(1, 1, 8, 8).unwrap().stride_pixels,
            2
        );
        assert_eq!(
            SurfaceLayout::validate(1, 1, 3, 3),
            Err(SurfaceLayoutError::StrideTooSmall)
        );
        assert_eq!(
            SurfaceLayout::validate(1, 1, 5, 5),
            Err(SurfaceLayoutError::StrideNotPixelAligned)
        );
        assert_eq!(
            SurfaceLayout::validate(2, 3, 8, 23),
            Err(SurfaceLayoutError::AllocationTooSmall)
        );
        assert_eq!(
            SurfaceLayout::for_new_surface(0, 1),
            Err(SurfaceLayoutError::ZeroDimension)
        );
        assert_eq!(
            SurfaceLayout::for_new_surface(MAX_SURFACE_DIM + 1, 1),
            Err(SurfaceLayoutError::DimensionTooLarge)
        );
    }

    #[test]
    fn rejects_zero_oversized_and_checked_overflow_inputs() {
        assert_eq!(
            SurfaceLayout::validate(0, 1, 0, 0),
            Err(SurfaceLayoutError::ZeroDimension)
        );
        assert_eq!(
            SurfaceLayout::validate(1, 0, 0, 0),
            Err(SurfaceLayoutError::ZeroDimension)
        );
        assert_eq!(
            SurfaceLayout::validate(
                MAX_SURFACE_DIM,
                MAX_SURFACE_DIM,
                MAX_SURFACE_BYTES / MAX_SURFACE_DIM as usize,
                MAX_SURFACE_BYTES
            ),
            Ok(SurfaceLayout::validate(
                MAX_SURFACE_DIM,
                MAX_SURFACE_DIM,
                MAX_SURFACE_BYTES / MAX_SURFACE_DIM as usize,
                MAX_SURFACE_BYTES
            )
            .unwrap())
        );
        assert_eq!(
            SurfaceLayout::validate(MAX_SURFACE_DIM + 1, 1, 0, 0),
            Err(SurfaceLayoutError::DimensionTooLarge)
        );
        assert_eq!(
            SurfaceLayout::validate(1, MAX_SURFACE_DIM + 1, 0, 0),
            Err(SurfaceLayoutError::DimensionTooLarge)
        );
        assert_eq!(
            SurfaceLayout::validate(u32::MAX, u32::MAX, usize::MAX, usize::MAX),
            Err(SurfaceLayoutError::DimensionTooLarge)
        );
        let layout = SurfaceLayout::validate(4, 4, 16, 64).unwrap();
        assert_eq!(layout.readable_rect(u32::MAX, 0, 1, 1).unwrap().width, 0);
        assert_eq!(layout.readable_rect(0, 0, u32::MAX, 1).unwrap().width, 4);
    }

    #[test]
    fn clips_edges_and_padded_final_row_without_exceeding_allocation() {
        let layout = SurfaceLayout::validate(3, 3, 16, 48).unwrap();
        for rect in [
            (0, 0, 1, 1),
            (2, 0, 1, 1),
            (0, 2, 1, 1),
            (2, 2, 1, 1),
            (0, 0, 3, 3),
            (0, 0, 4, 3),
            (0, 0, 3, 4),
        ] {
            let got = layout
                .readable_rect(rect.0, rect.1, rect.2, rect.3)
                .unwrap();
            assert!(
                got.width == 0
                    || (got.y as usize + got.height as usize - 1) * layout.stride_bytes
                        + (got.x as usize + got.width as usize) * 4
                        <= layout.surface_len_bytes
            );
        }
        assert_eq!(layout.readable_rect(3, 0, 1, 1).unwrap().width, 0);
    }
}
