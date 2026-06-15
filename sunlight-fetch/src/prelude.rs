#[cfg(feature = "host-linux")]
pub use std::string::{String, ToString};
#[cfg(feature = "host-linux")]
pub use std::vec::Vec;

#[cfg(not(feature = "host-linux"))]
pub use alloc::format;
#[cfg(not(feature = "host-linux"))]
pub use alloc::string::{String, ToString};
#[cfg(not(feature = "host-linux"))]
pub use alloc::vec;
#[cfg(not(feature = "host-linux"))]
pub use alloc::vec::Vec;

pub use core::fmt::Write;