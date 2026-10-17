#![no_std]
extern crate alloc;
pub mod editor;

#[cfg(all(test, not(target_os = "none")))]
extern crate std;
#[cfg(all(test, not(target_os = "none")))]
mod sample_tests;
