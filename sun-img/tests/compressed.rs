use sun_img::{decode_image, encode_simg_v2, ImageError};

const RGBA: &[u8] = include_bytes!("fixtures/rgba.png");
const JPEG: &[u8] = include_bytes!("fixtures/baseline.jpg");
const EXPECTED: &[u8] = &[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 12, 34, 56, 64];

#[test]
fn dimensions_can_be_read_without_pixel_data() {
    assert_eq!(sun_img::inspect_dimensions(&RGBA[..33]).unwrap(), (2, 2));
    let sof = JPEG.windows(2).position(|p| p == [0xff, 0xc0]).unwrap();
    let length = u16::from_be_bytes([JPEG[sof + 2], JPEG[sof + 3]]) as usize;
    assert_eq!(
        sun_img::inspect_dimensions(&JPEG[..sof + 2 + length]).unwrap(),
        (8, 8)
    );
    assert!(sun_img::inspect_dimensions(&JPEG[..sof + 4]).is_err());
}

#[test]
fn png_color_alpha_palette_and_adam7_agree() {
    for bytes in [
        RGBA,
        include_bytes!("fixtures/palette.png"),
        include_bytes!("fixtures/adam7.png"),
    ] {
        let image = decode_image(bytes).unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.pixels, EXPECTED);
        let simg = encode_simg_v2(&image).unwrap();
        assert_eq!(decode_image(&simg.bytes).unwrap(), image);
    }
}

#[test]
fn png_grayscale_depths_and_transparent_rgb() {
    let gray = decode_image(include_bytes!("fixtures/gray16.png")).unwrap();
    assert_eq!(gray.pixels, [0x12, 0x12, 0x12, 255, 0xab, 0xab, 0xab, 255]);
    let gray = decode_image(include_bytes!("fixtures/gray1.png")).unwrap();
    assert_eq!(gray.pixels, [0, 0, 0, 255, 255, 255, 255, 255]);
    let rgb = decode_image(include_bytes!("fixtures/rgb_trns.png")).unwrap();
    assert_eq!(rgb.pixels, [255, 0, 0, 0, 0, 255, 0, 255]);
}

#[test]
fn jpeg_baseline_progressive_gray_and_cmyk_are_opaque() {
    for bytes in [JPEG, include_bytes!("fixtures/progressive.jpg")] {
        let image = decode_image(bytes).unwrap();
        assert_eq!((image.width, image.height), (8, 8));
        for pixel in image.pixels.chunks_exact(4) {
            for (actual, expected) in pixel[..3].iter().zip([210i16, 40, 70]) {
                assert!((*actual as i16 - expected).abs() <= 3);
            }
            assert_eq!(pixel[3], 255);
        }
    }
    let gray = decode_image(include_bytes!("fixtures/gray.jpg")).unwrap();
    assert!(gray
        .pixels
        .chunks_exact(4)
        .all(|p| p == [123, 123, 123, 255]));
    let cmyk = decode_image(include_bytes!("fixtures/cmyk.jpg")).unwrap();
    assert!(
        cmyk.pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
        "{:?}",
        &cmyk.pixels[..16]
    );
}

#[test]
fn malformed_and_oversized_images_fail() {
    for bytes in [RGBA, JPEG] {
        for len in 0..bytes.len() / 2 {
            assert!(
                decode_image(&bytes[..len]).is_err(),
                "accepted prefix {len}"
            );
        }
    }
    let mut bad_crc = RGBA.to_vec();
    bad_crc[29] ^= 1;
    assert!(decode_image(&bad_crc).is_err());
    for (width, height) in [(0u32, 1u32), (8193, 1), (8192, 8192)] {
        let mut oversized = RGBA.to_vec();
        oversized[16..20].copy_from_slice(&width.to_be_bytes());
        oversized[20..24].copy_from_slice(&height.to_be_bytes());
        assert_eq!(decode_image(&oversized), Err(ImageError::InvalidDimensions));
    }
    let mut oversized = JPEG.to_vec();
    let sof = oversized
        .windows(2)
        .position(|p| p == [0xff, 0xc0])
        .unwrap();
    oversized[sof + 5..sof + 7].copy_from_slice(&8192u16.to_be_bytes());
    oversized[sof + 7..sof + 9].copy_from_slice(&8192u16.to_be_bytes());
    assert!(decode_image(&oversized).is_err());
}

#[test]
fn animated_png_is_rejected() {
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&sun_img::crc32::crc32_ieee(&out[4..]).to_be_bytes());
        out
    }
    let mut apng = RGBA[..33].to_vec();
    apng.extend(chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
    // One frame, 2x2 canvas, origin 0,0, delay 1/10 second.
    apng.extend(chunk(
        b"fcTL",
        &[
            0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 10, 0, 0,
        ],
    ));
    apng.extend_from_slice(&RGBA[33..]);
    assert_eq!(decode_image(&apng), Err(ImageError::UnsupportedFormat));
}
