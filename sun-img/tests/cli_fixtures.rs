use sun_img::{encode_tga_rgba32, ImageRgba8};

pub fn tiny_rgba_image() -> ImageRgba8 {
    ImageRgba8 {
        width: 2,
        height: 2,
        pixels: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
        ],
    }
}

pub fn tiny_tga_bytes() -> Vec<u8> {
    encode_tga_rgba32(&tiny_rgba_image()).expect("encode fixture")
}
