// Stub for sunlight-ipc crate
#![no_std]

pub struct Message;

pub fn send(_msg: Message) {
    // TODO: implement IPC send
}

pub fn recv() -> Message {
    // TODO: implement IPC receive
    Message
}
