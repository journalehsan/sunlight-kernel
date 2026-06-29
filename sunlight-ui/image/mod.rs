pub mod icon_theme;
pub mod tga;
pub mod simg;

pub use tga::TgaImage;
pub use simg::{decode as decode_simg, encode_tga_type2_bgr24, scale_fit, DecodeError, RgbaImage};
