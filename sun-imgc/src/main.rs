//! sun-imgc — host-side image inspect / convert / SIMG v2 tooling for SunlightOS.
//!
//! # Legal / patent notice
//!
//! SIMG v2 uses LZ4 and Sub filtering. We are **not sure** of patent-free status
//! and need formal legal review before “patent free” redistribution claims.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

use sun_img::{
    convert_to_simg_v2, convert_to_tga_rgba32, decode_image, detect_format, encode_simg_v2,
    encode_simg_v2_with, encode_tga_rgba32, inspect_tga, parse_simg_v2_header, simg_v2_method_name,
    ImageFormat, ImageRgba8, SimgV2Compression, SimgV2Filter,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("sun-imgc: {err}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        return Err(usage());
    };

    match cmd.as_str() {
        "inspect" => {
            let input = args.next().ok_or_else(usage)?;
            if args.next().is_some() {
                return Err(usage());
            }
            inspect_cmd(Path::new(&input))
        }
        "convert" => {
            let input = args.next().ok_or_else(usage)?;
            let output = args.next().ok_or_else(usage)?;
            if args.next().is_some() {
                return Err(usage());
            }
            convert_cmd(Path::new(&input), Path::new(&output))
        }
        "to-simg" => {
            let mut verify = false;
            let mut force_method: Option<(SimgV2Compression, SimgV2Filter)> = None;
            let mut paths = Vec::new();
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--verify" => verify = true,
                    "--method" => {
                        let m = args
                            .next()
                            .ok_or_else(|| "--method requires raw|lz4|sub+lz4".to_string())?;
                        force_method = Some(parse_method(&m)?);
                    }
                    _ => paths.push(a),
                }
            }
            if paths.len() != 2 {
                return Err(
                    "usage: sun-imgc to-simg [--verify] [--method raw|lz4|sub+lz4] <input> <output.simg>"
                        .into(),
                );
            }
            to_simg_cmd(
                Path::new(&paths[0]),
                Path::new(&paths[1]),
                verify,
                force_method,
            )
        }
        "from-simg" => {
            let input = args.next().ok_or_else(usage)?;
            let output = args.next().ok_or_else(usage)?;
            if args.next().is_some() {
                return Err(usage());
            }
            from_simg_cmd(Path::new(&input), Path::new(&output))
        }
        "bench-corpus" => {
            let dir = args
                .next()
                .ok_or_else(|| "usage: sun-imgc bench-corpus <dir> [--limit N]".to_string())?;
            let mut limit = usize::MAX;
            while let Some(a) = args.next() {
                if a == "--limit" {
                    let n = args
                        .next()
                        .ok_or_else(|| "--limit requires a number".to_string())?;
                    limit = n.parse().map_err(|_| "invalid --limit".to_string())?;
                } else {
                    return Err(format!("unknown argument: {a}"));
                }
            }
            bench_corpus_cmd(Path::new(&dir), limit)
        }
        _ => Err(usage()),
    }
}

fn parse_method(s: &str) -> Result<(SimgV2Compression, SimgV2Filter), String> {
    match s {
        "raw" => Ok((SimgV2Compression::None, SimgV2Filter::None)),
        "lz4" => Ok((SimgV2Compression::Lz4, SimgV2Filter::None)),
        "sub+lz4" | "sub-lz4" | "sublz4" => Ok((SimgV2Compression::Lz4, SimgV2Filter::Sub)),
        _ => Err(format!("unknown method '{s}' (want raw|lz4|sub+lz4)")),
    }
}

fn inspect_cmd(input: &Path) -> Result<(), String> {
    let bytes =
        fs::read(input).map_err(|err| format!("failed to read {}: {err}", input.display()))?;
    let format = detect_format(&bytes);
    println!("format: {}", format_name(format));
    match format {
        ImageFormat::Tga => {
            let info = inspect_tga(&bytes).map_err(|err| err.to_string())?;
            println!("width: {}", info.width);
            println!("height: {}", info.height);
            println!("bit_depth: {}", info.bit_depth);
            println!(
                "origin: {}",
                match info.origin {
                    sun_img::TgaOrigin::TopLeft => "top-left",
                    sun_img::TgaOrigin::BottomLeft => "bottom-left",
                }
            );
            println!(
                "support: {}",
                if info.supported {
                    "supported"
                } else {
                    "unsupported"
                }
            );
        }
        ImageFormat::SimgV2 => {
            let h = parse_simg_v2_header(&bytes).map_err(|err| err.to_string())?;
            println!("version: {}", h.version);
            println!("header_size: {}", h.header_size);
            println!("width: {}", h.width);
            println!("height: {}", h.height);
            println!("pixel_format: {}", h.pixel_format);
            println!("alpha_mode: {}", h.alpha_mode);
            println!("method: {}", simg_v2_method_name(h.compression, h.filter));
            println!("uncompressed_size: {}", h.uncompressed_size);
            println!("payload_size: {}", h.payload_size);
            println!("flags: {:#x}", h.flags);
            println!("crc32: {:#x}", h.crc32);
            println!("file_size: {}", bytes.len());
        }
        _ => {}
    }
    Ok(())
}

fn convert_cmd(input: &Path, output: &Path) -> Result<(), String> {
    let bytes =
        fs::read(input).map_err(|err| format!("failed to read {}: {err}", input.display()))?;
    let converted = convert_to_tga_rgba32(&bytes).map_err(|err| err.to_string())?;
    write_atomic(output, &converted)
}

fn to_simg_cmd(
    input: &Path,
    output: &Path,
    verify: bool,
    force: Option<(SimgV2Compression, SimgV2Filter)>,
) -> Result<(), String> {
    let bytes =
        fs::read(input).map_err(|err| format!("failed to read {}: {err}", input.display()))?;
    let image = decode_image(&bytes).map_err(|err| err.to_string())?;
    let report = match force {
        Some((c, f)) => encode_simg_v2_with(&image, c, f).map_err(|e| e.to_string())?,
        None => encode_simg_v2(&image).map_err(|e| e.to_string())?,
    };
    let method = simg_v2_method_name(report.compression, report.filter);
    let raw = report.raw_payload_size;
    let enc = report.file_size;
    let saved = raw.saturating_sub(report.encoded_payload_size);
    let ratio = if raw == 0 {
        0.0
    } else {
        100.0 * (1.0 - report.encoded_payload_size as f64 / raw as f64)
    };
    println!("method: {method}");
    println!("raw_payload: {raw}");
    println!("encoded_payload: {}", report.encoded_payload_size);
    println!("file_size: {enc}");
    println!("saved_payload_bytes: {saved}");
    println!("payload_compression_ratio_pct: {ratio:.2}");

    if verify {
        let decoded = decode_image(&report.bytes).map_err(|e| e.to_string())?;
        if decoded != image {
            return Err("verify failed: decoded pixels differ from source".into());
        }
        println!("verify: ok");
    }

    write_atomic(output, &report.bytes)
}

fn from_simg_cmd(input: &Path, output: &Path) -> Result<(), String> {
    let bytes =
        fs::read(input).map_err(|err| format!("failed to read {}: {err}", input.display()))?;
    if !sun_img::is_simg_v2(&bytes) {
        return Err("input is not SIMG v2".into());
    }
    let image = decode_image(&bytes).map_err(|err| err.to_string())?;
    let tga = encode_tga_rgba32(&image).map_err(|err| err.to_string())?;
    write_atomic(output, &tga)
}

fn bench_corpus_cmd(dir: &Path, limit: usize) -> Result<(), String> {
    let mut files = collect_images(dir)?;
    files.sort();
    if files.len() > limit {
        files.truncate(limit);
    }
    if files.is_empty() {
        return Err(format!("no .tga/.simg files under {}", dir.display()));
    }

    let mut total_current = 0u64;
    let mut total_raw = 0u64;
    let mut total_v2 = 0u64;
    let mut total_v2_raw = 0u64;
    let mut total_v2_lz4 = 0u64;
    let mut total_v2_sub = 0u64;
    let mut n_raw = 0u64;
    let mut n_lz4 = 0u64;
    let mut n_sub = 0u64;
    let mut encode_ns = 0u128;
    let mut decode_ns = 0u128;
    let mut decode_iters = 0u64;

    println!(
        "file\twxh\traw\tcurrent\tv2_raw\tv2_lz4\tv2_sub+lz4\tselected\tv2_size\tsaved_vs_raw%\tsaved_vs_current%"
    );

    for path in &files {
        let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let current = bytes.len() as u64;
        total_current += current;

        let image = match decode_image(&bytes) {
            Ok(img) => img,
            Err(err) => {
                eprintln!("skip {}: {err}", path.display());
                continue;
            }
        };
        let raw = (image.width as u64) * (image.height as u64) * 4;
        total_raw += raw;

        let t0 = Instant::now();
        let r_raw = encode_simg_v2_with(&image, SimgV2Compression::None, SimgV2Filter::None)
            .map_err(|e| e.to_string())?;
        let r_lz4 = encode_simg_v2_with(&image, SimgV2Compression::Lz4, SimgV2Filter::None)
            .map_err(|e| e.to_string())?;
        let r_sub = encode_simg_v2_with(&image, SimgV2Compression::Lz4, SimgV2Filter::Sub)
            .map_err(|e| e.to_string())?;
        let chosen = encode_simg_v2(&image).map_err(|e| e.to_string())?;
        encode_ns += t0.elapsed().as_nanos();

        total_v2_raw += r_raw.file_size as u64;
        total_v2_lz4 += r_lz4.file_size as u64;
        total_v2_sub += r_sub.file_size as u64;
        total_v2 += chosen.file_size as u64;
        match (chosen.compression, chosen.filter) {
            (SimgV2Compression::None, _) => n_raw += 1,
            (SimgV2Compression::Lz4, SimgV2Filter::None) => n_lz4 += 1,
            (SimgV2Compression::Lz4, SimgV2Filter::Sub) => n_sub += 1,
        }

        let t1 = Instant::now();
        let rounds = 3u32;
        for _ in 0..rounds {
            let dec = decode_image(&chosen.bytes).map_err(|e| e.to_string())?;
            if dec.pixels.len() != image.pixels.len() {
                return Err(format!("decode size mismatch for {}", path.display()));
            }
            // Byte-for-byte check once
        }
        let dec = decode_image(&chosen.bytes).map_err(|e| e.to_string())?;
        if dec != image {
            return Err(format!("lossless check failed for {}", path.display()));
        }
        decode_ns += t1.elapsed().as_nanos();
        decode_iters += u64::from(rounds) + 1;

        let method = simg_v2_method_name(chosen.compression, chosen.filter);
        let saved_raw = pct_saved(raw, chosen.file_size as u64);
        let saved_cur = pct_saved(current, chosen.file_size as u64);
        println!(
            "{}\t{}x{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}",
            path.display(),
            image.width,
            image.height,
            raw,
            current,
            r_raw.file_size,
            r_lz4.file_size,
            r_sub.file_size,
            method,
            chosen.file_size,
            saved_raw,
            saved_cur
        );
    }

    println!();
    println!("files: {}", files.len());
    println!("total_current_bytes: {total_current}");
    println!("total_raw_bgra_bytes: {total_raw}");
    println!("total_v2_selected_bytes: {total_v2}");
    println!("total_v2_raw_bytes: {total_v2_raw}");
    println!("total_v2_lz4_bytes: {total_v2_lz4}");
    println!("total_v2_sub_lz4_bytes: {total_v2_sub}");
    println!("selected_raw: {n_raw}");
    println!("selected_lz4: {n_lz4}");
    println!("selected_sub_lz4: {n_sub}");
    println!(
        "saved_vs_current_pct: {:.2}",
        pct_saved(total_current, total_v2)
    );
    println!("saved_vs_raw_pct: {:.2}", pct_saved(total_raw, total_v2));
    println!("encode_time_ms: {:.3}", encode_ns as f64 / 1_000_000.0);
    println!(
        "decode_time_ms ({} iters): {:.3}",
        decode_iters,
        decode_ns as f64 / 1_000_000.0
    );
    Ok(())
}

fn pct_saved(before: u64, after: u64) -> f64 {
    if before == 0 {
        0.0
    } else {
        100.0 * (1.0 - after as f64 / before as f64)
    }
}

fn collect_images(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for ent in rd {
            let ent = ent.map_err(|e| e.to_string())?;
            let path = ent.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let e = ext.to_ascii_lowercase();
                if e == "tga" || e == "simg" {
                    out.push(path);
                }
            }
        }
        Ok(())
    }
    walk(dir, &mut out)?;
    Ok(out)
}

fn write_atomic(output: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = tmp_output_path(output);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&tmp, data).map_err(|err| format!("failed to write {}: {err}", tmp.display()))?;
    fs::rename(&tmp, output).map_err(|err| {
        format!(
            "failed to rename {} -> {}: {err}",
            tmp.display(),
            output.display()
        )
    })?;
    Ok(())
}

fn tmp_output_path(output: &Path) -> PathBuf {
    let mut tmp = output.as_os_str().to_os_string();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Tga => "tga",
        ImageFormat::SimgV2 => "simg-v2",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Unknown => "unknown",
    }
}

fn usage() -> String {
    "usage: sun-imgc inspect <input>\n       sun-imgc convert <input> <output.tga>\n       sun-imgc to-simg [--verify] [--method raw|lz4|sub+lz4] <input> <output.simg>\n       sun-imgc from-simg <input.simg> <output.tga>\n       sun-imgc bench-corpus <dir> [--limit N]"
        .to_string()
}

#[allow(dead_code)]
fn _keep_convert_to_simg_v2_link(bytes: &[u8]) -> Result<sun_img::SimgV2EncodeReport, String> {
    convert_to_simg_v2(bytes).map_err(|e| e.to_string())
}

#[allow(dead_code)]
fn _keep_image_type(_: &ImageRgba8) {}
