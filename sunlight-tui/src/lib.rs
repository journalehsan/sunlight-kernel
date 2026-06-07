//! SunlightOS graphical boot TUI
//! 
//! Pure Rust, no_std, no heap, no floats.
//! Renders directly to Limine framebuffer.

#![no_std]
#![allow(dead_code)]

mod framebuffer;
mod font;
mod draw;
pub mod fmt;
mod layout;
mod modes;
mod splash;

pub use splash::{SplashScreen, BootMode};
pub use modes::debug::LogBuffer;
