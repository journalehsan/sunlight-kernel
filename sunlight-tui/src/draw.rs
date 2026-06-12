//! Draw primitives — no floats, fixed-point Q10 arithmetic

#![allow(dead_code)]

use crate::framebuffer::Framebuffer;

/// Fixed-point sin/cos tables for 0..=359 degrees, scaled by 1024 (Q10 format)
/// Generated with: sin(deg * PI / 180) * 1024
static SIN_TABLE: [i32; 360] = [
    0, 17, 35, 53, 71, 89, 107, 124, 142, 160, 177, 195, 212, 230, 247, 265, 282, 299, 316, 333,
    350, 366, 383, 400, 416, 432, 448, 464, 480, 496, 511, 527, 542, 557, 572, 587, 601, 616, 630,
    644, 658, 671, 685, 698, 711, 724, 736, 748, 760, 772, 784, 795, 806, 817, 828, 838, 848, 858,
    868, 877, 886, 895, 904, 912, 920, 928, 935, 942, 949, 955, 962, 968, 973, 979, 984, 989, 993,
    997, 1001, 1005, 1008, 1011, 1014, 1016, 1018, 1020, 1021, 1022, 1023, 1023, 1024, 1023, 1023,
    1022, 1021, 1020, 1018, 1016, 1014, 1011, 1008, 1005, 1001, 997, 993, 989, 984, 979, 973, 968,
    962, 955, 949, 942, 935, 928, 920, 912, 904, 895, 886, 877, 868, 858, 848, 838, 828, 817, 806,
    795, 784, 772, 760, 748, 736, 724, 711, 698, 685, 671, 658, 644, 630, 616, 601, 587, 572, 557,
    542, 527, 511, 496, 480, 464, 448, 432, 416, 400, 383, 366, 350, 333, 316, 299, 282, 265, 247,
    230, 212, 195, 177, 160, 142, 124, 107, 89, 71, 53, 35, 17, 0, -17, -35, -53, -71, -89, -107,
    -124, -142, -160, -177, -195, -212, -230, -247, -265, -282, -299, -316, -333, -350, -366, -383,
    -400, -416, -432, -448, -464, -480, -496, -512, -527, -542, -557, -572, -587, -601, -616, -630,
    -644, -658, -671, -685, -698, -711, -724, -736, -748, -760, -772, -784, -795, -806, -817, -828,
    -838, -848, -858, -868, -877, -886, -895, -904, -912, -920, -928, -935, -942, -949, -955, -962,
    -968, -973, -979, -984, -989, -993, -997, -1001, -1005, -1008, -1011, -1014, -1016, -1018,
    -1020, -1021, -1022, -1023, -1023, -1024, -1023, -1023, -1022, -1021, -1020, -1018, -1016,
    -1014, -1011, -1008, -1005, -1001, -997, -993, -989, -984, -979, -973, -968, -962, -955, -949,
    -942, -935, -928, -920, -912, -904, -895, -886, -877, -868, -858, -848, -838, -828, -817, -806,
    -795, -784, -772, -760, -748, -736, -724, -711, -698, -685, -671, -658, -644, -630, -616, -601,
    -587, -572, -557, -542, -527, -512, -496, -480, -464, -448, -432, -416, -400, -383, -366, -350,
    -333, -316, -299, -282, -265, -247, -230, -212, -195, -177, -160, -142, -124, -107, -89, -71,
    -53, -35, -17,
];

static COS_TABLE: [i32; 360] = [
    1024, 1023, 1023, 1022, 1021, 1020, 1018, 1016, 1014, 1011, 1008, 1005, 1001, 997, 993, 989,
    984, 979, 973, 968, 962, 955, 949, 942, 935, 928, 920, 912, 904, 895, 886, 877, 868, 858, 848,
    838, 828, 817, 806, 795, 784, 772, 760, 748, 736, 724, 711, 698, 685, 671, 658, 644, 630, 616,
    601, 587, 572, 557, 542, 527, 512, 496, 480, 464, 448, 432, 416, 400, 383, 366, 350, 333, 316,
    299, 282, 265, 247, 230, 212, 195, 177, 160, 142, 124, 107, 89, 71, 53, 35, 17, 0, -17, -35,
    -53, -71, -89, -107, -124, -142, -160, -177, -195, -212, -230, -247, -265, -282, -299, -316,
    -333, -350, -366, -383, -400, -416, -432, -448, -464, -480, -496, -511, -527, -542, -557, -572,
    -587, -601, -616, -630, -644, -658, -671, -685, -698, -711, -724, -736, -748, -760, -772, -784,
    -795, -806, -817, -828, -838, -848, -858, -868, -877, -886, -895, -904, -912, -920, -928, -935,
    -942, -949, -955, -962, -968, -973, -979, -984, -989, -993, -997, -1001, -1005, -1008, -1011,
    -1014, -1016, -1018, -1020, -1021, -1022, -1023, -1023, -1024, -1023, -1023, -1022, -1021,
    -1020, -1018, -1016, -1014, -1011, -1008, -1005, -1001, -997, -993, -989, -984, -979, -973,
    -968, -962, -955, -949, -942, -935, -928, -920, -912, -904, -895, -886, -877, -868, -858, -848,
    -838, -828, -817, -806, -795, -784, -772, -760, -748, -736, -724, -711, -698, -685, -671, -658,
    -644, -630, -616, -601, -587, -572, -557, -542, -527, -512, -496, -480, -464, -448, -432, -416,
    -400, -383, -366, -350, -333, -316, -299, -282, -265, -247, -230, -212, -195, -177, -160, -142,
    -124, -107, -89, -71, -53, -35, -17, 0, 17, 35, 53, 71, 89, 107, 124, 142, 160, 177, 195, 212,
    230, 247, 265, 282, 299, 316, 333, 350, 366, 383, 400, 416, 432, 448, 464, 480, 496, 511, 527,
    542, 557, 572, 587, 601, 616, 630, 644, 658, 671, 685, 698, 711, 724, 736, 748, 760, 772, 784,
    795, 806, 817, 828, 838, 848, 858, 868, 877, 886, 895, 904, 912, 920, 928, 935, 942, 949, 955,
    962, 968, 973, 979, 984, 989, 993, 997, 1001, 1005, 1008, 1011, 1014, 1016, 1018, 1020, 1021,
    1022, 1023, 1023,
];

#[inline]
fn fixed_sin(deg: u32) -> i32 {
    SIN_TABLE[(deg % 360) as usize]
}

#[inline]
fn fixed_cos(deg: u32) -> i32 {
    COS_TABLE[(deg % 360) as usize]
}

/// Filled rectangle
pub fn rect(fb: &mut Framebuffer, x: u32, y: u32, w: u32, h: u32, color: u32) {
    fb.fill_rect(x, y, w, h, color);
}

/// Rectangle outline only
pub fn rect_outline(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    thickness: u32,
    color: u32,
) {
    // Top
    fb.fill_rect(x, y, w, thickness, color);
    // Bottom
    if y + h >= thickness {
        fb.fill_rect(x, y + h - thickness, w, thickness, color);
    }
    // Left
    fb.fill_rect(x, y, thickness, h, color);
    // Right
    if x + w >= thickness {
        fb.fill_rect(x + w - thickness, y, thickness, h, color);
    }
}

/// Progress bar — track + fill, no floats
/// progress: 0..=1000 (permille, avoids float)
pub fn progress_bar(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    progress_permille: u32,
    track_color: u32,
    fill_color: u32,
) {
    // Draw track
    fb.fill_rect(x, y, w, h, track_color);

    // Draw fill
    let fill_width = (w * progress_permille.min(1000)) / 1000;
    if fill_width > 0 {
        fb.fill_rect(x, y, fill_width, h, fill_color);
    }
}

/// Spinner: draws a static arc frame
/// angle_step: 0..=359 — current rotation angle in degrees (integer)
/// sweep: arc sweep in degrees (e.g., 108)
pub fn spinner_frame(
    fb: &mut Framebuffer,
    cx: u32,
    cy: u32,
    radius: u32,
    thickness: u32,
    angle_step: u32,
    sweep_deg: u32,
    color: u32,
) {
    let inner_radius = if radius > thickness {
        radius - thickness
    } else {
        0
    };

    for deg in 0..sweep_deg {
        let angle = (angle_step + deg) % 360;
        let cos = fixed_cos(angle);
        let sin = fixed_sin(angle);

        // Draw thicker arc by filling between inner and outer radius
        for r in inner_radius..=radius {
            let px = (cx as i32 + (cos * r as i32) / 1024) as u32;
            let py = (cy as i32 + (sin * r as i32) / 1024) as u32;
            fb.put_pixel(px, py, color);
        }
    }
}

/// Simple sun logo — geometric, drawn procedurally (no bitmap needed)
/// cx, cy = center; size = radius of center circle
pub fn draw_sun_logo(fb: &mut Framebuffer, cx: u32, cy: u32, size: u32, color: u32) {
    // Center circle
    for dy in -(size as i32)..=(size as i32) {
        for dx in -(size as i32)..=(size as i32) {
            if dx * dx + dy * dy <= (size * size) as i32 {
                let px = (cx as i32 + dx) as u32;
                let py = (cy as i32 + dy) as u32;
                fb.put_pixel(px, py, color);
            }
        }
    }

    // Rays (8 directions)
    let ray_length = size * 2;
    let ray_thickness = size / 3;

    for angle in (0..360).step_by(45) {
        let cos = fixed_cos(angle);
        let sin = fixed_sin(angle);

        for dist in size..(size + ray_length) {
            for thick in 0..ray_thickness {
                let offset_x = (thick as i32 - ray_thickness as i32 / 2) * sin / 1024;
                let offset_y = (thick as i32 - ray_thickness as i32 / 2) * (-cos) / 1024;

                let px = cx as i32 + (cos * dist as i32) / 1024 + offset_x;
                let py = cy as i32 + (sin * dist as i32) / 1024 + offset_y;

                if px >= 0 && py >= 0 {
                    fb.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}
