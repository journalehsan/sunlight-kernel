pub mod icon_theme;
pub mod mime_icon;
pub mod mono_icon;
pub mod simg;
pub mod tga;

pub use mime_icon::{
    family_fallback_name, generic_fallback_name, is_image_mime, is_text_like_mime,
    mime_to_freedesktop_name, resolve_file_icon, MimeIconLookup, DIRECTORY_MIMETYPE_ICON,
    MAX_MIME_ICON_NAME, UNKNOWN_ICON,
};
pub use mono_icon::{draw_mono_icon, MonoIcon, MonoIconError};
pub use simg::{decode as decode_simg, encode_tga_type2_bgr24, scale_fit, DecodeError, RgbaImage};
pub use tga::TgaImage;
