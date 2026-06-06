// Stub for sunlight-drivers crate
#![no_std]

pub trait Driver {
    fn init(&self);
    fn shutdown(&self);
}

pub struct NullDriver;

impl Driver for NullDriver {
    fn init(&self) {}
    fn shutdown(&self) {}
}
