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
    let output = Command::new(cli_bin()).arg("inspect").arg(&input).output().unwrap();
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
