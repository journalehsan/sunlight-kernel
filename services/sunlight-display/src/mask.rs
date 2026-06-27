/// Maximum supported corner radius.
pub const MAX_RADIUS: usize = 16;

/// Precomputed anti-aliased corner coverage mask for a single radius.
///
/// Stores a `radius × radius` quadrant table. Every corner of a window looks
/// up its pixel's coverage from the same table (mirrored by the `coverage`
/// helper). Interior pixels are fast-pathed to 255 without a table lookup.
pub struct CornerMask {
    pub radius: u32,
    /// `data[qy][qx]` — coverage 0..=255 for the corner quadrant pixel at
    /// distance `(qx, qy)` from the corner, counting inward (qx=0, qy=0 is
    /// the outermost corner pixel).
    data: [[u8; MAX_RADIUS]; MAX_RADIUS],
}

impl CornerMask {
    pub fn new(radius: u32) -> Self {
        let mut data = [[0u8; MAX_RADIUS]; MAX_RADIUS];
        let r = radius as i32;
        // The quarter-circle is centred at (r, r) in corner-local coords.
        // Pixel (qx, qy) with qx ∈ [0,r), qy ∈ [0,r) has local position
        // (qx, qy) where the corner pixel is (0,0).
        // Distance from centre: sqrt((r - qx)^2 + (r - qy)^2).
        for qy in 0..r.min(MAX_RADIUS as i32) {
            for qx in 0..r.min(MAX_RADIUS as i32) {
                let ddx = (r - qx) as u32;
                let ddy = (r - qy) as u32;
                // dist in 1/256 pixel units to avoid float
                let dist_256 = isqrt256(ddx * ddx * 65536 + ddy * ddy * 65536);
                let r_outer = (r as u32) * 256;
                let r_inner = r_outer.saturating_sub(256); // one-pixel AA band
                data[qy as usize][qx as usize] = if dist_256 <= r_inner {
                    255
                } else if dist_256 >= r_outer {
                    0
                } else {
                    // Linear ramp across the 1-pixel transition band
                    ((r_outer - dist_256) & 0xFF) as u8
                };
            }
        }
        Self { radius, data }
    }

    /// Coverage (0=transparent, 255=opaque) for a pixel at rect-local
    /// position `(lx, ly)` inside a rect of size `(w, h)`.
    ///
    /// Returns 255 immediately for interior pixels (fast path).
    #[inline]
    pub fn coverage(&self, lx: i32, ly: i32, w: i32, h: i32) -> u8 {
        let r = self.radius as i32;
        let in_cx = lx < r || lx >= w - r;
        let in_cy = ly < r || ly >= h - r;
        if !in_cx || !in_cy {
            return 255; // interior — fast path
        }
        // Map to corner quadrant [0, r)
        let qx = if lx < r { lx } else { w - 1 - lx };
        let qy = if ly < r { ly } else { h - 1 - ly };
        if qx < 0 || qy < 0 || qx >= r || qy >= r {
            return 0;
        }
        self.data[qy as usize][qx as usize]
    }
}

/// Integer square root of n, returning floor(sqrt(n)).
/// Used only during mask generation (not in the compositing hot path).
fn isqrt256(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
