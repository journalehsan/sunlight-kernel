//! Allocation-free scenery. Time animates weather only; geometry is seed-stable.
use sunlight_ui::{Canvas, Color, Rect};

const INK: Color = Color::rgb(10, 10, 12);
const BONE: Color = Color::rgb(237, 230, 216);
const AMBER: Color = Color::rgb(255, 152, 0);
fn bone(alpha: u8) -> Color {
    Color::rgba(237, 230, 216, alpha)
}
fn amber(alpha: u8) -> Color {
    Color::rgba(255, 152, 0, alpha)
}

// Canvas line primitives replace pixels; these strokes composite instead.
fn line(c: &mut Canvas, a: (i32, i32), b: (i32, i32), color: Color) {
    let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
    for i in 0..=steps {
        c.blend_pixel(
            a.0 + (b.0 - a.0) * i / steps,
            a.1 + (b.1 - a.1) * i / steps,
            color,
        );
    }
}

pub fn outdoors(scene: &str) -> bool {
    matches!(
        scene,
        "street" | "transit" | "c2-route" | "c2-exterior" | "c2-displacement"
    )
}

/// Layered skyline, window lights and broken reflections, clipped to the viewport.
pub fn city(canvas: &mut Canvas, rect: Rect, seed: u32, time: u64) {
    let mut c = canvas.sub_canvas(rect);
    let w = c.width as i32;
    let h = c.height as i32;
    c.fill_rect(Rect::new(0, 0, w as u32, h as u32), INK);
    let horizon = h * 72 / 100;
    for y in 0..horizon {
        c.blend_rect(
            Rect::new(0, y, w as u32, 1),
            amber((6 + y * 24 / horizon.max(1)) as u8),
        );
    }
    for layer in 0..3 {
        let step = (w / (15 - layer * 3)).max(12);
        for i in 0..(w / step + 1) {
            let hash = seed
                .wrapping_add(i as u32 * 7919 + layer as u32 * 104729)
                .wrapping_mul(2654435761);
            let bh = h / 6 + (hash % (h as u32 / 3).max(1)) as i32;
            let x = i * step - layer * 13;
            let top = horizon - bh + layer * h / 18;
            let building = Rect::new(x, top, (step - 3) as u32, (horizon - top) as u32);
            c.fill_rect(building, INK);
            c.blend_rect(building, bone((22 - layer * 6) as u8));
            line(&mut c, (x, top), (x + step - 4, top), bone(42));
            if i % 3 == 0 {
                line(
                    &mut c,
                    (x + step / 2, top),
                    (x + step / 2, top - 12),
                    bone(55),
                );
            }
            for row in 0..(bh / 12).max(1) {
                for col in 0..((step - 8) / 11).max(1) {
                    if hash.rotate_left((row * 3 + col) as u32) & 3 == 0 {
                        let wy = top + 8 + row * 12;
                        if wy < horizon - 4 {
                            c.blend_rect(
                                Rect::new(x + 5 + col * 11, wy, 3, 4),
                                amber(80 + layer as u8 * 35),
                            );
                        }
                    }
                }
            }
        }
    }
    for i in 0..28 {
        let x = (i * 83 + 29) % w.max(1);
        for row in 0..8 {
            let y = horizon + 5 + row * (h - horizon).max(8) / 8;
            let shift = ((time / 240 + i as u64 + row as u64) % 3) as i32;
            c.blend_rect(
                Rect::new(x - row * 2 + shift, y, (3 + row * 3) as u32, 1),
                amber((32 - row * 3) as u8),
            );
        }
    }
}

/// Architectural depth behind the authored objects, without moving their hitboxes.
pub fn interior(canvas: &mut Canvas, rect: Rect) {
    let mut c = canvas.sub_canvas(rect);
    let w = c.width as i32;
    let h = c.height as i32;
    let floor = h * 70 / 100;
    for y in 0..floor {
        c.blend_rect(
            Rect::new(0, y, w as u32, 1),
            bone((3 + y * 12 / floor.max(1)) as u8),
        );
    }
    line(&mut c, (12, floor), (w - 12, floor), bone(65));
    line(&mut c, (12, floor + 4), (w - 12, floor + 4), bone(25));
    for i in 0..11 {
        line(
            &mut c,
            (w / 2 + (i - 5) * w / 22, floor + 5),
            (i * w / 10, h - 12),
            bone(22),
        );
    }
    for y in [floor + 16, floor + 39, floor + 73] {
        if y < h - 12 {
            line(&mut c, (12, y), (w - 12, y), bone(18));
        }
    }
    for x in (24..w - 12).step_by(64) {
        line(&mut c, (x, 12), (x, floor - 4), bone(10));
    }
}

pub fn bedroom_props(canvas: &mut Canvas, rect: Rect, desk: Rect, window: Rect) {
    interior(canvas, rect);
    // A slanted shaft of city light crosses the wall and floor.
    let bottom = rect.bottom() - 18;
    for y in window.bottom()..bottom {
        let t = y - window.bottom();
        let left = window.x - t * 2;
        let right = (window.right() - t).min(rect.right() - 12);
        if right > left {
            canvas.blend_rect(
                Rect::new(
                    left.max(rect.x + 12),
                    y,
                    (right - left.max(rect.x + 12)).max(0) as u32,
                    1,
                ),
                amber(8),
            );
        }
    }
    // Low bed, pillow and a folded striped blanket occupy the quiet left corner.
    let bed = Rect::new(
        rect.x + 42,
        rect.y + rect.h as i32 * 64 / 100,
        rect.w * 31 / 100,
        rect.h * 22 / 100,
    );
    canvas.blend_rounded_rect(
        Rect::new(bed.x - 12, bed.bottom() - 8, bed.w + 28, 22),
        10,
        Color::rgba(10, 10, 12, 160),
    );
    canvas.fill_rect(Rect::new(bed.x - 6, bed.y - 36, 12, bed.h + 44), INK);
    canvas.blend_rect(Rect::new(bed.x - 5, bed.y - 36, 10, bed.h + 40), bone(80));
    canvas.fill_rounded_rect(bed, 5, INK);
    canvas.blend_rounded_rect(bed, 5, bone(48));
    canvas.blend_rounded_rect(
        Rect::new(bed.x + 10, bed.y + 8, bed.w / 4, 22),
        7,
        bone(130),
    );
    let blanket = Rect::new(
        bed.x + bed.w as i32 / 3,
        bed.y + 4,
        bed.w * 2 / 3 - 6,
        bed.h - 9,
    );
    canvas.blend_rect(blanket, bone(32));
    for x in (blanket.x + 8..blanket.right()).step_by(14) {
        canvas.blend_rect(Rect::new(x, blanket.y, 2, blanket.h), amber(36));
    }
    canvas.blend_rect(
        Rect::new(bed.x, bed.bottom() - 9, bed.w, 9),
        Color::rgba(10, 10, 12, 130),
    );
    // Desk cast shadow and a cable falling behind it.
    canvas.blend_rounded_rect(
        Rect::new(desk.x - 20, desk.bottom() + 28, desk.w + 40, 20),
        10,
        Color::rgba(10, 10, 12, 170),
    );
    line(
        canvas,
        (desk.right() - 65, desk.bottom()),
        (desk.right() - 50, desk.bottom() + 37),
        bone(75),
    );
    line(
        canvas,
        (desk.right() - 50, desk.bottom() + 37),
        (desk.right() - 95, desk.bottom() + 43),
        bone(75),
    );
    // Pinned sketches above the bed.
    for i in 0..3 {
        let note = Rect::new(
            rect.x + 54 + i * 47,
            rect.y + rect.h as i32 * 43 / 100 + i % 2 * 8,
            34,
            40,
        );
        canvas.blend_rect(note, bone(45));
        canvas.blend_rect(Rect::new(note.x + 12, note.y - 2, 10, 4), amber(125));
        for row in 0..3 {
            canvas.blend_rect(
                Rect::new(note.x + 6, note.y + 12 + row * 7, 22 - row as u32 * 4, 1),
                bone(65),
            );
        }
    }
}

pub fn workstation_details(canvas: &mut Canvas, crt: Rect, desk: Rect) {
    let keys = Rect::new(crt.x + 12, desk.bottom() - 19, crt.w - 24, 13);
    canvas.fill_rounded_rect(keys, 2, INK);
    for row in 0..2 {
        for col in 0..18 {
            canvas.blend_rect(
                Rect::new(
                    keys.x + 4 + col * (keys.w as i32 - 8) / 18,
                    keys.y + 3 + row * 5,
                    6,
                    3,
                ),
                bone(105),
            );
        }
    }
    let mug = Rect::new(desk.right() - 37, desk.y + 8, 19, 24);
    canvas.blend_rounded_rect(Rect::new(mug.x + 14, mug.y + 5, 12, 13), 5, bone(90));
    canvas.fill_rounded_rect(mug, 3, INK);
    canvas.blend_rounded_rect(mug, 3, bone(130));
    canvas.blend_rect(Rect::new(mug.x + 3, mug.y + 2, 13, 3), amber(85));
    for i in 0..4 {
        canvas.blend_rect(
            Rect::new(
                desk.x + desk.w as i32 / 8 + 6,
                desk.y + 16 + i * 5,
                48 - i as u32 * 7,
                1,
            ),
            Color::rgba(10, 10, 12, 135),
        );
    }
    canvas.blend_rect(Rect::new(crt.right() - 18, crt.bottom() - 6, 3, 2), AMBER);
}

/// Restrained atmosphere stays inside the illustration, never over reading text.
pub fn atmosphere(canvas: &mut Canvas, rect: Rect, rain: bool, seed: u32, time: u64) {
    let mut c = canvas.sub_canvas(rect);
    let w = c.width as i32;
    let h = c.height as i32;
    for i in 0..if rain { 64 } else { 18 } {
        let hash = seed.wrapping_add(i * 7919).wrapping_mul(2654435761);
        let x = (hash % w.max(1) as u32) as i32;
        let speed = if rain { 32 } else { 260 };
        let y = ((hash.rotate_left(13) as u64 + time / speed) % h.max(1) as u64) as i32;
        if rain {
            line(&mut c, (x, y), (x - 3, y + 10), bone(36));
        } else {
            c.blend_pixel(x, y, amber(65));
        }
    }
    // Soft edge falloff, bounded strips rather than a full-frame filter.
    for i in 0..16 {
        let shade = Color::rgba(10, 10, 12, (64 - i * 4) as u8);
        c.blend_rect(
            Rect::new(i, i, w.saturating_sub(i * 2).max(0) as u32, 1),
            shade,
        );
        c.blend_rect(
            Rect::new(i, h - 1 - i, w.saturating_sub(i * 2).max(0) as u32, 1),
            shade,
        );
        c.blend_rect(
            Rect::new(i, i, 1, h.saturating_sub(i * 2).max(0) as u32),
            shade,
        );
        c.blend_rect(
            Rect::new(w - 1 - i, i, 1, h.saturating_sub(i * 2).max(0) as u32),
            shade,
        );
    }
}

/// Large, deliberately pixel-cut wordmark for the 1993 title card.
pub fn title_mark(canvas: &mut Canvas, rect: Rect) {
    let glyphs: [[u8; 7]; 13] = [
        [31, 16, 16, 31, 1, 1, 31],
        [31, 4, 4, 4, 4, 4, 31],
        [16, 16, 16, 16, 16, 16, 31],
        [31, 4, 4, 4, 4, 4, 31],
        [31, 16, 16, 16, 16, 16, 31],
        [31, 17, 17, 17, 17, 17, 31],
        [17, 25, 25, 21, 19, 19, 17],
        [0; 7],
        [31, 16, 16, 30, 16, 16, 31],
        [31, 16, 16, 16, 16, 16, 31],
        [17, 17, 17, 31, 17, 17, 17],
        [31, 17, 17, 17, 17, 17, 31],
        [31, 16, 16, 30, 16, 16, 31],
    ];
    // SILICON ECHOES: the final S shares the first glyph.
    let scale = (rect.w / 100).clamp(2, 6) as i32;
    let left = rect.x + (rect.w as i32 - 83 * scale) / 2;
    for index in 0..14 {
        let glyph = if index == 13 {
            glyphs[0]
        } else {
            glyphs[index]
        };
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    let tile = Rect::new(
                        left + (index as i32 * 6 + col) * scale,
                        rect.y + row as i32 * scale,
                        (scale - 1) as u32,
                        (scale - 1) as u32,
                    );
                    canvas.blend_rect(
                        Rect::new(tile.x + 2, tile.y + 3, tile.w + 2, tile.h + 2),
                        amber(36),
                    );
                    canvas.fill_rect(tile, BONE);
                }
            }
        }
    }
}
