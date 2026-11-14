#![no_std]
//! Experimental SunFS 0.2 codecs, checked geometry and bootstrap publication.
//! Bounded hosted-qualified mutation engine; no native device adapter or production integration.
extern crate alloc;
pub mod block;
pub mod bootstrap;
pub mod codec;
pub mod engine;
pub mod geometry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Bounds,
    Overflow,
    Alignment,
    Geometry,
    Capacity,
    Reserve,
    Budget,
    Magic,
    Unsupported,
    Reserved,
    Checksum,
    Structure,
    Reference,
    NoMemory,
}
pub type Result<T> = core::result::Result<T, Error>;
