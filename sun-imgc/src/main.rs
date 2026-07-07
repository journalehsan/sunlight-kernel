use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use sun_img::{convert_to_tga_rgba32, detect_format, inspect_tga, ImageFormat};

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
        _ => Err(usage()),
    }
}

fn inspect_cmd(input: &Path) -> Result<(), String> {
    let bytes =
        fs::read(input).map_err(|err| format!("failed to read {}: {err}", input.display()))?;
    let format = detect_format(&bytes);
    println!("format: {}", format_name(format));
    if format == ImageFormat::Tga {
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
    Ok(())
}

fn convert_cmd(input: &Path, output: &Path) -> Result<(), String> {
    let bytes =
        fs::read(input).map_err(|err| format!("failed to read {}: {err}", input.display()))?;
    let converted = convert_to_tga_rgba32(&bytes).map_err(|err| err.to_string())?;
    let tmp = tmp_output_path(output);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&tmp, converted)
        .map_err(|err| format!("failed to write {}: {err}", tmp.display()))?;
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
        ImageFormat::Bmp => "bmp",
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Unknown => "unknown",
    }
}

fn usage() -> String {
    "usage: sun-imgc inspect <input> | sun-imgc convert <input> <output.tga>".to_string()
}
