//! Host harness for shared open descriptions. Pipe/eventfd backends are inert;
//! their live behavior is covered by the native pipeline gate.
extern crate alloc;
mod pipe {
    pub fn socket_retain(_: u32, _: usize) {}
    pub fn pipe_retain_end(_: u32, _: bool) {}
    pub fn pipe_close_end(_: u32, _: bool) {}
    pub fn socket_close(_: u32, _: usize) {}
    pub fn socket_stream(_: u32, _: usize) -> Option<(u32, u32)> { None }
}
mod eventfd {
    pub fn retain(_: u32) {}
    pub fn release(_: u32) {}
}
#[path = "../../../../kernel/src/process/fd_table.rs"]
mod fd_table;
