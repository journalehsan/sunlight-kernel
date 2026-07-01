pub mod icon_theme;
pub mod simg;
pub mod tga;

pub use simg::{decode as decode_simg, encode_tga_type2_bgr24, scale_fit, DecodeError, RgbaImage};
pub use tga::TgaImage;
