//! Build-time TrueType to MiniType conversion.
//!
//! MTF1 remains readable at runtime. New assets use MTF2: an explicit Unicode
//! cmap, glyph-offset table, alpha-mask glyph records, and an optional fixed-
//! width sequence table for bounded longest-match emoji lookup.

use fontdue::{Font, FontSettings};
use rustybuzz::{Face as ShapingFace, UnicodeBuffer};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fs};

const MTF2_HEADER_SIZE: usize = 32;
const MAX_SEQUENCE_LEN: usize = 16;
const SEQUENCE_ENTRY_SIZE: usize = 4 + MAX_SEQUENCE_LEN * 4 + 4;
const MAX_GLYPH_DIMENSION: usize = 64;
const MAX_FONT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
struct RasterGlyph {
    advance: u8,
    left: i8,
    top: i8,
    width: u8,
    height: u8,
    pixels: Vec<u8>,
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().unwrap();
    let inter = root.join("docs/fonts/Inter/static");
    let fira = root.join("docs/fonts/FiraCode/ttf");
    let material = root.join("assets/fonts/Material-Icons/MaterialIcons-Regular.ttf");
    let emoji_ttf = root.join("assets/fonts/OpenMoji-Black/OpenMoji-black-glyf.ttf");
    let emoji_json = root.join("assets/fonts/OpenMoji-Black/openmoji.json");
    let noto_serif = PathBuf::from("/usr/share/fonts/noto/NotoSerif-Regular.ttf");
    if !noto_serif.is_file() {
        panic!(
            "sun-font: Sun Serif source missing: {}",
            noto_serif.display()
        );
    }

    let sources = [
        inter.join("Inter_18pt-Regular.ttf"),
        inter.join("Inter_18pt-Medium.ttf"),
        inter.join("Inter_18pt-SemiBold.ttf"),
        fira.join("FiraCode-Regular.ttf"),
        fira.join("FiraCode-Medium.ttf"),
        noto_serif.clone(),
        material,
        emoji_ttf.clone(),
        emoji_json.clone(),
    ];
    for path in &sources {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-changed=build.rs");

    let font = |path: &Path| {
        Font::from_bytes(
            fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display())),
            FontSettings::default(),
        )
        .unwrap_or_else(|_| panic!("cannot parse {}", path.display()))
    };
    let regular = font(&sources[0]);
    let medium = font(&sources[1]);
    let semibold = font(&sources[2]);
    let mono = font(&sources[3]);
    let mono_medium = font(&sources[4]);
    let serif = font(&sources[5]);
    let material = font(&sources[6]);
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    for (face, px, name) in [
        (&regular, 11.0, "sunlight_ui_11.mtf"),
        (&regular, 13.0, "sunlight_ui_13.mtf"),
        (&medium, 13.0, "sunlight_ui_medium_13.mtf"),
        (&semibold, 13.0, "sunlight_ui_semibold_13.mtf"),
        (&regular, 16.0, "sunlight_ui_16.mtf"),
        (&medium, 18.0, "sunlight_ui_title_18.mtf"),
        (&mono, 14.0, "sunlight_mono_regular_14.mtf"),
        (&mono_medium, 14.0, "sunlight_mono_medium_14.mtf"),
        (&serif, 16.0, "sunlight_serif_regular_16.mtf"),
        (&material, 16.0, "material_icons_16.mtf"),
        (&material, 24.0, "material_icons_24.mtf"),
    ] {
        generate_text_font(face, px, &out.join(name));
    }
    generate_emoji_font(&emoji_ttf, &emoji_json, &out.join("sunlight_emoji_16.mtf"));
}

fn text_codepoints() -> Vec<u32> {
    let mut codes = (0x20..=0x7e).collect::<Vec<_>>();
    codes.extend(0xa0..=0xff);
    codes.extend([
        0x152, 0x153, 0x160, 0x161, 0x178, 0x17d, 0x17e, 0x192, 0x2c6, 0x2dc, 0x2013, 0x2014,
        0x2018, 0x2019, 0x201a, 0x201c, 0x201d, 0x201e, 0x2020, 0x2021, 0x2022, 0x2026, 0x2030,
        0x2039, 0x203a, 0x20ac, 0x2122, 0x2190, 0x2191, 0x2192, 0x2193,
    ]);
    codes.sort_unstable();
    codes.dedup();
    codes
}

fn generate_text_font(font: &Font, px: f32, output: &Path) {
    let mut cmap = Vec::new();
    let mut glyphs = Vec::new();
    for code in text_codepoints() {
        let Some(ch) = char::from_u32(code) else {
            continue;
        };
        let glyph_id = font.lookup_glyph_index(ch);
        if glyph_id == 0 && ch != ' ' {
            continue;
        }
        cmap.push((code, glyphs.len() as u32));
        glyphs.push(rasterize(font, glyph_id, px));
    }
    write_mtf2(font, px, &cmap, &[], &glyphs, output);
}

fn generate_emoji_font(ttf_path: &Path, json_path: &Path, output: &Path) {
    let bytes = fs::read(ttf_path).unwrap();
    let font = Font::from_bytes(bytes.clone(), FontSettings::default()).unwrap();
    let face = ShapingFace::from_slice(&bytes, 0).expect("OpenMoji shaping face");
    let records: Value = serde_json::from_slice(&fs::read(json_path).unwrap()).unwrap();
    let mut cmap_glyphs = BTreeMap::<u32, u16>::new();
    let mut sequences = Vec::<(Vec<u32>, u16)>::new();
    let mut omitted_sequences = 0usize;
    let mut omitted_private = Vec::<String>::new();

    for record in records.as_array().unwrap() {
        if record.get("unicode").and_then(Value::as_f64).is_none() {
            if let Some(hexcode) = record.get("hexcode").and_then(Value::as_str) {
                omitted_private.push(hexcode.to_owned());
            }
            continue; // OpenMoji private-use extras are not default fallback.
        }
        let Some(hexcode) = record.get("hexcode").and_then(Value::as_str) else {
            continue;
        };
        let sequence = hexcode
            .split('-')
            .filter_map(|part| u32::from_str_radix(part, 16).ok())
            .collect::<Vec<_>>();
        if sequence.is_empty() || sequence.len() > MAX_SEQUENCE_LEN {
            omitted_sequences += usize::from(sequence.len() > 1);
            continue;
        }
        let text = sequence
            .iter()
            .filter_map(|code| char::from_u32(*code))
            .collect::<String>();
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&text);
        let shaped = rustybuzz::shape(&face, &[], buffer);
        if shaped.glyph_infos().len() != 1 {
            omitted_sequences += usize::from(sequence.len() > 1);
            continue;
        }
        let glyph_id = shaped.glyph_infos()[0].glyph_id as u16;
        if sequence.len() == 1 {
            cmap_glyphs.insert(sequence[0], glyph_id);
        } else {
            sequences.push((sequence, glyph_id));
        }
    }

    let glyph_ids = cmap_glyphs
        .values()
        .chain(sequences.iter().map(|(_, glyph)| glyph))
        .copied()
        .collect::<BTreeSet<_>>();
    let glyph_index = glyph_ids
        .iter()
        .enumerate()
        .map(|(index, glyph)| (*glyph, index as u32))
        .collect::<BTreeMap<_, _>>();
    let glyphs = glyph_ids
        .iter()
        .map(|glyph| rasterize(&font, *glyph, 16.0))
        .collect::<Vec<_>>();
    let cmap = cmap_glyphs
        .into_iter()
        .map(|(code, glyph)| (code, glyph_index[&glyph]))
        .collect::<Vec<_>>();
    let mut sequence_map = sequences
        .into_iter()
        .map(|(sequence, glyph)| (sequence, glyph_index[&glyph]))
        .collect::<Vec<_>>();
    sequence_map.sort_by(|left, right| left.0.cmp(&right.0));
    write_mtf2(&font, 16.0, &cmap, &sequence_map, &glyphs, output);
    write_emoji_manifest(
        &output.with_file_name("sunlight_emoji_manifest.txt"),
        &cmap,
        &sequence_map,
        &omitted_private,
        omitted_sequences,
    );
    println!(
        "cargo:warning=Sun Emoji: cmap={} sequences={} glyphs={} omitted_sequences={} bytes={}",
        cmap.len(),
        sequence_map.len(),
        glyphs.len(),
        omitted_sequences,
        fs::metadata(output).unwrap().len()
    );
}

fn write_emoji_manifest(
    output: &Path,
    cmap: &[(u32, u32)],
    sequences: &[(Vec<u32>, u32)],
    omitted_private: &[String],
    omitted_unsupported: usize,
) {
    let mut file = fs::File::create(output).unwrap();
    writeln!(file, "Sun Emoji / OpenMoji 17.0.0").unwrap();
    writeln!(file, "profile=full-unicode").unwrap();
    writeln!(file, "included_single_codepoints={}", cmap.len()).unwrap();
    writeln!(file, "included_sequences={}", sequences.len()).unwrap();
    writeln!(file, "omitted_private_use={}", omitted_private.len()).unwrap();
    writeln!(file, "omitted_unsupported_sequences={omitted_unsupported}").unwrap();
    writeln!(file, "[included]").unwrap();
    for (code, _) in cmap {
        writeln!(file, "{code:04X}").unwrap();
    }
    for (sequence, _) in sequences {
        for (index, code) in sequence.iter().enumerate() {
            if index > 0 {
                write!(file, "-").unwrap();
            }
            write!(file, "{code:04X}").unwrap();
        }
        writeln!(file).unwrap();
    }
    writeln!(file, "[omitted-private-use]").unwrap();
    for sequence in omitted_private {
        writeln!(file, "{sequence}").unwrap();
    }
}

fn rasterize(font: &Font, glyph_id: u16, px: f32) -> RasterGlyph {
    let (metrics, source_pixels) = font.rasterize_indexed(glyph_id, px);
    let width = metrics.width.min(MAX_GLYPH_DIMENSION);
    let height = metrics.height.min(MAX_GLYPH_DIMENSION);
    let pixels = if width == metrics.width && height == metrics.height {
        source_pixels
    } else {
        let mut cropped = Vec::with_capacity(width * height);
        for row in 0..height {
            let start = row * metrics.width;
            cropped.extend_from_slice(&source_pixels[start..start + width]);
        }
        cropped
    };
    RasterGlyph {
        advance: (metrics.advance_width.ceil().max(0.0) as u32).min(255) as u8,
        left: metrics.xmin.clamp(-128, 127) as i8,
        top: (metrics.ymin + metrics.height as i32).clamp(-128, 127) as i8,
        width: width as u8,
        height: height as u8,
        pixels,
    }
}

fn write_mtf2(
    font: &Font,
    px: f32,
    cmap: &[(u32, u32)],
    sequences: &[(Vec<u32>, u32)],
    glyphs: &[RasterGlyph],
    output: &Path,
) {
    let metrics = font.horizontal_line_metrics(px).unwrap();
    let cmap_offset = MTF2_HEADER_SIZE;
    let sequence_offset = cmap_offset + cmap.len() * 8;
    let glyph_offsets_offset = sequence_offset + sequences.len() * SEQUENCE_ENTRY_SIZE;
    let glyph_data_offset = glyph_offsets_offset + glyphs.len() * 4;
    let mut glyph_offsets = Vec::with_capacity(glyphs.len());
    let mut cursor = glyph_data_offset;
    for glyph in glyphs {
        glyph_offsets.push(cursor as u32);
        cursor += 5 + glyph.width as usize * glyph.height as usize;
    }
    assert!(cursor <= MAX_FONT_BYTES, "MiniType font exceeds size limit");

    let mut out = fs::File::create(output).unwrap();
    out.write_all(b"MTF2").unwrap();
    out.write_all(&[
        (metrics.ascent - metrics.descent + metrics.line_gap)
            .ceil()
            .clamp(1.0, 255.0) as u8,
        metrics.ascent.ceil().clamp(0.0, 255.0) as u8,
        1, // GlyphPaintKind::MonochromeMask
        MAX_SEQUENCE_LEN as u8,
    ])
    .unwrap();
    for value in [
        glyphs.len(),
        cmap.len(),
        sequences.len(),
        cmap_offset,
        sequence_offset,
        glyph_offsets_offset,
    ] {
        out.write_all(&(value as u32).to_le_bytes()).unwrap();
    }
    for (code, glyph) in cmap {
        out.write_all(&code.to_le_bytes()).unwrap();
        out.write_all(&glyph.to_le_bytes()).unwrap();
    }
    for (sequence, glyph) in sequences {
        out.write_all(&[sequence.len() as u8, 0, 0, 0]).unwrap();
        for index in 0..MAX_SEQUENCE_LEN {
            out.write_all(&sequence.get(index).copied().unwrap_or(0).to_le_bytes())
                .unwrap();
        }
        out.write_all(&glyph.to_le_bytes()).unwrap();
    }
    for offset in glyph_offsets {
        out.write_all(&offset.to_le_bytes()).unwrap();
    }
    for glyph in glyphs {
        out.write_all(&[
            glyph.advance,
            glyph.left as u8,
            glyph.top as u8,
            glyph.width,
            glyph.height,
        ])
        .unwrap();
        out.write_all(&glyph.pixels[..glyph.width as usize * glyph.height as usize])
            .unwrap();
    }
}
