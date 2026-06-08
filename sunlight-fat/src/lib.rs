#![no_std]

pub mod share;
mod fat;

pub use fat::Fat32;
pub use share::{FatSharePage, FatShareFile, FAT_SHARE_VADDR, SHARE_MAGIC};
