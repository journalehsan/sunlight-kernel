//! `minitype-demo` — renders a font sample sheet to a PPM image for visual verification.
//!
//! Usage (host, requires the "std" feature):
//!   cargo run --bin minitype-demo --features std -p sun-font
//!
//! The output file `minitype-demo.ppm` can be viewed with any image viewer.

fn main() {
    use sun_font::{draw_text, FontRole, TextStyle};
    use sunlight_ui::{
        paint::Canvas,
        theme::{Color, Theme},
    };

    let w = 720u32;
    let h = 400u32;
    let theme = Theme::sunlight_dark();

    let mut pixels = vec![0u32; (w * h) as usize];
    {
        let mut canvas = Canvas::new(&mut pixels, w, w, h);
        canvas.fill_rect(sunlight_ui::Rect::new(0, 0, w, h), theme.bg);

        let samples: &[(&str, FontRole, Color)] = &[
            ("SunlightOS",                                    FontRole::UiLarge,    theme.accent),
            ("The quick brown fox jumps over the lazy dog.",  FontRole::UiRegular,  theme.text),
            ("0123456789  /root/Pictures/Sample Pictures",    FontRole::MonoRegular, theme.text),
            ("Regular: file names, toolbar labels",           FontRole::UiRegular,  theme.text_dim),
            ("Medium: selected items, emphasis",              FontRole::UiMedium,   theme.text),
            ("Small: status text, captions (11px)",           FontRole::UiSmall,    theme.text_dim),
            ("Large (16 px) - window titles",                 FontRole::UiLarge,    theme.text),
        ];

        let mut y = 24i32;
        for (text, role, color) in samples {
            let style = TextStyle::new(*role, *color);
            draw_text(&mut canvas, text, 20, y, &style);
            y += sun_font::line_height(*role) as i32 + 12;
        }
    }

    // Write as PPM (P6 binary) to stdout or file.
    let path = "minitype-demo.ppm";
    let mut data = format!("P6\n{} {}\n255\n", w, h).into_bytes();
    for px in &pixels {
        data.push(((px >> 16) & 0xFF) as u8); // R
        data.push(((px >>  8) & 0xFF) as u8); // G
        data.push(( px        & 0xFF) as u8); // B
    }
    std::fs::write(path, &data).expect("failed to write minitype-demo.ppm");
    println!("Wrote {path}  ({w}×{h})");
}
