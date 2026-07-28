use std::fs;
use std::path::PathBuf;
use std::process::Command;

use sun_img::{encode_tga_rgba32, ImageRgba8};

fn fixture_tga() -> Vec<u8> {
    encode_tga_rgba32(&ImageRgba8 {
        width: 2,
        height: 2,
        pixels: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
        ],
    })
    .unwrap()
}

fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("sun-imgc-{name}-{}", std::process::id()));
    path
}

fn cli_bin() -> String {
    std::env::var("CARGO_BIN_EXE_sun-imgc").expect("sun-imgc test binary path")
}

#[test]
fn inspect_success_on_valid_tga() {
    let input = temp_path("inspect.tga");
    fs::write(&input, fixture_tga()).unwrap();
    let output = Command::new(cli_bin())
        .arg("inspect")
        .arg(&input)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("format: tga"));
    assert!(stdout.contains("width: 2"));
    let _ = fs::remove_file(input);
}

#[test]
fn convert_success_on_valid_tga() {
    let input = temp_path("convert-in.tga");
    let output = temp_path("convert-out.tga");
    fs::write(&input, fixture_tga()).unwrap();
    let status = Command::new(cli_bin())
        .arg("convert")
        .arg(&input)
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = fs::read(&output).unwrap();
    assert_eq!(bytes[2], 2);
    assert_eq!(bytes[16], 32);
    let _ = fs::remove_file(input);
    let _ = fs::remove_file(output);
}

#[test]
fn convert_fails_on_garbage_input() {
    let input = temp_path("garbage.bin");
    let output = temp_path("garbage.tga");
    fs::write(&input, b"nope").unwrap();
    let result = Command::new(cli_bin())
        .arg("convert")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("unsupported image format") || stderr.contains("truncated"));
    let _ = fs::remove_file(input);
    let _ = fs::remove_file(output);
}

#[test]
fn to_simg_and_from_simg_roundtrip() {
    let input = temp_path("rt-in.tga");
    let simg = temp_path("rt-out.simg");
    let back = temp_path("rt-back.tga");
    fs::write(&input, fixture_tga()).unwrap();
    let out = Command::new(cli_bin())
        .args([
            "to-simg",
            "--verify",
            &input.to_string_lossy(),
            &simg.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("method:"));
    assert!(stdout.contains("verify: ok"));
    let status = Command::new(cli_bin())
        .args([
            "from-simg",
            &simg.to_string_lossy(),
            &back.to_string_lossy(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let inspect = Command::new(cli_bin())
        .args(["inspect", &simg.to_string_lossy()])
        .output()
        .unwrap();
    assert!(inspect.status.success());
    let text = String::from_utf8(inspect.stdout).unwrap();
    assert!(text.contains("format: simg-v2"));
    let _ = fs::remove_file(input);
    let _ = fs::remove_file(simg);
    let _ = fs::remove_file(back);
}
