//! Immediate game scene on the existing Canvas, with tile/actor culling.
use crate::input::{Input, TOP};
use core::fmt::{self, Write};
use sun_font::{draw_text, FontRole, TextStyle};
use sunlight_empires::{BuildingKind, Terrain, Tile, UnitKind, World, MAP_SIZE, SUB};
use sunlight_ui::{Canvas, Color, Rect, Theme};
/// Stack formatting avoids per-frame Strings.
struct Text {
    bytes: [u8; 240],
    len: usize,
}
impl Text {
    fn new() -> Self {
        Self {
            bytes: [0; 240],
            len: 0,
        }
    }
    fn str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}
impl Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.len + s.len() > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
        Ok(())
    }
}
fn text(c: &mut Canvas, s: &str, x: i32, y: i32, color: Color) {
    draw_text(c, s, x, y, &TextStyle::new(FontRole::UiRegular, color));
}
pub fn outline(c: &mut Canvas, r: Rect, col: Color) {
    c.hline(r.x, r.y, r.w, col);
    c.hline(r.x, r.bottom() - 1, r.w, col);
    c.vline(r.x, r.y, r.h, col);
    c.vline(r.right() - 1, r.y, r.h, col);
}
pub fn draw(c: &mut Canvas, theme: &Theme, w: &World, input: &mut Input, notice: &str) {
    let top = TOP;
    let bottom = (c.height as i32 - 116).max(top);
    input
        .camera
        .clamp(MAP_SIZE, MAP_SIZE, c.width as i32, bottom - top);
    let camera = input.camera;
    let z = camera.tile_px;
    c.fill_rect(Rect::new(0, 0, c.width, c.height), theme.bg);
    let rows = (bottom - top + z - 1) / z;
    let cols = (c.width as i32 + z - 1) / z;
    for y in camera.y..(camera.y + rows).min(MAP_SIZE) {
        for x in camera.x..(camera.x + cols).min(MAP_SIZE) {
            let t = Tile::new(x, y);
            let n = t.index().unwrap();
            let (px, py) = camera.tile_to_screen(x, y, top);
            let mut col = match w.map.terrain[n] {
                Terrain::Grass => Color::rgb(52, 91, 59),
                Terrain::Ground => Color::rgb(93, 94, 62),
                Terrain::Forest => Color::rgb(27, 62, 38),
                Terrain::Water => Color::rgb(40, 89, 133),
                Terrain::Stone => Color::rgb(112, 118, 125),
            };
            if !w.explored(0, t) {
                col = Color::rgb(10, 14, 19);
            } else if !w.visible(0, t) {
                col = col.darken(35);
            }
            c.fill_rect(Rect::new(px, py, z as u32 - 1, z as u32 - 1), col);
            if w.explored(0, t) && matches!(w.map.terrain[n], Terrain::Forest | Terrain::Stone) {
                c.fill_rect(
                    Rect::new(px + z / 3, py + z / 4, (z / 3) as u32, (z / 2) as u32),
                    if w.map.remaining[n] > 0 {
                        col.lighten(32)
                    } else {
                        col.darken(20)
                    },
                );
            }
        }
    }
    // Map boundaries are visible even where a viewport is larger than the map.
    let (bx, by) = camera.tile_to_screen(0, 0, top);
    outline(
        c,
        Rect::new(bx, by, (MAP_SIZE * z) as u32, (MAP_SIZE * z) as u32),
        theme.accent,
    );
    for b in w
        .buildings
        .iter()
        .filter(|b| b.owner == 0 || w.visible(0, b.tile))
    {
        let (px, py) = camera.tile_to_screen(b.tile.x, b.tile.y, top);
        if px + z < 0 || px >= c.width as i32 || py + z <= top || py >= bottom {
            continue;
        }
        let r = Rect::new(px + 2, py + 2, (z - 4) as u32, (z - 4) as u32);
        c.fill_rect(
            r,
            if b.owner != 0 {
                Color::rgb(185, 65, 65)
            } else if b.complete {
                Color::rgb(153, 124, 85)
            } else {
                Color::rgb(105, 87, 64)
            },
        );
        let label = match b.kind {
            BuildingKind::TownCenter => "T",
            BuildingKind::House => "H",
            BuildingKind::Storage => "S",
            BuildingKind::University => "U",
        };
        text(c, label, px + z / 3, py + z / 4, theme.text);
        c.fill_rect(
            Rect::new(
                px + 2,
                py + 2,
                ((z - 4) as u32 * b.health / sunlight_empires::content::BUILDING_HEALTH).max(1),
                2,
            ),
            theme.ok,
        );
        if !b.complete {
            c.fill_rect(
                Rect::new(
                    px + 2,
                    py + z - 5,
                    ((z - 4) as u32 * b.progress / b.kind.duration()).max(1),
                    3,
                ),
                theme.accent,
            );
        }
        if input.building == Some(b.id) {
            outline(c, r, theme.accent);
        }
        if let Some(t) = b.rally {
            let (x, y) = camera.tile_to_screen(t.x, t.y, top);
            outline(
                c,
                Rect::new(x + z / 3, y + z / 3, (z / 3) as u32, (z / 3) as u32),
                theme.accent,
            );
        }
    }
    let (r, g, b) = w.civilization.color();
    let civ = Color::rgb(r, g, b);
    for u in w
        .units
        .iter()
        .filter(|u| u.owner == 0 || w.visible(0, u.tile()))
    {
        let px = (u.x - camera.x * SUB) * z / SUB;
        let py = (u.y - camera.y * SUB) * z / SUB + top;
        if px < 0 || px >= c.width as i32 || py < top || py >= bottom {
            continue;
        }
        let size = if u.kind == UnitKind::Worker {
            z / 3
        } else {
            z / 2
        };
        let r = Rect::new(px - size / 2, py - size / 2, size as u32, size as u32);
        c.fill_rect(
            r,
            if u.owner == 0 {
                civ
            } else {
                Color::rgb(210, 70, 70)
            },
        );
        c.fill_rect(
            Rect::new(
                px - size / 2,
                py - size / 2 - 5,
                (size as u32 * u.health / u.kind.combat().health).max(1),
                3,
            ),
            theme.ok,
        );
        if u.kind == UnitKind::Archer {
            outline(c, r, theme.text);
        }
        if u.kind == UnitKind::Guard {
            c.vline(px + size / 2, py - size / 2, size as u32, theme.text);
        }
        if u.cargo > 0 {
            c.fill_rect(
                Rect::new(px + size / 2, py - 2, 4, 4),
                if u.cargo_stone {
                    Color::rgb(200, 200, 200)
                } else {
                    Color::rgb(224, 193, 128)
                },
            );
        }
        if input.selected.contains(&u.id) {
            outline(c, Rect::new(r.x - 3, r.y - 3, r.w + 6, r.h + 6), theme.text);
            let destination = match u.task {
                sunlight_empires::Task::AttackMove(t) => Some(t),
                sunlight_empires::Task::Attack(id) => w.target(0, id),
                _ => u.destination(),
            };
            if let Some(t) = destination {
                let (x, y) = camera.tile_to_screen(t.x, t.y, top);
                outline(
                    c,
                    Rect::new(x + z / 3, y + z / 3, (z / 3) as u32, (z / 3) as u32),
                    theme.accent,
                );
            }
        }
    }
    for e in &w.effects {
        if !w.visible(0, e.to) {
            continue;
        }
        let (x, y) = camera.tile_to_screen(e.to.x, e.to.y, top);
        if y < top || y >= bottom {
            continue;
        }
        if e.arrow {
            let (fx, fy) = camera.tile_to_screen(e.from.x, e.from.y, top);
            let age = (6 - e.until.saturating_sub(w.tick).min(6)) as i32;
            c.fill_rect(
                Rect::new(
                    fx + (x - fx) * age / 6 + z / 2,
                    fy + (y - fy) * age / 6 + z / 2,
                    5,
                    2,
                ),
                theme.text,
            );
        } else {
            outline(
                c,
                Rect::new(x + 3, y + 3, (z - 6) as u32, (z - 6) as u32),
                theme.danger,
            );
        }
        let mut label = Text::new();
        let _ = write!(label, "-{}", e.damage);
        text(c, label.str(), x, y, theme.danger);
    }
    if let Some((sx, sy)) = input.drag {
        let (x, y) = input.pointer;
        outline(
            c,
            Rect::new(
                sx.min(x),
                sy.min(y),
                (sx - x).unsigned_abs().max(1),
                (sy - y).unsigned_abs().max(1),
            ),
            theme.text,
        );
    }
    if input.placing.is_some() {
        let t = input.tile(input.pointer.0, input.pointer.1);
        let (x, y) = camera.tile_to_screen(t.x, t.y, top);
        outline(
            c,
            Rect::new(x, y, z as u32, z as u32),
            if w.passable(t) {
                theme.ok
            } else {
                theme.danger
            },
        );
    }
    // Panels cover geometry crossing the viewport edge, with no subwidgets.
    c.fill_rect(Rect::new(0, 0, c.width, top as u32), theme.panel);
    let mut line = Text::new();
    let r = w.resources;
    let cap = w.capacity(0);
    let _ = write!(
        line,
        "Food {}   Wood {}   Stone {}   Gold {}   Knowledge {}   Pop {}/{}",
        r.food,
        r.wood,
        r.stone,
        r.gold,
        r.knowledge,
        w.population(0),
        cap
    );
    text(c, w.civilization.name(), 12, 5, civ);
    text(
        c,
        "M2 | Space pause | 1:0.5x 2:1x 3:2x | Esc menu",
        110,
        5,
        theme.text_dim,
    );
    text(c, line.str(), 12, 27, theme.text);
    c.fill_rect(
        Rect::new(0, bottom, c.width, c.height.saturating_sub(bottom as u32)),
        theme.panel,
    );
    let mut line = Text::new();
    if let Some(id) = input.building {
        if let Some(b) = w.buildings.iter().find(|b| b.id == id) {
            let _ = write!(
                line,
                "{:?} #{} | {} | progress {}/{}",
                b.kind,
                b.id,
                if b.complete {
                    "Complete"
                } else {
                    "Construction"
                },
                b.progress,
                b.kind.duration()
            );
            let _ = write!(line, " | HP {}", b.health);
            if let Some((kind, ticks)) = b.training {
                let _ = write!(line, " | Training {:?}: {} ticks", kind, ticks);
            }
            if !b.complete {
                let count = w
                    .units
                    .iter()
                    .filter(|u| u.task == sunlight_empires::Task::Build(b.id))
                    .count();
                let _ = write!(line, " | {} builders", count);
            }
        }
    } else if let Some(id) = input.selected.first() {
        if let Some(u) = w.units.iter().find(|u| u.id == *id) {
            let _ = write!(
                line,
                "{} selected | {:?} #{} | {:?} | HP {}",
                input.selected.len(),
                u.kind,
                u.id,
                u.task,
                u.health
            );
        }
    } else {
        let _ =
            line.write_str("Select a worker. Right-click trees / stone to gather; ground to move.");
    }
    text(c, line.str(), 12, bottom + 9, theme.text);
    for (i, label) in [
        "[H] House 40W",
        "[S] Store 60W",
        "[U] Univ 100W",
        "[X] Cancel",
        "[R] Rally",
    ]
    .iter()
    .enumerate()
    {
        let bw = (c.width as i32 - 16) / 5;
        let x = 8 + i as i32 * bw;
        let r = Rect::new(x, bottom + 33, (bw - 6).max(1) as u32, 27);
        c.fill_rect(r, theme.panel_alt);
        outline(c, r, theme.border);
        text(c, label, x + 5, bottom + 40, theme.text);
    }
    text(c, notice, 12, bottom + 68, theme.accent);
    text(
        c,
        "I/B train (TC) | A attack-move | O stop | D hold | Right-click: order",
        12,
        bottom + 91,
        theme.text_dim,
    );
}

pub fn status(c: &mut Canvas, t: &Theme, s: &str) {
    c.fill_rect(Rect::new(0, 0, c.width, 23), t.panel);
    text(c, s, 12, 5, t.text_dim);
}
pub fn overlay(c: &mut Canvas, t: &Theme, title: &str, hint: &str) {
    let y = c.height as i32 / 2 - 45;
    c.fill_rect(Rect::new(20, y, c.width.saturating_sub(40), 100), t.panel);
    text(c, title, 36, y + 16, t.accent);
    text(c, hint, 36, y + 46, t.text);
    text(
        c,
        "M: mute | -/+: master | [/]: effects | ,/.: music (reserved)",
        36,
        y + 74,
        t.text_dim,
    );
}
